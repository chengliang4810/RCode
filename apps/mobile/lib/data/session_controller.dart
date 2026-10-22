import 'dart:async';
import 'dart:convert';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/foundation.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:uuid/uuid.dart';

import 'rpc_client.dart';
import 'workspace_controller.dart' show WorkspaceController, jsonMap, jsonMaps;

class UploadAttachment {
  const UploadAttachment({
    required this.name,
    required this.bytes,
    this.mimeType = 'application/octet-stream',
  });
  final String name;
  final Uint8List bytes;
  final String mimeType;
}

/// An authoritative transcript plus device-local, id-stable pending echoes.
class SessionController extends ChangeNotifier {
  SessionController(this.workspace, this.chatId) {
    workspace.addListener(_workspaceChanged);
    _start();
  }
  final WorkspaceController workspace;
  final String chatId;
  List<JsonMap> messages = [];
  List<JsonMap> queue = [];
  List<JsonMap> commands = [];
  JsonMap? contextUsage;
  JsonMap? goal;
  Object? error;
  bool hydrated = false;
  bool sending = false;
  double? transferProgress;
  bool disposed = false;
  int _generation = 0;
  final List<StreamSubscription<dynamic>> _subscriptions = [];
  StreamSubscription<dynamic>? _transcript;
  final Map<String, JsonMap> _echoes = {};
  final Map<String, JsonMap> _attempts = {};
  Timer? _cacheDebounce;

  bool get connected => workspace.connected;
  JsonMap? get chat => workspace.chat(chatId);
  JsonMap? get live => workspace.sessionStatus(chatId);
  JsonMap get config => jsonMap(chat?['config']);
  String get title => '${chat?['title'] ?? ''}';
  bool get working =>
      live?['status'] == 'working' ||
      messages.lastOrNull?['status'] == 'streaming';
  bool get canInterrupt => working || openInput != null;
  List<JsonMap> get pendingMessages => _echoes.values.toList();
  List<JsonMap> get entries => [...messages, ..._echoes.values];

  JsonMap? get openInput {
    for (final entry in messages) {
      for (final part in jsonMaps(entry['parts'])) {
        if (part['kind'] == 'input' && part['resolved'] != true) {
          return {...part, 'entryId': entry['id']};
        }
      }
    }
    return null;
  }

  void _start() {
    final generation = ++_generation;
    if (workspace.cacheEnabled) unawaited(_loadCache(generation));
    _watchTranscript(generation);
    _subscriptions.add(
      workspace.rpc.watch('WatchQueue', {'chatId': chatId}).listen((value) {
        if (disposed || generation != _generation) return;
        queue = jsonMaps(jsonMap(value)['items']);
        notifyListeners();
      }, onError: _streamError),
    );
    _subscriptions.add(
      workspace.rpc.watch('WatchSessionState', {'chatId': chatId}).listen((
        value,
      ) {
        if (disposed || generation != _generation) return;
        final state = jsonMap(value);
        commands = jsonMaps(state['commands']);
        contextUsage = state['contextUsage'] == null
            ? null
            : jsonMap(state['contextUsage']);
        goal = state['goal'] == null ? null : jsonMap(state['goal']);
        for (final command in commands) {
          if (command['issuedBy'] != workspace.clientDeviceId) continue;
          final payload = jsonMap(command['payload']);
          final messageId = payload['messageId'];
          if (_echoes[messageId] case final echo?) {
            echo['deliveryStatus'] = command['status'];
            echo['deliveryError'] = command['resolution'];
          }
        }
        _adoptEchoes();
        notifyListeners();
      }, onError: _streamError),
    );
  }

  void _watchTranscript(int generation) {
    _transcript = workspace.rpc
        .watch('WatchDocMessages', {'chatId': chatId})
        .listen((value) {
          if (disposed || generation != _generation) return;
          try {
            final update = jsonMap(value);
            applyTranscriptUpdate(update);
            if (update.containsKey('contextUsage')) {
              contextUsage = update['contextUsage'] == null
                  ? null
                  : jsonMap(update['contextUsage']);
            }
            if (update.containsKey('goal')) {
              goal = update['goal'] == null ? null : jsonMap(update['goal']);
            }
            hydrated = true;
            error = null;
            _adoptEchoes();
            notifyListeners();
            _saveCache();
          } on FormatException {
            // Keep the rendered state until a fresh authoritative reset arrives.
            unawaited(_transcript?.cancel());
            if (!disposed && generation == _generation) {
              _watchTranscript(generation);
            }
          }
        }, onError: _streamError);
  }

  /// Applies the server's UTF-8 byte-checked reset/upsert/append protocol.
  void applyTranscriptUpdate(JsonMap update) {
    if (update.containsKey('reset')) {
      messages = jsonMaps(update['reset']);
      return;
    }
    final next = messages.toList();
    for (final removed in (update['remove'] as List? ?? const [])) {
      next.removeWhere((entry) => entry['id'] == removed);
    }
    for (final item in jsonMaps(update['upsert'])) {
      final entry = jsonMap(item['entry']);
      next.removeWhere((old) => old['id'] == entry['id']);
      final after = item['after'];
      final anchor = after == null
          ? -1
          : next.indexWhere((old) => old['id'] == after);
      if (after != null && anchor < 0) {
        throw const FormatException('missing transcript anchor');
      }
      next.insert(anchor + 1, entry);
    }
    for (final append in jsonMaps(update['append'])) {
      final index = next.indexWhere((entry) => entry['id'] == append['entry']);
      if (index < 0) throw const FormatException('missing transcript entry');
      final entry = {...next[index]};
      final parts = jsonMaps(entry['parts']);
      final partIndex = parts.indexWhere(
        (part) => part['id'] == append['part'],
      );
      if (partIndex < 0) throw const FormatException('missing transcript part');
      final part = {...parts[partIndex]};
      final text = '${part['text'] ?? ''}${append['text'] ?? ''}';
      if (utf8.encode(text).length != append['len']) {
        throw const FormatException('transcript length mismatch');
      }
      part['text'] = text;
      parts[partIndex] = part;
      entry['parts'] = parts;
      next[index] = entry;
    }
    if (update['count'] is! int || next.length != update['count']) {
      throw const FormatException('transcript count mismatch');
    }
    messages = next;
  }

  void _adoptEchoes() {
    final adopted = messages.map((entry) => entry['id']).toSet();
    _echoes.removeWhere((id, _) => adopted.contains(id));
    _attempts.removeWhere(
      (_, entry) => adopted.contains(jsonMap(entry['payload'])['messageId']),
    );
  }

  void _streamError(Object failure) {
    if (disposed) return;
    error = failure;
    notifyListeners();
  }

  void _workspaceChanged() {
    if (!disposed) notifyListeners();
  }

  Future<String> send(
    String text, {
    List<UploadAttachment> attachments = const [],
    bool steer = false,
    bool queue = false,
    JsonMap? worktree,
  }) async {
    if (text.trim().isEmpty && attachments.isEmpty) {
      throw const RpcException('empty_message', 'empty_message');
    }
    if (!connected) {
      throw const RpcException('disconnected', 'server_disconnected');
    }
    if (sending) throw const RpcException('busy', 'send_pending');
    sending = true;
    error = null;
    notifyListeners();
    final generation = _generation;
    try {
      final currentChat = await workspace.waitForChat(chatId);
      _ensureCurrent(generation);
      final paths = <String>[];
      for (final attachment in attachments) {
        paths.add(await upload(attachment));
        _ensureCurrent(generation);
      }
      final messageId = const Uuid().v4();
      final body = text.trim().isEmpty ? 'See the attached file(s).' : text;
      if (queue || (working && !steer)) {
        await call('QueueMessage', {
          'id': messageId,
          'issuedBy': workspace.clientDeviceId,
          'text': body,
          'attachments': paths,
          'holdForTurnEnd': true,
        });
        return messageId;
      }
      final currentConfig = jsonMap(currentChat['config']);
      final payload = steer
          ? <String, dynamic>{
              'kind': 'steer',
              'prompt': withAttachments(body, paths),
              'messageId': messageId,
            }
          : <String, dynamic>{
              'kind': 'run',
              'messageId': messageId,
              'request': {
                'prompt': withAttachments(body, paths),
                'harness': currentConfig['harness'] ?? 'native',
                'model': currentConfig['model'],
                'reasoning': currentConfig['reasoning'],
                'modelOptions': currentConfig['modelOptions'] ?? {},
                'cwd': currentChat['cwd'] ?? '~',
                'attachments': paths,
                'worktree': worktree,
                'resume': null,
              },
            };
      final entry = _command(payload);
      _echoes[messageId] = {
        'id': messageId,
        'role': 'user',
        'parts': [
          {'id': 'pending-text', 'kind': 'text', 'text': body},
          for (var i = 0; i < paths.length; i++)
            {
              'id': 'pending-image-$i',
              'kind': 'image',
              'path': paths[i],
              'name': attachments[i].name,
              'mimeType': attachments[i].mimeType,
            },
        ],
        'createdAt': DateTime.now().millisecondsSinceEpoch,
        'deviceId': workspace.clientDeviceId,
        'deliveryStatus': 'sending',
      };
      _attempts[messageId] = entry;
      notifyListeners();
      try {
        await call('RelayCommand', {'entry': entry});
        if (_echoes[messageId] case final echo?) {
          echo['deliveryStatus'] = 'pending';
        }
      } catch (failure) {
        if (_echoes[messageId] case final echo?) {
          echo['deliveryStatus'] = 'unconfirmed';
        }
        rethrow;
      }
      return messageId;
    } catch (failure) {
      if (!disposed && generation == _generation) error = failure;
      rethrow;
    } finally {
      if (!disposed && generation == _generation) {
        sending = false;
        transferProgress = null;
        notifyListeners();
      }
    }
  }

  JsonMap _command(JsonMap payload) => {
    'id': const Uuid().v4(),
    'payload': payload,
    'issuedBy': workspace.clientDeviceId,
    'issuedAt': DateTime.now().millisecondsSinceEpoch,
    'basedOn': null,
    'expiresAt': null,
    'status': 'pending',
    'resolution': null,
  };

  Future<void> retryPending(String messageId) async {
    final entry = _attempts[messageId];
    if (entry == null) return;
    // Only an explicit user retry resends, retaining the id already deduped by the host.
    error = null;
    _echoes[messageId]?['deliveryStatus'] = 'sending';
    notifyListeners();
    try {
      await call('RelayCommand', {'entry': entry});
      _echoes[messageId]?['deliveryStatus'] = 'pending';
    } catch (failure) {
      error = failure;
      _echoes[messageId]?['deliveryStatus'] = 'unconfirmed';
      rethrow;
    } finally {
      if (!disposed) notifyListeners();
    }
  }

  Future<void> interrupt() async {
    await call('RelayCommand', {
      'entry': _command({'kind': 'interrupt'}),
    });
  }

  Future<void> respond(
    String requestId,
    List<JsonMap> answers, {
    bool dismissed = false,
  }) async {
    await call('RelayCommand', {
      'entry': _command({
        'kind': 'respondInput',
        'requestId': requestId,
        'answer': dismissed
            ? {'kind': 'dismissed'}
            : {'kind': 'questions', 'answers': answers},
      }),
    });
  }

  Future<void> setConfig(JsonMap next) => workspace.mutate({
    'op': 'setChatConfig',
    'chatId': chatId,
    'config': next,
  });
  Future<dynamic> call(String method, [JsonMap params = const {}]) {
    if (disposed) throw const RpcException('closed', 'session_closed');
    return workspace.call(method, {'chatId': chatId, ...params});
  }

  Future<dynamic> queueAction(
    String method,
    String id, [
    JsonMap params = const {},
  ]) => call(method, {'id': id, ...params});
  Future<dynamic> beginQueueEdit(String id, String instanceId) =>
      queueAction('BeginQueuedMessageEdit', id, {
        'editorDeviceId': workspace.clientDeviceId,
        'editorInstanceId': instanceId,
      });
  Future<dynamic> renewQueueEdit(String id, String leaseId) =>
      queueAction('RenewQueuedMessageEdit', id, {'leaseId': leaseId});
  Future<dynamic> finishQueueEdit(
    String id,
    String leaseId,
    String action, {
    String? text,
    List<String>? attachments,
    String? expectedTextHash,
  }) => queueAction('FinishQueuedMessageEdit', id, {
    'leaseId': leaseId,
    'action': action,
    'text': ?text,
    'attachments': ?attachments,
    'expectedTextHash': ?expectedTextHash,
  });

  Future<String> upload(UploadAttachment attachment) async {
    if (attachment.bytes.length > 24 * 1024 * 1024) {
      throw const RpcException('attachment_too_large', 'attachment_too_large');
    }
    final id = const Uuid().v4();
    const chunkBytes = 510000;
    final count = math.max(1, (attachment.bytes.length / chunkBytes).ceil());
    for (var seq = 0; seq < count; seq++) {
      final begin = math.min(seq * chunkBytes, attachment.bytes.length);
      final end = math.min(begin + chunkBytes, attachment.bytes.length);
      await call('UploadChunk', {
        'uploadId': id,
        'seq': seq,
        'data': base64Encode(attachment.bytes.sublist(begin, end)),
      });
      if (disposed) throw const RpcException('closed', 'session_closed');
      transferProgress = (seq + 1) / count;
      notifyListeners();
    }
    final result = jsonMap(
      await call('UploadCommit', {'uploadId': id, 'fileName': attachment.name}),
    );
    final path = result['path'];
    if (path is! String) {
      throw const RpcException('bad_response', 'attachment_upload_failed');
    }
    return path;
  }

  Future<Uint8List> readAttachment(
    String path, {
    int maximumBytes = 24 * 1024 * 1024,
  }) async {
    final bytes = BytesBuilder(copy: false);
    var offset = 0;
    while (true) {
      final chunk = jsonMap(
        await call('ReadAttachmentChunk', {'path': path, 'offset': offset}),
      );
      final data = base64Decode('${chunk['data'] ?? ''}');
      if (bytes.length + data.length > maximumBytes) {
        throw const RpcException(
          'attachment_too_large',
          'attachment_too_large',
        );
      }
      bytes.add(data);
      if (chunk['done'] == true) break;
      final next = chunk['nextOffset'];
      if (next is! int || next <= offset) {
        throw const RpcException('bad_response', 'attachment_read_failed');
      }
      offset = next;
    }
    return bytes.takeBytes();
  }

  Future<String> toolBlob(String blobRef) async {
    // Copied subagent/tool parts keep their original source namespace.
    final source = blobRef.split('/').first.split('--sub--').first;
    final target = workspace.chat(source)?['deviceId'] ?? chat?['deviceId'];
    final result = await workspace.call('FetchToolBlob', {
      'blobRef': blobRef,
      'chatId': source.isEmpty ? chatId : source,
      'targetDeviceId': ?target,
    });
    return result is String ? result : '${jsonMap(result)['text'] ?? ''}';
  }

  void _ensureCurrent(int generation) {
    if (disposed || generation != _generation) {
      throw const RpcException('closed', 'session_closed');
    }
  }

  Future<void> _loadCache(int generation) async {
    try {
      final preferences = await SharedPreferences.getInstance();
      if (disposed || generation != _generation || hydrated) return;
      final raw = preferences.getString(
        'session.${workspace.cacheScope}.$chatId',
      );
      if (raw == null) return;
      messages = jsonMaps(jsonDecode(raw));
      hydrated = true;
      notifyListeners();
    } catch (_) {
      /* The network reset remains authoritative. */
    }
  }

  void _saveCache() {
    if (!workspace.cacheEnabled) return;
    _cacheDebounce?.cancel();
    final scope = workspace.cacheScope;
    // Bound local caches without changing the authoritative scrollback stream.
    final encoded = jsonEncode(
      messages.skip(math.max(0, messages.length - 200)).toList(),
    );
    _cacheDebounce = Timer(const Duration(milliseconds: 500), () async {
      try {
        final preferences = await SharedPreferences.getInstance();
        if (!disposed) {
          await preferences.setString('session.$scope.$chatId', encoded);
        }
      } catch (_) {
        /* A failed optional cache does not stop a live session. */
      }
    });
  }

  @override
  void dispose() {
    if (disposed) return;
    disposed = true;
    ++_generation;
    workspace.removeListener(_workspaceChanged);
    _cacheDebounce?.cancel();
    unawaited(_transcript?.cancel());
    for (final subscription in _subscriptions) {
      unawaited(subscription.cancel());
    }
    _subscriptions.clear();
    super.dispose();
  }
}

String withAttachments(String text, List<String> paths) => paths.isEmpty
    ? text
    : '$text\n\nAttached images (local files — open them to view):\n${paths.map((path) => '- $path').join('\n')}';
