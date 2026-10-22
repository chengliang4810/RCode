import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rcode_mobile/ui/message_text.dart';
import 'package:rcode_mobile/ui/queue_panel.dart';

import 'support/page_workspace.dart';

void main() {
  late PageWorkspace workspace;
  setUp(() {
    workspace = PageWorkspace();
    workspace.chats = [
      {
        'id': 'chat',
        'deviceId': 'a',
        'cwd': '/workspace',
        'config': {'harness': 'native'},
      },
    ];
  });
  tearDown(() => workspace.dispose());
  test(
    'editing attachment lists replaces legacy trailers and retains app context',
    () {
      const raw =
          'Compare.\n\n$attachmentMarker\n- /old.jpg$appshotMarker\n<context>';
      final edited = editedQueuedPrompt(raw, 'Explain.', ['/new.jpg']);
      expect(visiblePrompt(edited), 'Explain.');
      expect(promptImages(edited), ['/new.jpg']);
      expect(edited, endsWith('$appshotMarker\n<context>'));
      final removed = editedQueuedPrompt(raw, 'Explain.', []);
      expect(removed, isNot(contains('/old.jpg')));
      expect(removed, isNot(contains(attachmentMarker)));
    },
  );

  testWidgets(
    'queue edit saves the acquired version and can actually close its route',
    (tester) async {
      final session = workspace.session('chat')..hydrated = true;
      workspace.handler = (method, params) async {
        if (method == 'BeginQueuedMessageEdit')
          return {
            'outcome': 'acquired',
            'leaseId': 'lease',
            'baseTextHash': 'original-hash',
            'text': 'Before',
            'attachments': <String>[],
          };
        if (method == 'FinishQueuedMessageEdit')
          return {
            'outcome': params['action'] == 'commit' ? 'committed' : 'released',
          };
        return {'ok': true};
      };
      await tester.pumpWidget(
        app(
          Builder(
            builder: (context) => Scaffold(
              body: TextButton(
                onPressed: () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) =>
                        QueueEditor(session: session, item: {'id': 'queued'}),
                  ),
                ),
                child: const Text('Open editor'),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open editor'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), 'After');
      await tester.tap(find.text('Save'));
      await tester.pumpAndSettle();
      expect(find.text('Open editor'), findsOneWidget);
      expect(find.byType(QueueEditor), findsNothing);
      final finished = workspace.calls.singleWhere(
        (call) => call.method == 'FinishQueuedMessageEdit',
      );
      expect(finished.params['text'], 'After');
      expect(finished.params['expectedTextHash'], 'original-hash');
      expect(finished.params['attachments'], isEmpty);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    },
  );
}
