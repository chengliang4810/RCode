import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:rcode_mobile/preferences.dart';
import 'package:rcode_mobile/theme.dart';
import 'package:rcode_mobile/ui/session_page.dart';
import 'package:rcode_mobile/ui/workspace_shell.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'support/page_workspace.dart';

void main() {
  testWidgets('phone and tablet layouts render without overflow', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final preferences = await AppPreferences.load();
    await preferences.setValue('language', 'zh');
    await preferences.setValue('renderMath', false);
    final cjk = Platform.environment['RCODE_QA_CJK_FONT'];
    if (cjk != null) {
      final bytes = await tester.runAsync(() => File(cjk).readAsBytes());
      await (FontLoader(
        'Microsoft YaHei',
      )..addFont(Future.value(ByteData.sublistView(bytes!)))).load();
    }
    final workspace = PageWorkspace();
    workspace.spaces = [
      {'id': 'project', 'name': 'RCode', 'path': '/workspace', 'deviceId': 'a'},
    ];
    workspace.chats = [
      {
        'id': 'chat',
        'title': '完善移动端体验',
        'spaceId': 'project',
        'deviceId': 'a',
        'cwd': '/workspace',
        'branch': 'feature/mobile',
        'config': {
          'harness': 'native',
          'model': 'native-model',
          'reasoning': 'medium',
        },
      },
      {
        'id': 'other',
        'title': '检查断线重连',
        'spaceId': 'project',
        'deviceId': 'a',
        'config': {'harness': 'native'},
      },
    ];
    workspace.sidebarPreferences = {
      'pinnedSessionIds': ['chat'],
    };
    for (final size in [const Size(393, 852), const Size(1180, 820)]) {
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1;
      final session = workspace.session('chat')..hydrated = true;
      session.messages = [
        {
          'id': 'user',
          'role': 'user',
          'parts': [
            {'kind': 'text', 'text': '界面保持简洁，断线后任务继续运行。'},
          ],
        },
        {
          'id': 'assistant',
          'role': 'assistant',
          'status': 'complete',
          'durationMs': 4600,
          'parts': [
            {
              'id': 'text',
              'kind': 'text',
              'text': '移动端界面已接入。\n\n- 手机和平板布局\n- 安全连接远程设备\n- 任务在服务端继续运行\n\n```dart\nawait workspace.reconnect();\n```',
            },
            {
              'id': 'tool',
              'kind': 'tool',
              'resolved': true,
              'call': {'kind': 'read', 'path': 'lib/main.dart'},
              'output': 'Validation passed.',
            },
          ],
        },
      ];
      final boundary = GlobalKey();
      await tester.pumpWidget(
        RepaintBoundary(
          key: boundary,
          child: MaterialApp(
            debugShowCheckedModeBanner: false,
            locale: const Locale('zh'),
            supportedLocales: const [Locale('zh'), Locale('en')],
            localizationsDelegates: GlobalMaterialLocalizations.delegates,
            theme: rcodeTheme(Brightness.light),
            home: size.width < 840
                ? SessionPage(
                    workspace: workspace,
                    chatId: 'chat',
                    preferences: preferences,
                  )
                : WorkspaceShell(
                    workspace: workspace,
                    preferences: preferences,
                    initialChatId: 'chat',
                    sessionBuilder: (id) => SessionPage(
                      key: ValueKey(id),
                      workspace: workspace,
                      chatId: id,
                      preferences: preferences,
                    ),
                  ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(tester.takeException(), isNull);
      await tester.runAsync(() async {
        final render =
            boundary.currentContext!.findRenderObject()
                as RenderRepaintBoundary;
        final image = await render.toImage(pixelRatio: 1);
        final data = await image.toByteData(format: ui.ImageByteFormat.png);
        await Directory('build/qa').create(recursive: true);
        await File('build/qa/${size.width < 840 ? 'phone' : 'tablet'}.png')
            .writeAsBytes(data!.buffer.asUint8List());
        image.dispose();
      });
      await tester.pumpWidget(const SizedBox());
    }
    workspace.dispose();
    tester.view.resetPhysicalSize();
    tester.view.resetDevicePixelRatio();
  });
}
