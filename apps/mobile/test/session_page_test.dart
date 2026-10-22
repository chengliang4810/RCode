import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rcode_mobile/preferences.dart';
import 'package:rcode_mobile/data/workspace_controller.dart';
import 'package:rcode_mobile/ui/message_view.dart';
import 'package:rcode_mobile/ui/session_page.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'support/page_workspace.dart';

void main() {
  late PageWorkspace workspace;
  late AppPreferences preferences;
  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    preferences = await AppPreferences.load();
    workspace = PageWorkspace();
    workspace.chats = [
      {
        'id': 'chat',
        'title': 'Refactor',
        'deviceId': 'a',
        'cwd': '/workspace',
        'config': {
          'harness': 'native',
          'model': 'native-model',
          'reasoning': 'medium',
        },
      },
    ];
  });
  tearDown(() => workspace.dispose());

  testWidgets(
    'disposing during a host switch saves the draft in its original host scope',
    (tester) async {
      final original = workspace.cacheScope;
      workspace.session('chat').hydrated = true;
      await tester.pumpWidget(
        app(
          SessionPage(
            workspace: workspace,
            chatId: 'chat',
            preferences: preferences,
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const Key('composer-text')),
        'Private draft',
      );
      workspace.host = 'ws://another.test:27655/';
      final replacement = workspace.cacheScope;
      await tester.pumpWidget(const SizedBox());
      await tester.pump();
      expect(
        preferences.getJson('draft.$original.chat')?['text'],
        'Private draft',
      );
      expect(preferences.getJson('draft.$replacement.chat'), isNull);
    },
  );

  testWidgets(
    'lost acknowledgement leaves one retryable echo and clears its composer draft',
    (tester) async {
      workspace.session('chat').hydrated = true;
      workspace.handler = (method, params) async {
        if (method == 'RelayCommand')
          throw const RpcException('disconnected', 'server_disconnected');
        return <String, dynamic>{};
      };
      await tester.pumpWidget(
        app(
          SessionPage(
            workspace: workspace,
            chatId: 'chat',
            preferences: preferences,
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const Key('composer-text')),
        'Only once',
      );
      await tester.pump();
      await tester.tap(find.byKey(const Key('composer-send')));
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<TextField>(find.byKey(const Key('composer-text')))
            .controller!
            .text,
        '',
      );
      expect(find.text('Only once'), findsOneWidget);
      expect(find.text('Retry'), findsOneWidget);
      final id = workspace.calls
          .singleWhere((row) => row.method == 'RelayCommand')
          .params['entry']['id'];
      await tester.tap(find.text('Retry'));
      await tester.pumpAndSettle();
      expect(
        workspace.calls
            .where((row) => row.method == 'RelayCommand')
            .map((row) => row.params['entry']['id']),
        [id, id],
      );
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'sending clears only its draft and reuses unchanged transcript rows',
    (tester) async {
      final session = workspace.session('chat')..hydrated = true;
      session.messages = [
        {
          'id': 'old',
          'role': 'assistant',
          'parts': [
            {'id': 'text', 'kind': 'text', 'text': 'Previous result'},
          ],
        },
      ];
      await tester.pumpWidget(
        app(
          SessionPage(
            workspace: workspace,
            chatId: 'chat',
            preferences: preferences,
          ),
        ),
      );
      await tester.pumpAndSettle();
      final previous = tester.widget<MessageView>(
        find.byType(MessageView).first,
      );
      await tester.enterText(
        find.byKey(const Key('composer-text')),
        'Run tests',
      );
      await tester.pump();
      expect(
        identical(
          tester.widget<MessageView>(find.byType(MessageView).first),
          previous,
        ),
        isTrue,
      );
      await tester.tap(find.byKey(const Key('composer-send')));
      await tester.pumpAndSettle();
      final relay = workspace.calls.singleWhere(
        (call) => call.method == 'RelayCommand',
      );
      expect(
        relay.params['entry']['payload']['request']['prompt'],
        'Run tests',
      );
      expect(
        tester
            .widget<TextField>(find.byKey(const Key('composer-text')))
            .controller!
            .text,
        '',
      );
      expect(find.text('Run tests'), findsOneWidget);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
      await tester.pump();
      expect(session.disposed, isTrue);
    },
  );

  testWidgets(
    'streaming preserves reading position and jump returns to the tail',
    (tester) async {
      final session = workspace.session('chat')..hydrated = true;
      session.messages = List.generate(
        45,
        (i) => {
          'id': 'message-$i',
          'role': 'assistant',
          'parts': [
            {
              'id': 'text',
              'kind': 'text',
              'text':
                  'Message $i\n\n${List.filled(10, 'Transcript line.').join('\n\n')}',
            },
          ],
        },
      );
      await tester.pumpWidget(
        app(
          SessionPage(
            workspace: workspace,
            chatId: 'chat',
            preferences: preferences,
          ),
        ),
      );
      await tester.pumpAndSettle();
      final list = find.byType(ListView).first;
      final controller = tester.widget<ListView>(list).controller!;
      controller.jumpTo(controller.position.maxScrollExtent - 1200);
      await tester.pumpAndSettle();
      final before = controller.offset;
      session.messages = [
        ...session.messages,
        {
          'id': 'next',
          'role': 'assistant',
          'parts': [
            {'id': 'text', 'kind': 'text', 'text': 'Latest result'},
          ],
        },
      ];
      session.notifyListeners();
      await tester.pumpAndSettle();
      expect(controller.offset, before);
      expect(find.byTooltip('Jump to latest'), findsOneWidget);
      await tester.tap(find.byTooltip('Jump to latest'));
      await tester.pumpAndSettle();
      expect(
        controller.position.maxScrollExtent - controller.offset,
        lessThan(2),
      );
      expect(find.text('Latest result'), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
    },
  );
}
