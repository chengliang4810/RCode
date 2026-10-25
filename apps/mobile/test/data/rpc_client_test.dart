import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:rcode_mobile/data/rpc_client.dart';

Future<void> eventually(bool Function() predicate) async {
  final until = DateTime.now().add(const Duration(seconds: 5));
  while (!predicate()) {
    if (DateTime.now().isAfter(until)) fail('condition did not converge');
    await Future<void>.delayed(const Duration(milliseconds: 10));
  }
}

class RpcFixture {
  late final HttpServer server;
  final Set<WebSocket> sockets = {};
  final Map<WebSocket, Map<int, String>> watches = {};
  int mutations = 0;
  int subscriptions = 0;
  int cancellations = 0;
  int handshakes = 0;
  String identity = 'fixture-host';
  bool dropMutationReply = false;
  bool holdingHandshake = false;
  final Completer<void> handshakeRelease = Completer<void>();

  String get url => 'ws://127.0.0.1:${server.port}';

  Future<void> start() async {
    server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    server.listen((request) async {
      if (request.headers.value('Authorization') != 'Bearer FixtureKey12') {
        request.response.statusCode = HttpStatus.unauthorized;
        await request.response.close();
        return;
      }
      if (holdingHandshake) await handshakeRelease.future;
      final socket = await WebSocketTransformer.upgrade(request);
      handshakes++;
      sockets.add(socket);
      watches[socket] = {};
      socket.listen(
        (raw) {
          final frame = Map<String, dynamic>.from(
            jsonDecode(raw as String) as Map,
          );
          final id = frame['id'] as int;
          if (frame['cancel'] == true) {
            cancellations++;
            watches[socket]?.remove(id);
            return;
          }
          final method = frame['method'] as String;
          if (method.startsWith('Watch')) {
            subscriptions++;
            watches[socket]![id] = method;
            socket.add(
              jsonEncode({
                'id': id,
                'item': {'host': identity, 'revision': subscriptions},
              }),
            );
          } else if (method == 'Mutate') {
            mutations++;
            if (dropMutationReply) {
              unawaited(socket.close(1011, 'test disconnect'));
            } else {
              socket.add(
                jsonEncode({
                  'id': id,
                  'ok': {'accepted': true},
                }),
              );
            }
          } else if (method == 'Fail') {
            socket.add(
              jsonEncode({'id': id, 'err': 'do not display FixtureKey12'}),
            );
          } else if (method == 'Null') {
            socket.add(jsonEncode({'id': id, 'ok': null}));
          } else {
            socket.add(
              jsonEncode({
                'id': id,
                'ok': {'deviceId': identity},
              }),
            );
          }
        },
        onDone: () {
          sockets.remove(socket);
          watches.remove(socket);
        },
      );
    });
  }

  Future<void> close() async {
    if (!handshakeRelease.isCompleted) handshakeRelease.complete();
    await Future.wait(sockets.toList().map((socket) => socket.close()));
    await server.close(force: true);
  }
}

void main() {
  test('normalizes host addresses without embedding credentials', () {
    expect(normalizeServerUrl('127.0.0.1'), 'ws://127.0.0.1:27655/');
    expect(
      normalizeServerUrl('https://example.com/rpc'),
      'wss://example.com/rpc',
    );
    expect(normalizeServerUrl('wss://example.com'), 'wss://example.com/');
    for (final invalid in [
      'ws://key:secret@example.com',
      'ws://host/?key=secret',
      'file:///tmp',
      '',
    ]) {
      expect(() => normalizeServerUrl(invalid), throwsA(isA<RpcException>()));
    }
    expect(validApiKey('Abcd1234'), isTrue);
    expect(validApiKey('12345678'), isFalse);
    expect(validApiKey('密钥abc12345'), isFalse);
  });

  test('authenticates, preserves null replies and redacts errors', () async {
    final fixture = RpcFixture();
    await fixture.start();
    final rpc = RpcClient(retryDelay: const Duration(milliseconds: 30));
    addTearDown(() async {
      await rpc.disconnect();
      rpc.dispose();
      await fixture.close();
    });
    await expectLater(
      rpc.connect(fixture.url, 'WrongKey1234'),
      throwsA(isA<RpcException>()),
    );
    await rpc.connect(fixture.url, 'FixtureKey12');
    expect((await rpc.call('EngineInfo'))['deviceId'], 'fixture-host');
    expect(await rpc.call('Null'), isNull);
    await expectLater(
      rpc.call('Fail'),
      throwsA(
        isA<RpcException>().having(
          (error) => error.message,
          'redacted message',
          allOf(contains('[redacted]'), isNot(contains('FixtureKey12'))),
        ),
      ),
    );
  });

  test(
    'reconnects subscriptions but never replays an unconfirmed mutation',
    () async {
      final fixture = RpcFixture();
      await fixture.start();
      final rpc = RpcClient(retryDelay: const Duration(milliseconds: 20));
      addTearDown(() async {
        await rpc.disconnect();
        rpc.dispose();
        await fixture.close();
      });
      await rpc.connect(fixture.url, 'FixtureKey12');
      final values = <dynamic>[];
      final read = rpc.watch('WatchSomething').listen(values.add);
      await eventually(() => values.isNotEmpty);
      fixture.dropMutationReply = true;
      await expectLater(
        rpc.call('Mutate', {'op': 'run'}),
        throwsA(isA<RpcException>()),
      );
      await eventually(() => values.length >= 2 && rpc.connected);
      expect(fixture.mutations, 1);
      await read.cancel();
      await eventually(() => fixture.cancellations == 1);
      final count = fixture.subscriptions;
      await rpc.reconnect();
      expect(fixture.subscriptions, count);
    },
  );

  test('a successful handshake arriving after its timeout is closed', () async {
    final fixture = RpcFixture()..holdingHandshake = true;
    await fixture.start();
    final rpc = RpcClient(
      callTimeout: const Duration(milliseconds: 30),
      retryDelay: const Duration(days: 1),
    );
    addTearDown(() async {
      await rpc.disconnect();
      rpc.dispose();
      await fixture.close();
    });
    await expectLater(
      rpc.connect(fixture.url, 'FixtureKey12'),
      throwsA(isA<RpcException>()),
    );
    fixture.handshakeRelease.complete();
    await eventually(() => fixture.handshakes == 1);
    await eventually(() => fixture.sockets.isEmpty);
    expect(rpc.connected, isFalse);
  });

  test('an older delayed handshake cannot replace a newer host', () async {
    final old = RpcFixture()..holdingHandshake = true;
    await old.start();
    final current = RpcFixture()..identity = 'current-host';
    await current.start();
    final rpc = RpcClient();
    addTearDown(() async {
      await rpc.disconnect();
      rpc.dispose();
      await old.close();
      await current.close();
    });
    final pending = rpc.connect(old.url, 'FixtureKey12');
    await Future<void>.delayed(const Duration(milliseconds: 30));
    await rpc.connect(current.url, 'FixtureKey12');
    old.handshakeRelease.complete();
    await pending;
    expect((await rpc.call('EngineInfo'))['deviceId'], 'current-host');
  });
}
