import 'package:rcode_mobile/ui/new_session_page.dart';
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
    'new-session catalog and reasoning determine the actual first run',
    (tester) async {
      final workspace = PageWorkspace();
      final preferences = await AppPreferences.load();
      addTearDown(workspace.dispose);
      addTearDown(preferences.dispose);
      String? created;
      await tester.pumpWidget(
        app(
          NewSessionPage(
            workspace: workspace,
            preferences: preferences,
            onCreated: (id) => created = id,
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Fixture Model'), findsOneWidget);
      expect(find.text('medium'), findsOneWidget);
      await tester.enterText(
        find.byKey(const Key('new-message')),
        'Run the actual selected model',
      );
      await tester.tap(find.byKey(const Key('new-send')));
      await tester.pumpAndSettle();
      expect(created, isNotNull);
      final create = workspace.calls
          .firstWhere(
            (c) => c.method == 'Mutate' && c.params['op'] == 'createChat',
          )
          .params;
      expect(create['config']['model'], 'native-model');
      expect(create['config']['reasoning'], 'medium');
      final relay = workspace.calls
          .firstWhere((c) => c.method == 'RelayCommand')
          .params;
      expect(
        relay['entry']['payload']['request']['prompt'],
        'Run the actual selected model',
      );
      expect(relay['entry']['payload']['request']['model'], 'native-model');
      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(milliseconds: 300));
    },
  );
}
