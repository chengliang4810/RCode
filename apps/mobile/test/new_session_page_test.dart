import 'package:rcode_mobile/ui/new_session_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:rcode_mobile/data/workspace_controller.dart';
import 'package:rcode_mobile/preferences.dart';

import 'support/page_workspace.dart';

class RetryWorkspace extends PageWorkspace {
  var failures = 1;
  @override
  Future<dynamic> call(String method, [JsonMap params = const {}]) async {
    if (method == 'RelayCommand') {
      calls.add((method: method, params: Map.from(params)));
      if (failures-- > 0) throw StateError('Connection interrupted');
      return {'ok': true, 'disposition': 'executed'};
    }
    return super.call(method, params);
  }
}

class EmptyHostWorkspace extends PageWorkspace {
  @override
  Future<dynamic> call(String method, [JsonMap params = const {}]) async {
    if (method == 'ListModels' && params['targetDeviceId'] == 'b') return [];
    return super.call(method, params);
  }
}

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

  testWidgets(
    'first-send retry reuses the chat command and message identities',
    (tester) async {
      final workspace = RetryWorkspace();
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
      await tester.enterText(find.byKey(const Key('new-message')), 'Try once');
      await tester.tap(find.byKey(const Key('new-send')));
      await tester.pumpAndSettle();
      expect(created, null);
      expect(find.textContaining('Connection interrupted'), findsOneWidget);
      await tester.tap(find.byKey(const Key('new-send')));
      await tester.pumpAndSettle();
      final chats = workspace.calls.where(
        (call) => call.params['op'] == 'createChat',
      );
      final relays = workspace.calls
          .where((call) => call.method == 'RelayCommand')
          .toList();
      expect(chats.length, 1);
      expect(relays.length, 2);
      expect(
        relays.first.params['entry']['id'],
        relays.last.params['entry']['id'],
      );
      expect(
        relays.first.params['entry']['payload']['messageId'],
        relays.last.params['entry']['payload']['messageId'],
      );
      expect(created, chats.single.params['chatId']);
      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(milliseconds: 300));
      expect(tester.takeException(), null);
    },
  );

  testWidgets(
    'switching to a host with no models cannot send the previous host model',
    (tester) async {
      final workspace = EmptyHostWorkspace();
      final preferences = await AppPreferences.load();
      addTearDown(workspace.dispose);
      addTearDown(preferences.dispose);
      await tester.pumpWidget(
        app(NewSessionPage(workspace: workspace, preferences: preferences)),
      );
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const Key('new-message')),
        'Must not run',
      );
      await tester.tap(find.byKey(const Key('new-device')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Beta'));
      await tester.pumpAndSettle();
      expect(find.text('No models configured on this device'), findsOneWidget);
      expect(
        tester.widget<IconButton>(find.byKey(const Key('new-send'))).onPressed,
        null,
      );
      expect(
        workspace.calls.where((call) => call.params['op'] == 'createChat'),
        isEmpty,
      );
      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(milliseconds: 300));
    },
  );
}
