import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:rcode_mobile/data/session_cache.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  late Directory directory;
  setUp(() async {
    directory = await Directory.systemTemp.createTemp('rcode-session-cache-');
  });
  tearDown(() async {
    expect(
      directory.absolute.path,
      startsWith(Directory.systemTemp.absolute.path),
    );
    if (await directory.exists()) await directory.delete(recursive: true);
  });

  test(
    'global disk budget evicts least recently used snapshots across hosts',
    () async {
      var now = DateTime.utc(2026, 1, 1);
      final cache = FileSessionCache(
        directoryProvider: () async => directory,
        maximumBytes: 512,
        maximumEntries: 2,
        now: () => now,
      );
      await cache.open('host-a', 'old').write({'text': 'a' * 100});
      now = now.add(const Duration(minutes: 1));
      await cache.open('host-b', 'other').write({'text': 'b' * 100});
      now = now.add(const Duration(minutes: 1));
      expect(await cache.open('host-a', 'old').read(), isNotNull);
      now = now.add(const Duration(minutes: 1));
      await cache.open('host-a', 'new').write({'text': 'c' * 100});
      expect(await cache.open('host-b', 'other').read(), isNull);
      expect(await cache.open('host-a', 'old').read(), isNotNull);
      expect(await cache.open('host-a', 'new').read(), isNotNull);
      var bytes = 0;
      var files = 0;
      await for (final entry in directory.list(recursive: true)) {
        if (entry is File) {
          bytes += await entry.length();
          files++;
        }
      }
      expect(bytes, lessThanOrEqualTo(512));
      expect(files, lessThanOrEqualTo(2));
      now = now.add(const Duration(days: 31));
      expect(await cache.open('host-a', 'new').read(), isNull);
      expect(
        await directory
            .list(recursive: true)
            .where((entry) => entry is File)
            .toList(),
        isEmpty,
      );
    },
  );

  test('deleting one chat revokes in-flight and old writers but preserves its siblings', () async {
    final began = Completer<void>();
    final release = Completer<void>();
    var encodings = 0;
    final cache = FileSessionCache(
      directoryProvider: () async => directory,
      encoder: (state) async {
        if (++encodings == 2) {
          began.complete();
          await release.future;
        }
        return jsonEncode(state);
      },
    );
    await cache.open('host', 'sibling').write({'text': 'keep'});
    final removed = cache.open('host', 'deleted');
    final writing = removed.write({'text': 'private'});
    await began.future;
    final deleting = cache.remove('host', 'deleted');
    release.complete();
    await Future.wait([writing, deleting]);
    await removed.write({'text': 'must not return'});
    expect(await cache.open('host', 'deleted').read(), isNull);
    expect((await cache.open('host', 'sibling').read())!['text'], 'keep');
    await cache.open('host', 'deleted').write({'text': 'new generation'});
    expect(
      (await cache.open('host', 'deleted').read())!['text'],
      'new generation',
    );
  });

  test('full private snapshots survive restart and separate host and chat identities', () async {
    FileSessionCache cache() =>
        FileSessionCache(directoryProvider: () async => directory);
    final first = cache();
    final history = List.generate(
      720,
      (i) => {
        'id': '$i',
        'parts': [
          {'kind': 'text', 'text': 'Message $i'},
        ],
      },
    );
    final state = <String, dynamic>{
      'messages': history,
      'queue': [
        {'id': 'queued'},
      ],
      'commands': [
        {'id': 'command', 'status': 'applied'},
      ],
      'contextUsage': {'tokens': 4000, 'window': 16000},
      'goal': {'objective': 'Continue'},
    };
    await first.open('host-a', '../same/chat').write(state);
    await first.open('host-b', '../same/chat').write({
      'messages': [
        {'id': 'host-b-only'},
      ],
    });
    final restored = await cache().open('host-a', '../same/chat').read();
    expect(restored, state);
    expect(restored!['messages'], hasLength(720));
    expect(await first.open('host-a', 'other-chat').read(), isNull);
    expect((await first.open('host-b', '../same/chat').read())!['messages'], [
      {'id': 'host-b-only'},
    ]);
    final files = await directory
        .list(recursive: true)
        .where((entry) => entry is File)
        .toList();
    expect(files, hasLength(2));
    for (final file in files) {
      final relative = file.path
          .substring(directory.path.length + 1)
          .replaceAll('\\', '/');
      expect(relative, matches(RegExp(r'^[a-f0-9]{64}/[a-f0-9]{64}\.json$')));
    }
    await first.clearHost('host-a');
    expect(await cache().open('host-a', '../same/chat').read(), isNull);
    expect(await cache().open('host-b', '../same/chat').read(), isNotNull);
  });

  test('a slow encoder cannot overwrite a newer snapshot or recreate a forgotten host', () async {
    final began = Completer<void>();
    final release = Completer<void>();
    int encodings = 0;
    final cache = FileSessionCache(
      directoryProvider: () async => directory,
      encoder: (state) async {
        encodings++;
        if (encodings == 1) {
          began.complete();
          await release.future;
        }
        return jsonEncode(state);
      },
    );
    final entry = cache.open('host', 'chat');
    final first = entry.write({
      'messages': [
        {'id': 'old'},
      ],
    });
    await began.future;
    final latest = entry.write({
      'messages': [
        {'id': 'new'},
      ],
    });
    release.complete();
    await Future.wait([first, latest]);
    expect((await entry.read())!['messages'], [
      {'id': 'new'},
    ]);

    final stale = cache.open('host', 'chat');
    final clearing = cache.clearHost('host');
    final late = stale.write({
      'messages': [
        {'id': 'must-not-return'},
      ],
    });
    await Future.wait([clearing, late]);
    expect(await cache.open('host', 'chat').read(), isNull);
    expect(await stale.read(), isNull);
    await cache.open('host', 'chat').write({
      'messages': [
        {'id': 'new-pairing'},
      ],
    });
    expect((await cache.open('host', 'chat').read())!['messages'], [
      {'id': 'new-pairing'},
    ]);
  });

  test(
    'clearing during encoding revokes the in-flight writer before publication',
    () async {
      final began = Completer<void>();
      final release = Completer<void>();
      final cache = FileSessionCache(
        directoryProvider: () async => directory,
        encoder: (state) async {
          began.complete();
          await release.future;
          return jsonEncode(state);
        },
      );
      final old = cache.open('host', 'chat');
      final writing = old.write({
        'messages': [
          {'id': 'sensitive'},
        ],
      });
      await began.future;
      final clearing = cache.clearHost('host');
      release.complete();
      await Future.wait([writing, clearing]);
      expect(await cache.open('host', 'chat').read(), isNull);
      expect(await directory.list(recursive: true).toList(), isEmpty);
    },
  );
}
