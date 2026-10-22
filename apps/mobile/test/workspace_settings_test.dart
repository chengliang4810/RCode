import 'package:rcode_mobile/ui/settings_page.dart';
import 'package:rcode_mobile/ui/workspace_shell.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:rcode_mobile/data/workspace_controller.dart';
import 'package:rcode_mobile/preferences.dart';

import 'support/page_workspace.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() => SharedPreferences.setMockInitialValues({}));

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
}
