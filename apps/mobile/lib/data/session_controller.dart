import 'dart:async';
import 'dart:convert';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/foundation.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:uuid/uuid.dart';

import 'rpc_client.dart';
import 'session_cache.dart';
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
    _cacheScope = workspace.cacheScope;
    _cache = workspace.sessionCache.open(_cacheScope, chatId);
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
  bool _authoritativeTranscript = false;
  bool _authoritativeQueue = false, _authoritativeState = false;
  bool _localPendingChanged = false;
  int _metadataRevision = 0;
  bool sending = false;
  double? transferProgress;
  bool disposed = false;
  int _generation = 0;
  final List<StreamSubscription<dynamic>> _subscriptions = [];
  StreamSubscription<dynamic>? _transcript;
  final Map<String, JsonMap> _echoes = {};
  final Map<String, JsonMap> _attempts = {};
  final Map<String, JsonMap> _queueAttempts = {};
  final Map<String, Completer<JsonMap>> _commandWaiters = {};
  Timer? _cacheDebounce;
  late final String _cacheScope;
  late final SessionCacheEntry _cache;
  bool _cacheDirty = false;
  String? _legacyCacheKey;
  Future<void> _cacheWrite = Future.value();

  bool get connected => workspace.connected;
  JsonMap? get chat => workspace.chat(chatId);
  JsonMap? get live => workspace.sessionStatus(chatId);
  JsonMap get config => jsonMap(chat?['config']);
  String get title => '${chat?['title'] ?? ''}';
  bool get working =>
      live?['status'] == 'working' ||
      messages.lastOrNull?['status'] == 'streaming';
  bool get canInterrupt => working || openInput != null;
  bool get needsWarmSubscription =>
      sending ||
      working ||
      live?['status'] == 'awaitingInput' ||
      openInput != null ||
      _echoes.isNotEmpty ||
      _commandWaiters.isNotEmpty ||
      commands.any((command) => command['status'] == 'pending');
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
        _authoritativeQueue = true;
        _adoptEchoes();
        notifyListeners();
        _saveCache();
      }, onError: _streamError),
    );
    _subscriptions.add(
      workspace.rpc.watch('WatchSessionState', {'chatId': chatId}).listen((
        value,
      ) {
        if (disposed || generation != _generation) return;
        final state = jsonMap(value);
        _authoritativeState = true;
        ++_metadataRevision;
        commands = jsonMaps(state['commands']);
        for (final command in commands) {
          if (command['status'] != 'pending') {
            final waiter = _commandWaiters.remove(command['id']);
            waiter?.complete(command);
          }
        }
        contextUsage = state['contextUsage'] == null
            ? null
            : jsonMap(state['contextUsage']);
        goal = state['goal'] == null ? null : jsonMap(state['goal']);
        _restoreCommandEchoes();
        _adoptEchoes();
        notifyListeners();
        _saveCache();
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
              ++_metadataRevision;
              contextUsage = update['contextUsage'] == null
                  ? null
                  : jsonMap(update['contextUsage']);
            }
            if (update.containsKey('goal')) {
              ++_metadataRevision;
              goal = update['goal'] == null ? null : jsonMap(update['goal']);
            }
            final firstSnapshot = !_authoritativeTranscript;
            _authoritativeTranscript = true;
            hydrated = true;
            error = null;
            if (firstSnapshot) _restoreCommandEchoes();
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
    final adopted = {
      ...messages.map((entry) => entry['id']),
      ...queue.map((entry) => entry['id']),
    };
    _echoes.removeWhere((id, _) => adopted.contains(id));
    _attempts.removeWhere(
      (_, entry) => adopted.contains(jsonMap(entry['payload'])['messageId']),
    );
    _queueAttempts.removeWhere((id, _) => adopted.contains(id));
  }

  void _restoreCommandEchoes({bool cachedTranscript = false}) {
    // A newly opened view needs the transcript reset before it can decide
    // which accepted commands are still waiting to become user messages.
    if (!_authoritativeTranscript && !cachedTranscript) return;
    final adopted = messages.map((entry) => entry['id']).toSet();
    final attempts = <String, List<JsonMap>>{};
    for (final command in commands) {
      if (command['issuedBy'] != workspace.clientDeviceId) continue;
      final payload = jsonMap(command['payload']);
      if (payload['kind'] != 'run' && payload['kind'] != 'steer') continue;
      final id = payload['messageId'];
      if (id is! String || adopted.contains(id)) continue;
      attempts.putIfAbsent(id, () => []).add(command);
    }
    for (final group in attempts.entries) {
      final active = group.value
          .where((entry) => entry['status'] != 'cancelled')
          .toList();
      if (active.isEmpty) {
        _echoes.remove(group.key);
        _attempts.remove(group.key);
        continue;
      }
      final live = active.where(
        (entry) => entry['status'] == 'pending' || entry['status'] == 'applied',
      );
      final command = live.lastOrNull ?? active.last;
      final payload = jsonMap(command['payload']);
      final prompt = payload['kind'] == 'run'
          ? jsonMap(payload['request'])['prompt']
          : payload['prompt'];
      final echo = _echoes.putIfAbsent(
        group.key,
        () => {
          'id': group.key,
          'role': 'user',
          'parts': [
            {'id': 'pending-text', 'kind': 'text', 'text': prompt ?? ''},
          ],
          'createdAt': group.value.first['issuedAt'],
          'deviceId': workspace.clientDeviceId,
        },
      );
      echo['deliveryStatus'] = switch (command['status']) {
        'expired' || 'superseded' => 'failed',
        final status => status,
      };
      echo['deliveryError'] = command['resolution'];
      _attempts[group.key] = command;
    }
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
        final request = <String, dynamic>{
          'id': messageId,
          'issuedBy': workspace.clientDeviceId,
          'text': body,
          'attachments': paths,
          'holdForTurnEnd': true,
        };
        _queueAttempts[messageId] = request;
        _echoes[messageId] = _pendingEcho(messageId, body, paths, attachments);
        _saveCache();
        notifyListeners();
        try {
          await call('QueueMessage', request);
          _echoes[messageId]?['deliveryStatus'] = 'pending';
        } catch (_) {
          _echoes[messageId]?['deliveryStatus'] = 'unconfirmed';
          // A queue or transcript update can confirm acceptance before a
          // dropped unary acknowledgement. Otherwise keep an explicit retry.
          if (_queueAttempts.containsKey(messageId)) rethrow;
        }
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
      _echoes[messageId] = _pendingEcho(messageId, body, paths, attachments);
      _attempts[messageId] = entry;
      _saveCache();
      notifyListeners();
      try {
        await call('RelayCommand', {'entry': entry});
        if (_echoes[messageId] case final echo?) {
          if (echo['deliveryStatus'] == 'sending') {
            echo['deliveryStatus'] = 'pending';
          }
        }
      } catch (failure) {
        if (_echoes[messageId] case final echo?) {
          echo['deliveryStatus'] = 'unconfirmed';
        }
        _restoreCommandEchoes();
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
        _saveCache();
      }
    }
  }

  JsonMap _pendingEcho(
    String id,
    String text,
    List<String> paths,
    List<UploadAttachment> attachments,
  ) {
    _localPendingChanged = true;
    return {
      'id': id,
      'role': 'user',
      'parts': [
        {'id': 'pending-text', 'kind': 'text', 'text': text},
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
    _localPendingChanged = true;
    final queued = _queueAttempts[messageId];
    if (queued != null) {
      error = null;
      _echoes[messageId]?['deliveryStatus'] = 'sending';
      notifyListeners();
      try {
        await call('QueueMessage', queued);
        _echoes[messageId]?['deliveryStatus'] = 'pending';
      } catch (failure) {
        error = failure;
        _echoes[messageId]?['deliveryStatus'] = 'unconfirmed';
        if (_queueAttempts.containsKey(messageId)) rethrow;
      } finally {
        if (!disposed) {
          notifyListeners();
          _saveCache();
        }
      }
      return;
    }
    final previous = _attempts[messageId];
    if (previous == null) return;
    final settled = commands
        .where((entry) => entry['id'] == previous['id'])
        .firstOrNull;
    // A lost acknowledgement retries the same command. A known failed attempt
    // needs a new command id; the stable message id still deduplicates adoption.
    final entry =
        settled != null &&
            {'rejected', 'expired', 'superseded'}.contains(settled['status'])
        ? _command(jsonMap(previous['payload']))
        : previous;
    _attempts[messageId] = entry;
    error = null;
    _echoes[messageId]?['deliveryStatus'] = 'sending';
    notifyListeners();
    try {
      await call('RelayCommand', {'entry': entry});
      if (_echoes[messageId]?['deliveryStatus'] == 'sending') {
        _echoes[messageId]?['deliveryStatus'] = 'pending';
      }
    } catch (failure) {
      error = failure;
      _echoes[messageId]?['deliveryStatus'] = 'unconfirmed';
      _restoreCommandEchoes();
      rethrow;
    } finally {
      if (!disposed) {
        notifyListeners();
        _saveCache();
      }
    }
  }

  Future<void> interrupt() async {
    await _submitControl({'kind': 'interrupt'});
  }

  Future<void> respond(
    String requestId,
    List<JsonMap> answers, {
    bool dismissed = false,
  }) async {
    await _submitControl({
      'kind': 'respondInput',
      'requestId': requestId,
      'answer': dismissed
          ? {'kind': 'dismissed'}
          : {'kind': 'questions', 'answers': answers},
    });
  }

  Future<void> _submitControl(JsonMap payload) async {
    final entry = _command(payload);
    await call('RelayCommand', {'entry': entry});
    // Relay acknowledgement means the host evaluated the command. A typed
    // rejection is still an RPC success, so only its authoritative ledger
    // status can dismiss a question or confirm a Stop.
    JsonMap? outcome = commands
        .where(
          (command) =>
              command['id'] == entry['id'] && command['status'] != 'pending',
        )
        .firstOrNull;
    if (outcome == null) {
      final ready = Completer<JsonMap>();
      _commandWaiters[entry['id'] as String] = ready;
      try {
        outcome = await ready.future.timeout(
          const Duration(seconds: 15),
          onTimeout: () {
            throw const RpcException('unconfirmed', 'operation_unconfirmed');
          },
        );
      } finally {
        _commandWaiters.remove(entry['id']);
      }
    }
    if (outcome['status'] != 'applied') {
      throw RpcException(
        '${outcome['status']}',
        '${outcome['resolution'] ?? 'operation_rejected'}',
      );
    }
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
      var saved = await _cache.read();
      if (disposed || generation != _generation) return;
      if (saved == null) {
        final preferences = await SharedPreferences.getInstance();
        if (disposed || generation != _generation) return;
        final key = 'session.$_cacheScope.$chatId';
        final raw = preferences.getString(key);
        if (raw != null) {
          final legacy = jsonDecode(raw);
          if (legacy is List) {
            saved = {'messages': legacy};
            _legacyCacheKey = key;
          }
        }
      }
      if (saved == null || disposed || generation != _generation) return;
      // Each independently delivered server snapshot wins its own field.
      // A delayed disk read cannot replace a fresh transcript, queue or ledger.
      if (!_authoritativeTranscript) {
        messages = jsonMaps(saved['messages']);
        hydrated = true;
      }
      if (!_authoritativeQueue) queue = jsonMaps(saved['queue']);
      if (!_authoritativeState) commands = jsonMaps(saved['commands']);
      if (_metadataRevision == 0) {
        contextUsage = saved['contextUsage'] == null
            ? null
            : jsonMap(saved['contextUsage']);
        goal = saved['goal'] == null ? null : jsonMap(saved['goal']);
      }
      if (!_localPendingChanged) {
        for (final echo in jsonMaps(saved['pending'])) {
          if (echo['deviceId'] == workspace.clientDeviceId &&
              echo['id'] is String) {
            // A process restart ends the original RPC wait. Its result remains
            // unknown until a server snapshot adopts it or the user retries.
            if (echo['deliveryStatus'] == 'sending') {
              echo['deliveryStatus'] = 'unconfirmed';
            }
            _echoes.putIfAbsent(echo['id'] as String, () => echo);
          }
        }
        for (final attempt in jsonMaps(saved['attempts'])) {
          final id = jsonMap(attempt['payload'])['messageId'];
          if (id is String && attempt['issuedBy'] == workspace.clientDeviceId) {
            _attempts.putIfAbsent(id, () => attempt);
          }
        }
        for (final attempt in jsonMaps(saved['queueAttempts'])) {
          final id = attempt['id'];
          if (id is String && attempt['issuedBy'] == workspace.clientDeviceId) {
            _queueAttempts.putIfAbsent(id, () => attempt);
          }
        }
      }
      _restoreCommandEchoes(cachedTranscript: _legacyCacheKey == null);
      _adoptEchoes();
      notifyListeners();
      if (_legacyCacheKey != null) _saveCache();
    } catch (_) {
      /* The network reset remains authoritative. */
    }
  }

  void _saveCache() {
    if (!workspace.cacheEnabled || disposed) return;
    _cacheDirty = true;
    _cacheDebounce?.cancel();
    _cacheDebounce = Timer(const Duration(milliseconds: 500), () {
      unawaited(flushCache().catchError((Object _) {}));
    });
  }

  /// Backgrounding/closing a view can flush without replaying any server work.
  Future<void> flushCache() {
    _cacheDebounce?.cancel();
    if (!workspace.cacheEnabled || disposed || !_cacheDirty) return _cacheWrite;
    _cacheDirty = false;
    // No full-history encoding on the token path. FileSessionCache encodes
    // this state in an isolate, after the debounce, in write invocation order.
    final state = <String, dynamic>{
      'messages': messages,
      'queue': queue,
      'commands': commands,
      'contextUsage': contextUsage,
      'goal': goal,
      'pending': _echoes.values.toList(),
      'attempts': _attempts.values.toList(),
      'queueAttempts': _queueAttempts.values.toList(),
    };
    _cacheWrite = _cache
        .write(state)
        .then((_) async {
          if (_legacyCacheKey case final key?) {
            final preferences = await SharedPreferences.getInstance();
            await preferences.remove(key);
            _legacyCacheKey = null;
          }
        })
        .catchError((Object _) {
          if (!disposed) _cacheDirty = true;
        });
    return _cacheWrite;
  }

  @override
  void dispose() {
    if (disposed) return;
    unawaited(flushCache());
    disposed = true;
    ++_generation;
    workspace.removeListener(_workspaceChanged);
    _cacheDebounce?.cancel();
    for (final waiter in _commandWaiters.values) {
      waiter.completeError(const RpcException('closed', 'session_closed'));
    }
    _commandWaiters.clear();
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
