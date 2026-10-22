import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:crypto/crypto.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rcode_mobile/data/workspace_controller.dart';
import 'package:shared_preferences/shared_preferences.dart';

Future<void> until(bool Function() predicate) async {
  final deadline = DateTime.now().add(const Duration(seconds: 5));
  while (!predicate()) {
    if (DateTime.now().isAfter(deadline)) fail('state did not converge');
    await Future<void>.delayed(const Duration(milliseconds: 10));
  }
}

class MemoryStore implements ConnectionStore {
  final Map<String, String> values = {};
  Completer<void>? holdWrite;
  bool blocked = false;
  @override
  Future<String?> read(String key) async => values[key];
  @override
  Future<void> write(String key, String? value) async {
    if (key == 'rcode.mobile.host' && holdWrite != null && !blocked) {
      blocked = true;
      await holdWrite!.future;
    }
    if (value == null) {
      values.remove(key);
    } else {
      values[key] = value;
    }
  }
}

class EngineFixture {
  EngineFixture(this.deviceId);
  final String deviceId;
  late HttpServer http;
  final Set<WebSocket> sockets = {};
  final Set<WebSocket> closing = {};
  final Map<WebSocket, Map<int, JsonMap>> watchers = {};
  List<JsonMap> chats = [];
  List<JsonMap> messages = [];
  List<JsonMap> commands = [];
  List<JsonMap> queue = [];
  List<JsonMap> sessions = [];
  final Map<String, BytesBuilder> uploads = {};
  int relayCalls = 0;
  bool loseReply = false;
  bool slowIdentity = false;
  final identityRead = Completer<void>();
  final identityRelease = Completer<void>();
  JsonMap? lastCommand;
  JsonMap? lastLease;
  String get url => 'ws://127.0.0.1:${http.port}';

  Future<void> start() async {
    http = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    http.listen((request) async {
      if (request.headers.value('Authorization') != 'Bearer FixtureKey12') {
        request.response.statusCode = 401;
        await request.response.close();
        return;
      }
      final socket = await WebSocketTransformer.upgrade(request);
      sockets.add(socket);
      watchers[socket] = {};
      socket.listen(
        (raw) async {
          if (socket.readyState != WebSocket.open || closing.contains(socket)) {
            return;
          }
          final request = jsonMap(jsonDecode(raw as String));
          final id = request['id'] as int;
          if (request['cancel'] == true) {
            watchers[socket]?.remove(id);
            return;
          }
          final method = request['method'] as String;
          final params = jsonMap(request['params']);
          if (method.startsWith('Watch')) {
            watchers[socket]![id] = {'method': method, 'params': params};
            socket.add(jsonEncode({'id': id, 'item': snapshot(method)}));
            return;
          }
          dynamic result = {'ok': true};
          switch (method) {
            case 'EngineInfo':
              if (slowIdentity) {
                if (!identityRead.isCompleted) identityRead.complete();
                await identityRelease.future;
                if (socket.readyState != WebSocket.open) return;
              }
              result = {'deviceId': deviceId};
            case 'Mutate':
              if (params['op'] == 'createChat') {
                chats.add({
                  'id': params['chatId'],
                  'deviceId': deviceId,
                  'cwd': '/fixture',
                  'config': params['config'] ?? {'harness': 'native'},
                  'createdAt': '2026-10-22T00:00:00Z',
                });
                publish('WatchChats');
              } else if (params['op'] == 'setChatConfig') {
                chats.first['config'] = params['config'];
                publish('WatchChats');
              }
            case 'RelayCommand':
              relayCalls++;
              final command = jsonMap(params['entry']);
              lastCommand = command;
              final payload = jsonMap(command['payload']);
              if (!commands.any((entry) => entry['id'] == command['id'])) {
                commands.add({...command, 'status': 'applied'});
                if (payload['kind'] == 'run' || payload['kind'] == 'steer') {
                  final text = payload['kind'] == 'run'
                      ? jsonMap(payload['request'])['prompt']
                      : payload['prompt'];
                  messages.add({
                    'id': payload['messageId'],
                    'role': 'user',
                    'parts': [
                      {'kind': 'text', 'id': 'text', 'text': text},
                    ],
                    'createdAt': 1,
                    'deviceId': command['issuedBy'],
                  });
                }
                publish('WatchSessionState');
                publish('WatchDocMessages');
              }
              if (loseReply) {
                closing.add(socket);
                await socket.close(1011);
                return;
              }
              result = {'status': 'executed'};
            case 'QueueMessage':
              queue.add({...params, 'issuedAt': 1});
              publish('WatchQueue');
              result = {'id': params['id']};
            case 'BeginQueuedMessageEdit':
              lastLease = params;
              result = {
                'outcome': 'acquired',
                'leaseId': 'lease-1',
                'text': 'queued text',
                'attachments': <String>[],
                'baseTextHash': 'base-hash',
                'expiresAtMs': 123456,
              };
            case 'RenewQueuedMessageEdit':
              lastLease = params;
              result = {'outcome': 'renewed', 'expiresAtMs': 234567};
            case 'FinishQueuedMessageEdit':
              lastLease = params;
              result = {'outcome': 'committed'};
            case 'UploadChunk':
              uploads
                  .putIfAbsent(params['uploadId'] as String, BytesBuilder.new)
                  .add(base64Decode(params['data'] as String));
            case 'UploadCommit':
              result = {
                'path': '/uploads/${params['uploadId']}/${params['fileName']}',
              };
            case 'ReadAttachmentChunk':
              final bytes = uploads.values.first.toBytes();
              final offset = params['offset'] as int;
              final end = (offset + 17).clamp(0, bytes.length);
              result = {
                'data': base64Encode(bytes.sublist(offset, end)),
                'nextOffset': end,
                'done': end == bytes.length,
                'name': 'fixture.bin',
                'mimeType': 'application/octet-stream',
              };
            case 'FetchToolBlob':
              result = {'text': 'complete output'};
          }
          socket.add(jsonEncode({'id': id, 'ok': result}));
        },
        onDone: () {
          watchers.remove(socket);
          sockets.remove(socket);
        },
      );
    });
  }

  dynamic snapshot(String method) => switch (method) {
    'WatchDevices' => [
      {
        'id': deviceId,
        'platform': 'linux',
        'name': deviceId,
        'lastSeenAt': '2026-10-22T00:00:00Z',
      },
    ],
    'WatchChats' => chats,
    'WatchSpaces' => <JsonMap>[],
    'WatchSessions' => sessions,
    'WatchSidebarPreferences' => {
      'pinnedSessionIds': <String>[],
      'sections': <JsonMap>[],
    },
    'WatchDocMessages' => {'reset': messages},
    'WatchQueue' => {'items': queue},
    'WatchSessionState' => {
      'version': 1,
      'commands': commands,
      'contextUsage': {'tokens': 12, 'window': 1000},
      'goal': null,
    },
    _ => <String, dynamic>{},
  };

  void publish(String method) {
    for (final connection in watchers.entries.toList()) {
      if (connection.key.readyState != WebSocket.open) continue;
      for (final watch in connection.value.entries.toList()) {
        if (watch.value['method'] == method) {
          connection.key.add(
            jsonEncode({'id': watch.key, 'item': snapshot(method)}),
          );
        }
      }
    }
  }

  Future<void> close() async {
    if (!identityRelease.isCompleted) identityRelease.complete();
    closing.addAll(sockets);
    await Future.wait(sockets.toList().map((socket) => socket.close()));
    await http.close(force: true);
  }
}

WorkspaceController workspace(MemoryStore store, {bool cache = false}) =>
    WorkspaceController(
      store: store,
      cacheEnabled: cache,
      rpc: RpcClient(retryDelay: const Duration(milliseconds: 30)),
    );

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('failed first pairing keeps the connection screen and never stores a bad key', () async {
    final server = EngineFixture('server');
    await server.start();
    final store = MemoryStore();
    final model = workspace(store);
    addTearDown(() async {
      model.dispose();
      await server.close();
    });
    await expectLater(
      model.connect(server.url, 'WrongKey1234'),
      throwsA(isA<RpcException>()),
    );
    expect(model.authenticated, isFalse);
    expect(model.connected, isFalse);
    expect(store.values, isEmpty);
    await model.connect(server.url, 'FixtureKey12');
    await until(() => model.devices.isNotEmpty);
    expect(model.authenticated, isTrue);
    expect(model.deviceId, 'server');
    expect(store.values['rcode.mobile.host'], normalizeServerUrl(server.url));
    expect(
      store.values.values.where((value) => value == 'FixtureKey12').length,
      1,
    );
  });

  test('a delayed old identity and credential write cannot overwrite the current profile', () async {
    final old = EngineFixture('old')..slowIdentity = true;
    await old.start();
    final current = EngineFixture('current');
    await current.start();
    final store = MemoryStore();
    final model = workspace(store);
    addTearDown(() async {
      model.dispose();
      await old.close();
      await current.close();
    });
    final pending = model.connect(old.url, 'FixtureKey12');
    await old.identityRead.future;
    await model.connect(current.url, 'FixtureKey12');
    old.identityRelease.complete();
    await pending;
    await until(() => model.devices.isNotEmpty);
    expect(model.deviceId, 'current');
    expect(model.devices.single['id'], 'current');
    expect(store.values['rcode.mobile.host'], normalizeServerUrl(current.url));
  });

  test(
    'switching during a credential write serializes the final saved profile',
    () async {
      final old = EngineFixture('old');
      await old.start();
      final current = EngineFixture('current');
      await current.start();
      final store = MemoryStore()..holdWrite = Completer<void>();
      final model = workspace(store);
      addTearDown(() async {
        model.dispose();
        await old.close();
        await current.close();
      });
      final first = model.connect(old.url, 'FixtureKey12');
      await until(() => store.blocked);
      final second = model.connect(current.url, 'FixtureKey12');
      await until(() => model.deviceId == 'current');
      store.holdWrite!.complete();
      await Future.wait([first, second]);
      expect(
        store.values['rcode.mobile.host'],
        normalizeServerUrl(current.url),
      );
      final oldScope = sha256.convert(utf8.encode(normalizeServerUrl(old.url)));
      expect(store.values['rcode.mobile.key.$oldScope'], isNull);
    },
  );

  test('offline cache is scoped to a verified host and does not invent online presence', () async {
    SharedPreferences.setMockInitialValues({});
    final old = EngineFixture('cached-host');
    await old.start();
    final store = MemoryStore();
    final first = workspace(store, cache: true);
    await first.connect(old.url, 'FixtureKey12');
    await until(() => first.devices.isNotEmpty);
    final scope = first.cacheScope;
    await Future<void>.delayed(const Duration(milliseconds: 500));
    first.dispose();
    await old.close();
    final restored = workspace(store, cache: true);
    addTearDown(restored.dispose);
    final attempt = restored.initialize();
    await attempt;
    expect(restored.authenticated, isTrue);
    expect(restored.cacheScope, scope);
    expect(restored.devices.single['id'], 'cached-host');
    expect(restored.devices.single['lastSeenAt'], isNull);
    expect(restored.connected, isFalse);
  });

  test('session restores accepted results after a dropped reply without resending a run', () async {
    final server = EngineFixture('server');
    await server.start();
    final model = workspace(MemoryStore());
    addTearDown(() async {
      model.dispose();
      await server.close();
    });
    await model.connect(server.url, 'FixtureKey12');
    await model.call('Mutate', {
      'op': 'createChat',
      'chatId': 'chat',
      'config': {'harness': 'native', 'model': 'fixture-model'},
    });
    final session = model.session('chat');
    await until(() => session.hydrated);
    server.loseReply = true;
    await expectLater(session.send('run once'), throwsA(isA<RpcException>()));
    await until(
      () =>
          session.connected &&
          session.messages.isNotEmpty &&
          session.pendingMessages.isEmpty,
    );
    expect(server.relayCalls, 1);
    expect(
      session.messages.single['id'],
      jsonMap(server.lastCommand!['payload'])['messageId'],
    );
    expect(session.commands.single['status'], 'applied');
    expect(
      jsonMap(jsonMap(server.lastCommand!['payload'])['request'])['model'],
      'fixture-model',
    );
    expect(session.contextUsage, {'tokens': 12, 'window': 1000});
  });

  test('reopened views recover only their own undelivered commands and explicitly retry failures', () async {
    final server = EngineFixture('server');
    await server.start();
    final model = workspace(MemoryStore());
    addTearDown(() async {
      model.dispose();
      await server.close();
    });
    await model.connect(server.url, 'FixtureKey12');
    await model.call('Mutate', {'op': 'createChat', 'chatId': 'chat'});
    JsonMap command(
      String id,
      String status, {
      String? device,
      bool steer = false,
    }) => {
      'id': 'command-$id',
      'payload': {
        'kind': steer ? 'steer' : 'run',
        'messageId': id,
        if (steer)
          'prompt': 'steer text'
        else
          'request': {
            'prompt': 'retry text\n\nAttached images (local files — open them to view):\n- /uploads/recovered.png',
          },
      },
      'issuedBy': device ?? model.clientDeviceId,
      'issuedAt': 100,
      'status': status,
      'resolution': status == 'rejected' ? 'fixture_rejected' : null,
    };
    server.commands = [
      command('failed', 'rejected'),
      command('steer', 'pending', steer: true),
      command('cancelled', 'cancelled'),
      command('other', 'pending', device: 'another-phone'),
    ];
    final first = model.retainSession('chat');
    await until(() => first.hydrated && first.pendingMessages.length == 2);
    expect(first.pendingMessages.map((entry) => entry['id']), [
      'failed',
      'steer',
    ]);
    expect(first.pendingMessages.first['deliveryStatus'], 'rejected');
    expect(first.pendingMessages.first['deliveryError'], 'fixture_rejected');
    expect(
      jsonMaps(first.pendingMessages.first['parts']).single['text'],
      contains('/uploads/recovered.png'),
    );
    expect(server.relayCalls, 0);
    model.releaseSession(first);
    final reopened = model.retainSession('chat');
    await until(
      () => reopened.hydrated && reopened.pendingMessages.length == 2,
    );
    expect(server.relayCalls, 0);
    await reopened.retryPending('failed');
    await until(
      () =>
          reopened.messages.length == 1 && reopened.pendingMessages.length == 1,
    );
    expect(server.relayCalls, 1);
    expect(server.lastCommand!['id'], isNot('command-failed'));
    expect(jsonMap(server.lastCommand!['payload'])['messageId'], 'failed');
    server.commands = [command('steer', 'cancelled', steer: true)];
    server.publish('WatchSessionState');
    await until(() => reopened.pendingMessages.isEmpty);
    model.releaseSession(reopened);
  });

  test('question responses, interrupt, queue identities and edit leases use the real wire shapes', () async {
    final server = EngineFixture('server');
    await server.start();
    final model = workspace(MemoryStore());
    addTearDown(() async {
      model.dispose();
      await server.close();
    });
    await model.connect(server.url, 'FixtureKey12');
    await model.call('Mutate', {'op': 'createChat', 'chatId': 'chat'});
    final session = model.session('chat');
    await until(() => session.hydrated);
    server.messages = [
      {
        'id': 'assistant',
        'role': 'assistant',
        'parts': [
          {
            'kind': 'input',
            'id': 'request',
            'requestId': 'request',
            'request': {'kind': 'questions', 'questions': []},
            'resolved': false,
          },
        ],
      },
    ];
    server.publish('WatchDocMessages');
    await until(() => session.openInput != null);
    await session.respond('request', [
      {
        'questionId': 'q',
        'selectedOptionIds': ['yes'],
        'text': null,
      },
    ]);
    expect(jsonMap(server.lastCommand!['payload'])['answer'], {
      'kind': 'questions',
      'answers': [
        {
          'questionId': 'q',
          'selectedOptionIds': ['yes'],
          'text': null,
        },
      ],
    });
    await session.respond('request', [], dismissed: true);
    expect(jsonMap(server.lastCommand!['payload'])['answer'], {
      'kind': 'dismissed',
    });
    await session.interrupt();
    expect(jsonMap(server.lastCommand!['payload'])['kind'], 'interrupt');
    final queued = await session.send('queued text', queue: true);
    await until(() => session.queue.isNotEmpty);
    expect(session.queue.single['id'], queued);
    expect(session.queue.single['issuedBy'], model.clientDeviceId);
    final lease = jsonMap(await session.beginQueueEdit(queued, 'instance'));
    expect(lease['outcome'], 'acquired');
    await session.finishQueueEdit(
      queued,
      lease['leaseId'] as String,
      'commit',
      text: 'edited',
      expectedTextHash: lease['baseTextHash'] as String,
      attachments: [],
    );
    expect(server.lastLease!['expectedTextHash'], 'base-hash');
    expect(server.lastLease!['action'], 'commit');
  });

  test('attachment bytes round-trip in bounded chunks and source tool output loads', () async {
    final server = EngineFixture('server');
    await server.start();
    final model = workspace(MemoryStore());
    addTearDown(() async {
      model.dispose();
      await server.close();
    });
    await model.connect(server.url, 'FixtureKey12');
    await model.call('Mutate', {'op': 'createChat', 'chatId': 'chat'});
    final session = model.session('chat');
    final bytes = Uint8List.fromList(
      List.generate(510043, (index) => index % 256),
    );
    final path = await session.upload(
      UploadAttachment(name: 'fixture.bin', bytes: bytes),
    );
    expect(server.uploads.values.single.length, bytes.length);
    // A shorter read fixture exercises chunk offsets without thousands of requests.
    server.uploads.clear();
    server.uploads['read'] = BytesBuilder()..add(bytes.sublist(0, 71));
    expect(await session.readAttachment(path), bytes.sublist(0, 71));
    expect(await session.toolBlob('chat/tool-id'), 'complete output');
  });

  test('transcript delta applies UTF-8 lengths atomically and rejects an invalid anchor', () async {
    final model = workspace(MemoryStore());
    final session = model.session('chat');
    addTearDown(model.dispose);
    session.applyTranscriptUpdate({
      'reset': [
        {
          'id': 'entry',
          'role': 'assistant',
          'parts': [
            {'kind': 'text', 'id': 'text', 'text': '你'},
          ],
        },
      ],
    });
    session.applyTranscriptUpdate({
      'append': [
        {'entry': 'entry', 'part': 'text', 'text': '好🙂', 'len': 10},
      ],
      'count': 1,
    });
    expect(jsonMaps(session.messages.single['parts']).single['text'], '你好🙂');
    expect(
      () => session.applyTranscriptUpdate({
        'append': [
          {'entry': 'entry', 'part': 'text', 'text': '!', 'len': 1},
        ],
        'count': 1,
      }),
      throwsFormatException,
    );
    expect(jsonMaps(session.messages.single['parts']).single['text'], '你好🙂');
    expect(
      () => session.applyTranscriptUpdate({
        'upsert': [
          {
            'after': 'missing',
            'entry': {'id': 'other'},
          },
        ],
        'count': 2,
      }),
      throwsFormatException,
    );
    expect(session.messages.length, 1);
  });

  test('the last view releases its reads and an obsolete view cannot release a replacement', () async {
    final server = EngineFixture('server');
    await server.start();
    final model = workspace(MemoryStore());
    addTearDown(() async {
      model.dispose();
      await server.close();
    });
    await model.connect(server.url, 'FixtureKey12');
    final first = model.retainSession('chat');
    final second = model.retainSession('chat');
    expect(identical(first, second), isTrue);
    int reads() => server.watchers.values
        .expand((watches) => watches.values)
        .where((watch) => jsonMap(watch['params'])['chatId'] == 'chat')
        .length;
    await until(() => reads() == 3);
    model.releaseSession(first);
    expect(first.disposed, isFalse);
    expect(reads(), 3);
    model.releaseSession(second);
    await until(() => reads() == 0);
    expect(first.disposed, isTrue);
    final replacement = model.retainSession('chat');
    await until(() => reads() == 3);
    model.releaseSession(first);
    expect(replacement.disposed, isFalse);
    expect(identical(model.session('chat'), replacement), isTrue);
    model.releaseSession(replacement);
    await until(() => reads() == 0);
  });
}
