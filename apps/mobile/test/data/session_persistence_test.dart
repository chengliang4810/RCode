import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:rcode_mobile/data/workspace_controller.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'workspace_session_test.dart' as fixture;

class _DelayedCache implements SessionCache, SessionCacheEntry {
  final readReady = Completer<JsonMap?>();
  final writes = <JsonMap>[];
  @override
  SessionCacheEntry open(String scope, String chatId) => this;
  @override
  Future<JsonMap?> read() => readReady.future;
  @override
  Future<void> write(JsonMap state) async => writes.add(state);
  @override
  Future<void> clearHost(String scope) async {}
}

WorkspaceController _workspace(fixture.MemoryStore store, SessionCache cache) =>
    WorkspaceController(
      store: store,
      sessionCache: cache,
      rpc: RpcClient(retryDelay: const Duration(milliseconds: 30)),
    );

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('full sessions reopen offline with queue, ledger and metadata, then authoritative snapshots replace them', () async {
    SharedPreferences.setMockInitialValues({});
    final directory = await Directory.systemTemp.createTemp(
      'rcode-full-session-',
    );
    final server = fixture.EngineFixture('cached-server');
    await server.start();
    final cache = FileSessionCache(directoryProvider: () async => directory);
    final store = fixture.MemoryStore();
    final first = _workspace(store, cache);
    WorkspaceController? restored;
    addTearDown(() async {
      first.dispose();
      restored?.dispose();
      await server.close();
      await cache.open('barrier', 'barrier').read();
      expect(
        directory.absolute.path,
        startsWith(Directory.systemTemp.absolute.path),
      );
      await directory.delete(recursive: true);
    });
    await first.connect(server.url, 'FixtureKey12');
    await first.call('Mutate', {'op': 'createChat', 'chatId': 'chat'});
    server.messages = List.generate(
      750,
      (i) => {
        'id': 'message-$i',
        'role': i.isEven ? 'user' : 'assistant',
        'status': 'complete',
        'parts': [
          {'id': 'text', 'kind': 'text', 'text': 'Message $i'},
        ],
      },
    );
    server.queue = [
      {
        'id': 'queue-one',
        'text': 'Waiting',
        'deliveryGate': {'kind': 'editing'},
      },
    ];
    server.commands = [
      {
        'id': 'command-one',
        'status': 'pending',
        'payload': {'kind': 'interrupt'},
      },
    ];
    server.contextUsage = {'tokens': 8000, 'window': 32000};
    server.goal = {'objective': 'Keep the full history', 'status': 'working'};
    final online = first.retainSession('chat');
    await fixture.until(
      () =>
          online.messages.length == 750 &&
          online.queue.length == 1 &&
          online.commands.length == 1,
    );
    await first.flushCaches();
    final scope = first.cacheScope;
    first.dispose();
    final checkpoint = (await cache.open(scope, 'chat').read())!;
    checkpoint['pending'] = [
      {
        'id': 'interrupted-send',
        'deviceId': first.clientDeviceId,
        'deliveryStatus': 'sending',
        'parts': <JsonMap>[],
      },
    ];
    checkpoint['attempts'] = [
      {
        'id': 'stable-command-id',
        'issuedBy': first.clientDeviceId,
        'payload': {
          'kind': 'run',
          'messageId': 'interrupted-send',
          'request': {'prompt': 'Await explicit retry'},
        },
      },
    ];
    await cache.open(scope, 'chat').write(checkpoint);
    server.rejectConnections = true;
    restored = _workspace(store, cache);
    await restored.initialize();
    expect(restored.connected, isFalse);
    final offline = restored.retainSession('chat');
    await fixture.until(
      () => offline.hydrated && offline.messages.length == 750,
    );
    expect(offline.messages.first['id'], 'message-0');
    expect(offline.queue.single['id'], 'queue-one');
    expect(offline.commands.single['id'], 'command-one');
    expect(offline.contextUsage, {'tokens': 8000, 'window': 32000});
    expect(offline.goal?['objective'], 'Keep the full history');
    expect(offline.pendingMessages.single['id'], 'interrupted-send');
    expect(offline.pendingMessages.single['deliveryStatus'], 'unconfirmed');
    expect(server.relayCalls, 0);
    expect(server.queueCalls, 0);
    final preferences = await SharedPreferences.getInstance();
    expect(
      preferences.getKeys().where((key) => key.startsWith('session.$scope.')),
      isEmpty,
    );
    server.messages = [
      {'id': 'fresh', 'role': 'assistant', 'parts': <JsonMap>[]},
    ];
    server.queue = [];
    server.commands = [];
    server.contextUsage = {'tokens': 1, 'window': 100};
    server.goal = null;
    server.rejectConnections = false;
    await fixture.until(
      () =>
          restored!.connected &&
          offline.messages.length == 1 &&
          offline.messages.first['id'] == 'fresh' &&
          offline.commands.isEmpty,
    );
    expect(offline.queue, isEmpty);
    expect(offline.contextUsage, {'tokens': 1, 'window': 100});
    expect(offline.goal, isNull);
    expect(offline.pendingMessages.single['deliveryStatus'], 'unconfirmed');
    expect(
      server.relayCalls,
      0,
      reason: 'restoration and resubscription never replay mutations',
    );
    expect(server.queueCalls, 0);
    await restored.flushCaches();
    final otherScope = 'another-host';
    await cache.open(otherScope, 'chat').write({
      'messages': [
        {'id': 'other-host'},
      ],
    });
    await preferences.setString(
      'session.$scope.legacy-chat',
      jsonEncode([
        {'id': 'legacy'},
      ]),
    );
    await restored.disconnect();
    expect(await cache.open(scope, 'chat').read(), isNull);
    expect(await cache.open(otherScope, 'chat').read(), isNotNull);
    expect(
      preferences.getKeys().where(
        (key) => key == 'workspace.$scope' || key.startsWith('session.$scope.'),
      ),
      isEmpty,
    );
  });

  test(
    'a delayed disk read cannot overwrite live transcript, queue or metadata',
    () async {
      SharedPreferences.setMockInitialValues({});
      final server = fixture.EngineFixture('server');
      await server.start();
      final cache = _DelayedCache();
      final model = _workspace(fixture.MemoryStore(), cache);
      addTearDown(() async {
        model.dispose();
        await server.close();
      });
      await model.connect(server.url, 'FixtureKey12');
      server.messages = [
        {'id': 'live', 'parts': <JsonMap>[]},
      ];
      server.queue = [
        {'id': 'live-queue'},
      ];
      server.commands = [
        {'id': 'live-command', 'status': 'applied'},
      ];
      final session = model.retainSession('chat');
      await fixture.until(
        () =>
            session.hydrated &&
            session.queue.isNotEmpty &&
            session.commands.isNotEmpty,
      );
      cache.readReady.complete({
        'messages': [
          {'id': 'stale'},
        ],
        'queue': [
          {'id': 'stale-queue'},
        ],
        'commands': [
          {'id': 'stale-command'},
        ],
        'contextUsage': {'tokens': 999},
        'goal': {'objective': 'stale'},
      });
      await Future<void>.delayed(const Duration(milliseconds: 30));
      expect(session.messages.single['id'], 'live');
      expect(session.queue.single['id'], 'live-queue');
      expect(session.commands.single['id'], 'live-command');
      expect(session.contextUsage, server.contextUsage);
      expect(session.goal, isNull);
    },
  );

  test('legacy preference history migrates to files after a successful private write', () async {
    SharedPreferences.setMockInitialValues({});
    final directory = await Directory.systemTemp.createTemp(
      'rcode-session-migration-',
    );
    final cache = FileSessionCache(directoryProvider: () async => directory);
    final model = _workspace(fixture.MemoryStore(), cache);
    model.host = 'ws://offline.test:27655/';
    model.clientDeviceId = 'phone';
    final scope = model.cacheScope;
    final preferences = await SharedPreferences.getInstance();
    final key = 'session.$scope.chat';
    await preferences.setString(
      key,
      jsonEncode(List.generate(200, (i) => {'id': '$i'})),
    );
    final session = model.retainSession('chat');
    await fixture.until(() => session.hydrated);
    expect(session.messages, hasLength(200));
    await session.flushCache();
    expect(preferences.containsKey(key), isFalse);
    expect(
      (await cache.open(scope, 'chat').read())!['messages'],
      hasLength(200),
    );
    model.dispose();
    await cache.open('barrier', 'barrier').read();
    expect(
      directory.absolute.path,
      startsWith(Directory.systemTemp.absolute.path),
    );
    await directory.delete(recursive: true);
  });
}
