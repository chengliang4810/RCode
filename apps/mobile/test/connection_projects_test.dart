import 'dart:async';

import 'package:rcode_mobile/ui/connection_page.dart';
import 'package:rcode_mobile/ui/project_browser_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:rcode_mobile/data/workspace_controller.dart';
import 'package:rcode_mobile/preferences.dart';

import 'support/page_workspace.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() => SharedPreferences.setMockInitialValues({}));

  test('folder parent handles Unix roots and Windows drive roots', () {
    expect(parentFolder('/'), null);
    expect(parentFolder('/work/project/'), '/work');
    expect(parentFolder('/work'), '/');
    expect(parentFolder(r'C:\'), null);
    expect(parentFolder(r'C:\work'), 'C:/');
    expect(parentFolder(r'C:\work\project'), 'C:/work');
  });

  testWidgets(
    'pairing masks keys and rejects invalid input before connecting',
    (tester) async {
      final workspace = PageWorkspace();
      addTearDown(workspace.dispose);
      await tester.pumpWidget(app(ConnectionPage(workspace: workspace)));
      await tester.enterText(
        find.byKey(const Key('pair-host')),
        'ws://example.test:27655',
      );
      await tester.enterText(find.byKey(const Key('pair-token')), 'abcdefgh');
      await tester.tap(find.byKey(const Key('pair')));
      await tester.pump();
      expect(workspace.connections, isEmpty);
      expect(find.text('Use at least 8 letters and numbers'), findsOneWidget);
      var key = tester.widget<TextField>(
        find.descendant(
          of: find.byKey(const Key('pair-token')),
          matching: find.byType(TextField),
        ),
      );
      expect(key.obscureText, isTrue);
      await tester.tap(find.byTooltip('Show key'));
      await tester.pump();
      key = tester.widget<TextField>(
        find.descendant(
          of: find.byKey(const Key('pair-token')),
          matching: find.byType(TextField),
        ),
      );
      expect(key.obscureText, isFalse);
      await tester.enterText(
        find.byKey(const Key('pair-token')),
        'Fixture12345',
      );
      await tester.tap(find.byKey(const Key('pair')));
      await tester.pumpAndSettle();
      expect(workspace.connections.single.host, 'ws://example.test:27655');
      expect(workspace.connections.single.key, 'Fixture12345');
      expect(tester.takeException(), null);
    },
  );

  testWidgets(
    'switching folder hosts rejects a late response from the previous host',
    (tester) async {
      final workspace = PageWorkspace();
      addTearDown(workspace.dispose);
      final alpha = Completer<JsonMap>();
      final beta = Completer<JsonMap>();
      workspace.handler = (method, params) async {
        if (method == 'ListFolders')
          return params['targetDeviceId'] == 'a' ? alpha.future : beta.future;
        return {'ok': true};
      };
      await tester.pumpWidget(
        app(
          Builder(
            builder: (context) => Scaffold(
              body: TextButton(
                onPressed: () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => ProjectBrowserPage(workspace: workspace),
                  ),
                ),
                child: const Text('Browse'),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Browse'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 350));
      await tester.tap(find.byKey(const Key('project-device')));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 350));
      await tester.tap(find.text('Beta').last);
      await tester.pump();
      beta.complete({
        'path': '/beta',
        'entries': [
          {'name': '.git', 'isDir': true},
        ],
      });
      await tester.pumpAndSettle();
      alpha.complete({'path': '/alpha', 'entries': []});
      await tester.pumpAndSettle();
      expect(find.text('Use beta'), findsOneWidget);
      expect(find.text('/alpha'), findsNothing);
      await tester.tap(find.byKey(const Key('use-folder')));
      await tester.pumpAndSettle();
      final mutation = workspace.calls
          .where((c) => c.method == 'Mutate')
          .single
          .params;
      expect(mutation['deviceId'], 'b');
      expect(mutation['path'], '/beta');
      expect(mutation['gitDetected'], true);
    },
  );
}
