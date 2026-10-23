import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:rcode_mobile/preferences.dart';
import 'package:rcode_mobile/ui/new_session_page.dart';

import 'support/page_workspace.dart';

void main() {
  setUp(() => SharedPreferences.setMockInitialValues({}));
  testWidgets(
    'new composer focuses after route entrance and remains scrollable with IME',
    (tester) async {
      final preferences = await AppPreferences.load();
      final workspace = PageWorkspace();
      await tester.pumpWidget(
        app(
          Builder(
            builder: (context) => Scaffold(
              body: TextButton(
                onPressed: () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => NewSessionPage(
                      workspace: workspace,
                      preferences: preferences,
                      autofocus: true,
                    ),
                  ),
                ),
                child: const Text('Open'),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 100));
      var field = tester.widget<TextField>(
        find.byKey(const Key('new-message')),
      );
      expect(field.focusNode!.hasFocus, isFalse);
      await tester.pumpAndSettle();
      field = tester.widget<TextField>(find.byKey(const Key('new-message')));
      expect(field.focusNode!.hasFocus, isTrue);
      expect(find.byType(IntrinsicHeight), findsNothing);
      await tester.pumpWidget(const SizedBox.shrink());
      workspace.dispose();
    },
  );
}
