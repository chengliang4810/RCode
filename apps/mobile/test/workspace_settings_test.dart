import 'package:rcode_mobile/ui/settings_page.dart';
import 'package:rcode_mobile/ui/workspace_shell.dart';
import 'package:rcode_mobile/ui/session_page.dart';
import 'package:rcode_mobile/ui/attachments.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:rcode_mobile/preferences.dart';
import 'package:rcode_mobile/theme.dart';

import 'support/page_workspace.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() => SharedPreferences.setMockInitialValues({}));

  testWidgets(
    'language selection rebuilds the actual locale from Chinese to English',
    (tester) async {
      final workspace = PageWorkspace();
      final preferences = await AppPreferences.load();
      addTearDown(workspace.dispose);
      addTearDown(preferences.dispose);
      await preferences.setValue('language', 'zh');
      await tester.pumpWidget(
        ListenableBuilder(
          listenable: preferences,
          builder: (context, _) => MaterialApp(
            theme: rcodeTheme(Brightness.light),
            locale: Locale(preferences.language),
            supportedLocales: const [Locale('zh'), Locale('en')],
            localizationsDelegates: GlobalMaterialLocalizations.delegates,
            home: SettingsPage(
              workspace: workspace,
              preferences: preferences,
              onOpen: (_) {},
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('设置'), findsOneWidget);
      expect(find.text('连接'), findsOneWidget);
      expect(find.text('Linux'), findsOneWidget);
      expect(find.text('linux'), findsNothing);
      await tester.scrollUntilVisible(
        find.byKey(const Key('settings-language')),
        250,
      );
      await tester.ensureVisible(find.byKey(const Key('settings-language')));
      await tester.pumpAndSettle();
      expect(find.text('语言'), findsOneWidget);
      await tester.tap(find.byKey(const Key('settings-language')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('English').last);
      await tester.pumpAndSettle();
      expect(preferences.language, 'en');
      expect(find.text('Settings'), findsOneWidget);
      expect(find.text('Language'), findsOneWidget);
      expect(find.text('设置'), findsNothing);
      expect(tester.takeException(), null);
    },
  );

  testWidgets(
    'settings appearance and grouping persist instead of decorative toggles',
    (tester) async {
      final workspace = PageWorkspace();
      final preferences = await AppPreferences.load();
      addTearDown(workspace.dispose);
      addTearDown(preferences.dispose);
      await tester.pumpWidget(
        app(
          SettingsPage(
            workspace: workspace,
            preferences: preferences,
            onOpen: (_) {},
          ),
        ),
      );
      await tester.scrollUntilVisible(
        find.byKey(const ValueKey('appearance-dark')),
        250,
      );
      await tester.ensureVisible(find.byKey(const ValueKey('appearance-dark')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('appearance-dark')));
      await tester.pumpAndSettle();
      expect(preferences.themeMode, ThemeMode.dark);
      await tester.scrollUntilVisible(
        find.byKey(const ValueKey('settings-groupByProject')),
        300,
      );
      await tester.ensureVisible(
        find.byKey(const ValueKey('settings-groupByProject')),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('settings-groupByProject')));
      await tester.pumpAndSettle();
      expect(preferences.groupByProject, true);
    },
  );

  testWidgets('phone tabs and tablet split share working navigation', (
    tester,
  ) async {
    final workspace = PageWorkspace();
    workspace.chats = [
      {'id': 'chat', 'title': 'Open me', 'archived': false},
    ];
    final preferences = await AppPreferences.load();
    addTearDown(workspace.dispose);
    addTearDown(preferences.dispose);
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(390, 844);
    addTearDown(tester.view.resetDevicePixelRatio);
    addTearDown(tester.view.resetPhysicalSize);
    await tester.pumpWidget(
      app(
        WorkspaceShell(
          workspace: workspace,
          preferences: preferences,
          sessionBuilder: (id) =>
              Scaffold(appBar: AppBar(), body: Text('Session $id')),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('workspace-tabs')), findsOneWidget);
    await tester.tap(find.text('Open me'));
    await tester.pumpAndSettle();
    expect(find.text('Session chat'), findsOneWidget);
    await tester.pageBack();
    await tester.pumpAndSettle();
    tester.view.physicalSize = const Size(1100, 800);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('sidebar-settings')), findsOneWidget);
    expect(find.byKey(const Key('workspace-tabs')), findsNothing);
    await tester.tap(find.text('Open me'));
    await tester.pumpAndSettle();
    expect(find.text('Session chat'), findsOneWidget);
    expect(find.text('Open me'), findsOneWidget);
    expect(tester.takeException(), null);
    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(milliseconds: 300));
  });

  testWidgets(
    'an open tablet conversation keeps its draft across both width changes',
    (tester) async {
      final workspace = PageWorkspace();
      workspace.chats = [
        {'id': 'chat', 'title': 'Rotate me', 'archived': false},
      ];
      workspace.session('chat').hydrated = true;
      final preferences = await AppPreferences.load();
      addTearDown(workspace.dispose);
      addTearDown(preferences.dispose);
      resetRecoveredImagesForTesting(retrieval: Future.value([]));
      addTearDown(resetRecoveredImagesForTesting);
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(1100, 800);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.view.resetPhysicalSize);
      await tester.pumpWidget(
        app(
          WorkspaceShell(
            workspace: workspace,
            preferences: preferences,
            sessionBuilder: (id) => SessionPage(
              workspace: workspace,
              chatId: id,
              preferences: preferences,
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('session-chat')));
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const Key('composer-text')),
        'Draft survives rotation',
      );
      tester.testTextInput.hide();
      FocusManager.instance.primaryFocus?.unfocus();
      await tester.pumpAndSettle();

      tester.view.physicalSize = const Size(600, 900);
      await tester.pumpAndSettle();
      expect(find.byType(SessionPage), findsOneWidget);
      expect(find.byType(BackButton), findsOneWidget);
      expect(find.text('Draft survives rotation'), findsOneWidget);
      expect(find.byKey(const Key('sidebar-settings')), findsNothing);

      tester.view.physicalSize = const Size(1100, 800);
      await tester.pumpAndSettle();
      expect(find.byType(SessionPage), findsOneWidget);
      expect(find.byType(BackButton), findsNothing);
      expect(find.text('Draft survives rotation'), findsOneWidget);
      expect(find.byKey(const Key('sidebar-settings')), findsOneWidget);
      expect(tester.takeException(), null);
      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(milliseconds: 400));
    },
  );

  testWidgets(
    'a compact conversation widens in place and closing it stays closed',
    (tester) async {
      final workspace = PageWorkspace();
      workspace.chats = [
        {'id': 'chat', 'title': 'Open me', 'archived': false},
      ];
      final preferences = await AppPreferences.load();
      addTearDown(workspace.dispose);
      addTearDown(preferences.dispose);
      tester.view.devicePixelRatio = 1;
      tester.view.physicalSize = const Size(390, 844);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.view.resetPhysicalSize);
      await tester.pumpWidget(
        app(
          WorkspaceShell(
            workspace: workspace,
            preferences: preferences,
            sessionBuilder: (id) =>
                Scaffold(appBar: AppBar(), body: Text('Session $id')),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('session-chat')));
      await tester.pumpAndSettle();
      tester.view.physicalSize = const Size(1100, 800);
      await tester.pumpAndSettle();
      expect(find.text('Session chat'), findsOneWidget);
      expect(find.byKey(const Key('sidebar-settings')), findsOneWidget);
      expect(find.byType(BackButton), findsNothing);
      tester.view.physicalSize = const Size(390, 844);
      await tester.pumpAndSettle();
      expect(find.byType(BackButton), findsOneWidget);
      await tester.pageBack();
      await tester.pumpAndSettle();
      tester.view.physicalSize = const Size(1100, 800);
      await tester.pumpAndSettle();
      expect(find.text('Session chat'), findsNothing);
      expect(find.byKey(const Key('sidebar-settings')), findsOneWidget);
      expect(tester.takeException(), null);
      await tester.pumpWidget(const SizedBox());
    },
  );
}
