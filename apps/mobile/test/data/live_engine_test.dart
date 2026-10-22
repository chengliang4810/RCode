import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:rcode_mobile/data/workspace_controller.dart';

class _Store implements ConnectionStore {
  final Map<String, String> values = {};
  @override
  Future<String?> read(String key) async => values[key];
  @override
  Future<void> write(String key, String? value) async {
    if (value == null) {
      values.remove(key);
    } else {
      values[key] = value;
    }
  }
}

Future<void> _until(bool Function() ready) async {
  final deadline = DateTime.now().add(const Duration(seconds: 20));
  while (!ready()) {
    if (DateTime.now().isAfter(deadline)) {
      fail('real engine state did not converge');
    }
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
}

String _text(JsonMap entry) =>
    jsonMaps(entry['parts'])
        .where((part) => part['kind'] == 'text')
        .map((part) => part['text'])
        .join();

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final address = Platform.environment['RCODE_FLUTTER_RPC_URL'];
  final project = Platform.environment['RCODE_FLUTTER_PROJECT'];

  test(
    'Dart controllers operate a real authenticated engine and recover completed offline turns',
    () async {
      final workspace = WorkspaceController(
        cacheEnabled: false,
        store: _Store(),
      );
      addTearDown(workspace.dispose);
      await workspace.connect(address!, 'FixtureKey12');
      await _until(() => workspace.devices.isNotEmpty);
      expect(workspace.deviceId, isNotEmpty);
      final device = workspace.deviceId;
      await workspace.call('Mutate', {
        'op': 'createSpace',
        'spaceId': 'flutter-project',
        'deviceId': device,
        'path': project,
        'name': 'Flutter fixture',
        'gitDetected': false,
      });
      await workspace.call('Mutate', {
        'op': 'createChat',
        'chatId': 'flutter-live-chat',
        'spaceId': 'flutter-project',
        'cwd': project,
        'config': {'harness': 'mock'},
      });
      await workspace.renameChat(
        'flutter-live-chat',
        'Flutter direct acceptance',
      );
      await workspace.changeSidebar({
        'action': 'pin',
        'sessionId': 'flutter-live-chat',
      });
      final session = workspace.retainSession('flutter-live-chat');
      await _until(() => session.hydrated);
      final attachment = Uint8List.fromList(
        List.generate(513, (index) => index % 256),
      );
      final path = await session.upload(
        UploadAttachment(name: 'fixture.bin', bytes: attachment),
      );
      expect(await session.readAttachment(path), attachment);
      await session.send('Flutter live question');
      await _until(() => session.openInput != null);
      final input = session.openInput!;
      final question = jsonMaps(jsonMap(input['request'])['questions']).single;
      await expectLater(
        session.respond('stale-request-id', [
          {
            'questionId': question['id'],
            'selectedOptionIds': ['b'],
            'text': null,
          },
        ]),
        throwsA(isA<RpcException>()),
      );
      await _until(
        () =>
            session.commands.any((command) => command['status'] == 'rejected'),
      );
      final queued = await session.send('Flutter queued once', queue: true);
      await _until(() => session.queue.any((item) => item['id'] == queued));
      final lease = jsonMap(
        await session.beginQueueEdit(queued, 'flutter-live-instance'),
      );
      expect(lease['outcome'], 'acquired');
      final edited = jsonMap(
        await session.finishQueueEdit(
          queued,
          lease['leaseId'] as String,
          'commit',
          text: 'Flutter queued once',
          expectedTextHash: lease['baseTextHash'] as String,
          attachments: [],
        ),
      );
      expect(edited['outcome'], 'committed');
      await session.respond(input['requestId'] as String, [
        {
          'questionId': question['id'],
          'selectedOptionIds': ['b'],
          'text': null,
        },
      ]);
      // The fixture waits before producing its result. There are now no viewers,
      // no subscription, and no Dart connection until after both runs complete.
      await workspace.disconnect(forget: false);
      expect(session.disposed, isTrue);
      await Future<void>.delayed(const Duration(seconds: 2));
      await workspace.connect(address, 'FixtureKey12');
      final restored = workspace.retainSession('flutter-live-chat');
      await _until(
        () =>
            restored.messages.any(
              (entry) => _text(entry) == 'Flutter offline result',
            ) &&
            restored.messages.any(
              (entry) => _text(entry) == 'Flutter queued result',
            ),
      );
      expect(
        restored.messages.where((entry) => entry['role'] == 'user').length,
        2,
      );
      expect(
        restored.messages
            .where((entry) => _text(entry) == 'Flutter offline result')
            .length,
        1,
      );
      expect(
        restored.messages
            .where((entry) => _text(entry) == 'Flutter queued result')
            .length,
        1,
      );
      await _until(
        () => restored.commands.any((entry) => entry['status'] == 'applied'),
      );
      final command = restored.commands.firstWhere(
        (entry) =>
            jsonMap(entry['payload'])['kind'] == 'run' &&
            jsonMap(jsonMap(entry['payload'])['request'])['prompt'] ==
                'Flutter live question',
      );
      final replay = jsonMap(
        await restored.call('RelayCommand', {'entry': command}),
      );
      expect(replay['outcome'], 'duplicate');
      expect(
        workspace.sidebarPreferences['pinnedSessionIds'],
        contains('flutter-live-chat'),
      );
      workspace.releaseSession(restored);
    },
    skip: address == null || project == null
        ? 'run through the ignored Rust flutter_direct integration test'
        : false,
    timeout: const Timeout(Duration(seconds: 60)),
  );
}
