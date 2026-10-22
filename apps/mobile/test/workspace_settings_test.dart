import 'package:rcode_mobile/ui/settings_page.dart';
import 'package:rcode_mobile/ui/workspace_shell.dart';
import 'package:rcode_mobile/ui/session_page.dart';
import 'package:rcode_mobile/ui/attachments.dart';
import 'package:rcode_mobile/ui/new_session_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:rcode_mobile/preferences.dart';
import 'package:rcode_mobile/theme.dart';

import 'support/page_workspace.dart';
import 'support/keyboard.dart';

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

  for (final control in [false, true]) {
    testWidgets(
      '${control ? 'Android Ctrl' : 'iOS Cmd'} tablet commands retain the same conversation draft',
      (tester) async {
        final workspace = PageWorkspace();
        workspace.chats = [
          {'id': 'chat', 'title': 'Keyboard chat', 'archived': false},
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
          'Unsaved keyboard draft',
        );
        final controller = tester
            .widget<TextField>(find.byKey(const Key('composer-text')))
            .controller;
        await pressShortcut(tester, LogicalKeyboardKey.keyB, control: control);
        await tester.pumpAndSettle();
        expect(find.byKey(const Key('sidebar-settings')), findsNothing);
        expect(find.byKey(const Key('sidebar-show')), findsOneWidget);
        expect(
          tester
              .widget<TextField>(find.byKey(const Key('composer-text')))
              .controller,
          same(controller),
        );
        await pressShortcut(tester, LogicalKeyboardKey.keyB, control: control);
        await tester.pumpAndSettle();
        expect(find.byKey(const Key('sidebar-settings')), findsOneWidget);
        await pressShortcut(tester, LogicalKeyboardKey.keyB, control: control);
        await tester.pumpAndSettle();
        await pressShortcut(
          tester,
          LogicalKeyboardKey.keyF,
          control: control,
          shift: true,
        );
        await tester.pumpAndSettle();
        expect(find.byKey(const Key('sidebar-settings')), findsOneWidget);
        final search = tester.widget<TextField>(
          find.byKey(const Key('session-search')),
        );
        expect(search.focusNode!.hasFocus, isTrue);
        tester.testTextInput.enterText('Keyboard');
        await tester.pump();
        expect(search.controller!.text, 'Keyboard');
        expect(find.text('Unsaved keyboard draft'), findsOneWidget);
        expect(
          tester
              .widget<TextField>(find.byKey(const Key('composer-text')))
              .controller,
          same(controller),
        );
        await pressShortcut(tester, LogicalKeyboardKey.keyN, control: control);
        await tester.pumpAndSettle();
        expect(find.byType(NewSessionPage), findsOneWidget);
        expect(
          tester
              .widget<TextField>(find.byKey(const Key('new-message')))
              .focusNode!
              .hasFocus,
          isTrue,
        );
        expect(tester.takeException(), null);
        await tester.pumpWidget(const SizedBox());
        await tester.pump(const Duration(milliseconds: 400));
      },
      variant: TargetPlatformVariant.only(
        control ? TargetPlatform.android : TargetPlatform.iOS,
      ),
    );

    testWidgets(
      '${control ? 'Android Ctrl' : 'iOS Cmd'} phone commands do not stack new-session dialogs',
      (tester) async {
        final workspace = PageWorkspace();
        workspace.chats = [
          {'id': 'chat', 'title': 'Keyboard chat', 'archived': false},
        ];
        workspace.session('chat').hydrated = true;
        final preferences = await AppPreferences.load();
        addTearDown(workspace.dispose);
        addTearDown(preferences.dispose);
        resetRecoveredImagesForTesting(retrieval: Future.value([]));
        addTearDown(resetRecoveredImagesForTesting);
        tester.view.devicePixelRatio = 1;
        tester.view.physicalSize = const Size(390, 844);
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
          'Keep conversation draft',
        );
        await pressShortcut(tester, LogicalKeyboardKey.keyN, control: control);
        await pressShortcut(tester, LogicalKeyboardKey.keyN, control: control);
        await tester.pumpAndSettle();
        expect(
          find.byType(NewSessionPage, skipOffstage: false),
          findsOneWidget,
        );
        await pressShortcut(tester, LogicalKeyboardKey.keyN, control: control);
        await tester.pumpAndSettle();
        expect(
          find.byType(NewSessionPage, skipOffstage: false),
          findsOneWidget,
        );
        await tester.tap(find.byTooltip('Close'));
        await tester.pumpAndSettle();
        expect(find.text('Keep conversation draft'), findsOneWidget);
        await tester.tap(find.byKey(const Key('composer-text')));
        await pressShortcut(
          tester,
          LogicalKeyboardKey.keyF,
          control: control,
          shift: true,
        );
        await tester.pumpAndSettle();
        final search = tester.widget<TextField>(
          find.byKey(const Key('session-search')),
        );
        expect(search.focusNode!.hasFocus, isTrue);
        expect(find.byType(SessionPage), findsNothing);
        expect(find.byKey(const Key('workspace-tabs')), findsOneWidget);
        await tester.tap(find.text('Sessions').last);
        await tester.pumpAndSettle();
        await tester.tap(find.text('Search').last);
        await tester.pumpAndSettle();
        expect(
          tester
              .widget<TextField>(find.byKey(const Key('session-search')))
              .focusNode!
              .hasFocus,
          isTrue,
        );
        expect(tester.takeException(), null);
        await tester.pumpWidget(const SizedBox());
        await tester.pump(const Duration(milliseconds: 400));
      },
      variant: TargetPlatformVariant.only(
        control ? TargetPlatform.android : TargetPlatform.iOS,
      ),
    );
  }

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
