import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rcode_mobile/preferences.dart';
import 'package:rcode_mobile/data/workspace_controller.dart';
import 'package:rcode_mobile/theme.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'data/workspace_session_test.dart' as fixture;

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  test(
    'forget and repair the same real host never revives either composer draft',
    () async {
      SharedPreferences.setMockInitialValues({});
      final server = fixture.EngineFixture('draft-server');
      await server.start();
      final workspace = WorkspaceController(
        store: fixture.MemoryStore(),
        cacheEnabled: false,
      );
      addTearDown(() async {
        workspace.dispose();
        await server.close();
      });
      final preferences = await AppPreferences.load();
      await workspace.connect(server.url, 'FixtureKey12');
      final scope = workspace.cacheScope;
      final newDraft = preferences.captureHostDraft(scope);
      final sessionDraft = preferences.captureHostDraft(scope, chatId: 'chat');
      await newDraft.save({
        'text': 'Unsent new session',
        'model': 'saved-model',
      });
      await sessionDraft.save({'text': 'Unsent follow-up'});
      await workspace.disconnect();
      await Future.wait([
        newDraft.save({'text': 'Late old new page'}),
        sessionDraft.save({'text': 'Late old session page'}),
      ]);
      await workspace.connect(server.url, 'FixtureKey12');
      expect(preferences.captureHostDraft(scope).value, isNull);
      expect(preferences.captureHostDraft(scope, chatId: 'chat').value, isNull);
      expect(server.relayCalls, 0);
      expect(server.queueCalls, 0);
    },
  );
  test(
    'forgetting one host revokes queued and late page draft writers',
    () async {
      SharedPreferences.setMockInitialValues({});
      final preferences = await AppPreferences.load();
      final newSession = preferences.captureHostDraft('host-a');
      final session = preferences.captureHostDraft('host-a', chatId: 'chat');
      final other = preferences.captureHostDraft('host-b', chatId: 'chat');
      await other.save({'text': 'Other host remains'});
      await preferences.setValue('language', 'zh');
      await preferences.setValue('notifications', false);
      await newSession.save({'text': 'Old new-session text'});
      final queued = session.save({'text': 'Queued before forgetting'});
      final clearing = AppPreferences.forgetHostDrafts('host-a');
      final delayedDispose = newSession.save({
        'text': 'Late dispose must not return',
      });
      await Future.wait([queued, clearing, delayedDispose]);
      final repaired = preferences.captureHostDraft('host-a');
      expect(repaired.value, isNull);
      expect(
        preferences.captureHostDraft('host-a', chatId: 'chat').value,
        isNull,
      );
      expect(other.value?['text'], 'Other host remains');
      expect(preferences.language, 'zh');
      expect(preferences.notifications, isFalse);
      await repaired.save({'text': 'New pairing draft'});
      await session.save({'text': 'Late old page'});
      expect(repaired.value?['text'], 'New pairing draft');
      expect(
        preferences.captureHostDraft('host-a', chatId: 'chat').value,
        isNull,
      );
    },
  );
  test(
    'appearance and drafts survive restart without storing credentials',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await AppPreferences.load();
      await prefs.setValue('themeMode', ThemeMode.dark);
      await prefs.setValue('mathSize', 1.35);
      await prefs.setJson('draft.host', {
        'text': 'unsent',
        'model': 'model-id',
      });
      final restored = await AppPreferences.load();
      expect(restored.themeMode, ThemeMode.dark);
      expect(restored.mathSize, 1.35);
      expect(restored.getJson('draft.host')?['text'], 'unsent');
      expect(
        (await SharedPreferences.getInstance()).getKeys().any(
          (key) => key.contains('apiKey'),
        ),
        false,
      );
    },
  );
  test('theme matches the source iOS palette in both modes', () {
    expect(RCodeColors.light.background, const Color(0xfff3f3f5));
    expect(RCodeColors.dark.background, const Color(0xff060606));
    expect(RCodeColors.light.accent, const Color(0xff5b43e8));
    expect(RCodeColors.dark.accent, const Color(0xff8b7cf6));
    expect(
      rcodeTheme(Brightness.dark).textTheme.bodyMedium?.fontFamily,
      'Geist',
    );
  });
}
