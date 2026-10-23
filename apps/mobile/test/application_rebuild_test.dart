import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:rcode_mobile/main.dart';
import 'package:rcode_mobile/preferences.dart';
import 'package:rcode_mobile/theme.dart';

import 'support/page_workspace.dart';

class _Workspace extends PageWorkspace {
  @override
  bool get authenticated => true;
  void updateDirectory() => notifyListeners();
}

void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));
  test('theme identity is stable across repeated workspace updates', () {
    expect(
      identical(rcodeTheme(Brightness.light), rcodeTheme(Brightness.light)),
      isTrue,
    );
    expect(
      identical(rcodeTheme(Brightness.dark), rcodeTheme(Brightness.dark)),
      isTrue,
    );
  });
  testWidgets(
    'directory updates retain the application while presentation changes apply',
    (tester) async {
      final workspace = _Workspace();
      final preferences = await AppPreferences.load();
      await tester.pumpWidget(
        RCodeApp(
          workspace: workspace,
          preferences: preferences,
          enableNotifications: false,
        ),
      );
      await tester.pumpAndSettle();
      final original = tester.widget<MaterialApp>(find.byType(MaterialApp));
      workspace.chats = [
        {'id': 'new', 'title': 'Visible directory update', 'deviceId': 'a'},
      ];
      for (var i = 0; i < 30; i++) {
        workspace.updateDirectory();
      }
      await tester.pump();
      expect(
        identical(
          tester.widget<MaterialApp>(find.byType(MaterialApp)),
          original,
        ),
        isTrue,
      );
      expect(find.text('Visible directory update'), findsOneWidget);
      await preferences.setValue('themeMode', ThemeMode.dark);
      await tester.pumpAndSettle();
      expect(
        tester.widget<MaterialApp>(find.byType(MaterialApp)).themeMode,
        ThemeMode.dark,
      );
      await preferences.setValue('language', 'en');
      await tester.pumpAndSettle();
      expect(
        tester.widget<MaterialApp>(find.byType(MaterialApp)).locale,
        const Locale('en'),
      );
      await tester.pumpWidget(const SizedBox.shrink());
      workspace.dispose();
    },
  );
}
