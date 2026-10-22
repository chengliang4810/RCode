import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rcode_mobile/data/workspace_controller.dart';
import 'package:rcode_mobile/l10n.dart';
import 'package:rcode_mobile/main.dart';
import 'package:rcode_mobile/preferences.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'support/page_workspace.dart';

void main() {
  testWidgets(
    'the production app switches Chinese and English without restarting',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await AppPreferences.load();
      await prefs.setValue('language', 'zh');
      final workspace = PageWorkspace();
      await tester.pumpWidget(
        RCodeApp(
          workspace: workspace,
          preferences: prefs,
          enableNotifications: false,
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('连接'), findsOneWidget);
      expect(find.text('Connect'), findsNothing);
      expect(
        Localizations.localeOf(tester.element(find.byType(Form))),
        const Locale('zh'),
      );
      await prefs.setValue('language', 'en');
      await tester.pumpAndSettle();
      expect(find.text('Connect'), findsOneWidget);
      expect(find.text('连接'), findsNothing);
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
    },
  );

  testWidgets(
    'known failures and effort labels follow locale while user content is retained',
    (tester) async {
      late BuildContext page;
      await tester.pumpWidget(
        MaterialApp(
          locale: const Locale('zh'),
          supportedLocales: const [Locale('zh'), Locale('en')],
          localizationsDelegates: GlobalMaterialLocalizations.delegates,
          home: Builder(
            builder: (context) {
              page = context;
              return const SizedBox();
            },
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(page.effortLabel('xhigh'), '超高');
      expect(
        page.errorText(
          const RpcException('connection_failed', 'connection_failed'),
        ),
        '无法连接服务器',
      );
      expect(
        page.errorText(const FormatException('Unsupported image format')),
        '不支持此图片格式',
      );
      expect(
        page.errorText(const FormatException('A custom server detail')),
        'A custom server detail',
      );
      expect(page.statusLabel('running'), '正在运行');
    },
  );
}
