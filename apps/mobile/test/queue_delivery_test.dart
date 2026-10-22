import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rcode_mobile/ui/queue_panel.dart';

import 'support/page_workspace.dart';

void main() {
  for (final kind in ['text', 'images', 'legacy']) {
    testWidgets(
      'primary queued $kind delivery preserves the original interrupt policy',
      (tester) async {
        final workspace = PageWorkspace();
        final session = workspace.session('chat');
        session.queue = [
          {
            'id': 'queued',
            'text': kind == 'legacy'
                ? 'Check\n\nAttached images (local files — open them to view):\n- /images/old.png'
                : 'Check',
            'attachments': kind == 'images' ? ['/images/new.png'] : <String>[],
          },
        ];
        await tester.pumpWidget(
          app(Scaffold(body: QueuePanel(session: session))),
        );
        await tester.tap(find.byTooltip('Queue actions'));
        await tester.pumpAndSettle();
        await tester.tap(find.text('Send now'));
        await tester.pumpAndSettle();
        final delivery = workspace.calls.single;
        expect(
          delivery.method,
          kind == 'text' ? 'SteerQueuedMessageNow' : 'SendQueuedMessageNow',
        );
        expect(delivery.params['id'], 'queued');
        await tester.pumpWidget(const SizedBox());
        workspace.dispose();
      },
    );
  }

  testWidgets(
    'an open queue menu resolves the same row latest attachment state',
    (tester) async {
      final workspace = PageWorkspace();
      final session = workspace.session('chat');
      session.queue = [
        {'id': 'queued', 'text': 'Check', 'attachments': <String>[]},
      ];
      await tester.pumpWidget(
        app(Scaffold(body: QueuePanel(session: session))),
      );
      await tester.tap(find.byTooltip('Queue actions'));
      await tester.pumpAndSettle();
      session.queue = [
        {
          'id': 'queued',
          'text': 'Check',
          'attachments': ['/images/added.png'],
        },
      ];
      await tester.tap(find.text('Send now'));
      await tester.pumpAndSettle();
      expect(workspace.calls.single.method, 'SendQueuedMessageNow');
      expect(workspace.calls.single.params['id'], 'queued');
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
    },
  );
}
