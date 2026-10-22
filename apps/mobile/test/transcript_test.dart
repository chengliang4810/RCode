import 'dart:math';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rcode_mobile/preferences.dart';
import 'package:rcode_mobile/ui/diff_view.dart';
import 'package:rcode_mobile/ui/message_view.dart';
import 'package:rcode_mobile/ui/transcript_markdown.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'support/page_workspace.dart';

void main() {
  testWidgets('copied frozen subagents read the original host blob namespace', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await AppPreferences.load();
    final workspace = PageWorkspace();
    workspace.chats = [
      {'id': 'parent', 'deviceId': 'a'},
      {'id': 'origin', 'deviceId': 'b'},
    ];
    workspace.handler = (method, params) async {
      if (method == 'FetchToolBlob') {
        return jsonEncode([
          {
            'id': 'child-result',
            'role': 'assistant',
            'parts': [
              {'id': 'text', 'kind': 'text', 'text': 'Frozen child result'},
            ],
          },
        ]);
      }
      return <String, dynamic>{};
    };
    await tester.pumpWidget(
      app(
        Scaffold(
          body: ToolPartView(
            session: workspace.session('parent'),
            preferences: prefs,
            part: {
              'id': 'copied',
              'kind': 'tool',
              'resolved': true,
              'call': {'title': 'Read docs'},
              'subagentRef': 'origin--sub--call',
              'subagentStatus': 'done',
            },
          ),
        ),
      ),
    );
    await tester.tap(find.text('Read docs'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Subagent'));
    await tester.pumpAndSettle();
    final request = workspace.calls
        .singleWhere((row) => row.method == 'FetchToolBlob')
        .params;
    expect(request['blobRef'], 'origin/origin--sub--call');
    expect(request['chatId'], 'origin');
    expect(request['targetDeviceId'], 'b');
    expect(find.text('Frozen child result'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
    workspace.dispose();
  });
  test('line diffs reconstruct both files and preserve minimal edits', () {
    final random = Random(2048);
    for (var attempt = 0; attempt < 300; attempt++) {
      final old = List.generate(
        1 + random.nextInt(18),
        (_) => '${random.nextInt(5)}',
      );
      final next = List.generate(
        1 + random.nextInt(18),
        (_) => '${random.nextInt(5)}',
      );
      final diff = lineDiff([old.join('\n'), next.join('\n')]);
      expect(
        diff.where((line) => line['kind'] != '+').map((line) => line['text']),
        old,
      );
      expect(
        diff.where((line) => line['kind'] != '-').map((line) => line['text']),
        next,
      );
      final lcs = List.generate(
        old.length + 1,
        (_) => List.filled(next.length + 1, 0),
      );
      for (var i = old.length - 1; i >= 0; i--) {
        for (var j = next.length - 1; j >= 0; j--) {
          lcs[i][j] = old[i] == next[j]
              ? lcs[i + 1][j + 1] + 1
              : max(lcs[i + 1][j], lcs[i][j + 1]);
        }
      }
      expect(
        diff.where((line) => line['kind'] != ' ').length,
        old.length + next.length - 2 * lcs[0][0],
      );
    }
    final diff = lineDiff([
      List.filled(700, 'old').join('\n'),
      List.filled(700, 'new').join('\n'),
    ]);
    expect(diff.where((line) => line['kind'] == '-').length, 700);
    expect(diff.where((line) => line['kind'] == '+').length, 700);
  });

  testWidgets(
    'Markdown renders tables and code copy without exposing raw fences',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await AppPreferences.load();
      await prefs.setValue('renderMath', false);
      await tester.pumpWidget(
        app(
          Scaffold(
            body: SingleChildScrollView(
              child: TranscriptMarkdown(
                text: '# Result\n\n| File | Status |\n| --- | --- |\n| main.dart | Ready |\n\n```dart\nfinal count = 2;\n```',
                preferences: prefs,
              ),
            ),
          ),
        ),
      );
      expect(find.text('Result'), findsOneWidget);
      expect(find.text('Ready'), findsOneWidget);
      expect(find.byType(CodeBlock), findsOneWidget);
      expect(find.byTooltip('Copy code'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets(
    'adjacent tools collapse together and an unconfirmed message can retry',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await AppPreferences.load();
      final workspace = PageWorkspace();
      final session = workspace.session('chat');
      await tester.pumpWidget(
        app(
          Scaffold(
            body: SingleChildScrollView(
              child: Column(
                children: [
                  MessageView(
                    session: session,
                    preferences: prefs,
                    entry: {
                      'id': 'assistant',
                      'role': 'assistant',
                      'parts': [
                        {
                          'id': 'a',
                          'kind': 'tool',
                          'call': {'kind': 'read', 'path': 'main.dart'},
                          'resolved': true,
                        },
                        {
                          'id': 'b',
                          'kind': 'tool',
                          'call': {'kind': 'read', 'path': 'theme.dart'},
                          'resolved': true,
                        },
                      ],
                    },
                  ),
                  MessageView(
                    session: session,
                    preferences: prefs,
                    entry: {
                      'id': 'user',
                      'role': 'user',
                      'deliveryStatus': 'unconfirmed',
                      'parts': [
                        {'kind': 'text', 'text': 'Run checks'},
                      ],
                    },
                  ),
                ],
              ),
            ),
          ),
        ),
      );
      expect(find.text('2 tool calls'), findsOneWidget);
      expect(find.text('main.dart'), findsNothing);
      expect(find.text('Delivery is unconfirmed'), findsOneWidget);
      expect(find.text('Retry'), findsOneWidget);
      await tester.tap(find.text('2 tool calls'));
      await tester.pumpAndSettle();
      expect(find.text('main.dart'), findsOneWidget);
      expect(find.text('theme.dart'), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
    },
  );
}
