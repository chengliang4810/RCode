import 'dart:async';
import 'dart:typed_data';

import 'package:rcode_mobile/ui/new_session_page.dart';
import 'package:rcode_mobile/ui/attachments.dart';
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

class ReconnectWorkspace extends PageWorkspace {
  var online = false;
  var catalogLoads = 0;
  @override
  bool get connected => online;
  void changeConnection(bool value) {
    online = value;
    notifyListeners();
  }

  @override
  Future<dynamic> call(String method, [JsonMap params = const {}]) async {
    if (method == 'ListHarnesses') {
      catalogLoads++;
      if (!online) {
        throw const RpcException('disconnected', 'server_disconnected');
      }
    }
    return super.call(method, params);
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() {
    SharedPreferences.setMockInitialValues({});
    resetRecoveredImagesForTesting();
  });

  testWidgets(
    'forgetting a host revokes draft writes from the closing new-session page',
    (tester) async {
      resetRecoveredImagesForTesting(retrieval: Future.value([]));
      final workspace = PageWorkspace();
      final preferences = await AppPreferences.load();
      addTearDown(workspace.dispose);
      final oldScope = workspace.cacheScope;
      await tester.pumpWidget(
        app(NewSessionPage(workspace: workspace, preferences: preferences)),
      );
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byKey(const Key('new-message')),
        'Draft on the old host',
      );
      await tester.pump(const Duration(milliseconds: 300));
      expect(
        preferences.captureHostDraft(oldScope).value?['text'],
        'Draft on the old host',
      );
      await AppPreferences.forgetHostDrafts(oldScope);
      workspace.host = 'ws://another-host:27655/';
      final nextScope = workspace.cacheScope;
      await tester.pumpWidget(const SizedBox());
      await tester.pump();
      expect(preferences.captureHostDraft(oldScope).value, isNull);
      expect(preferences.captureHostDraft(nextScope).value, isNull);
    },
  );

  testWidgets(
    'new-session catalog and reasoning determine the actual first run',
    (tester) async {
      resetRecoveredImagesForTesting(retrieval: Future.value([]));
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
      expect(find.byKey(const ValueKey('new-effort')), findsOneWidget);
      expect(
        tester.widget<IconButton>(find.byKey(const Key('new-send'))).onPressed,
        isNotNull,
      );
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
      resetRecoveredImagesForTesting(retrieval: Future.value([]));
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
      resetRecoveredImagesForTesting(retrieval: Future.value([]));
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

  testWidgets('lost picker images restore into the draft without being sent', (
    tester,
  ) async {
    final recovered = Completer<List<UploadAttachment>>();
    resetRecoveredImagesForTesting(retrieval: recovered.future);
    final workspace = PageWorkspace();
    final preferences = await AppPreferences.load();
    addTearDown(workspace.dispose);
    addTearDown(preferences.dispose);
    await tester.pumpWidget(
      app(NewSessionPage(workspace: workspace, preferences: preferences)),
    );
    await tester.pumpAndSettle();
    await tester.enterText(
      find.byKey(const Key('new-message')),
      'Restored draft',
    );
    expect(
      tester.widget<IconButton>(find.byKey(const Key('new-send'))).onPressed,
      null,
    );
    recovered.complete([
      UploadAttachment(name: 'Recovered.jpg', bytes: Uint8List(1)),
    ]);
    await tester.pumpAndSettle();
    expect(find.text('Recovered.jpg'), findsOneWidget);
    expect(
      workspace.calls.where(
        (call) =>
            call.params['op'] == 'createChat' || call.method == 'RelayCommand',
      ),
      isEmpty,
    );
    await tester.pumpWidget(const SizedBox());
    await tester.pump(const Duration(milliseconds: 300));
    expect(tester.takeException(), null);
  });

  testWidgets(
    'connection recovery refreshes the model catalog and live send state',
    (tester) async {
      resetRecoveredImagesForTesting(retrieval: Future.value([]));
      final workspace = ReconnectWorkspace();
      final preferences = await AppPreferences.load();
      addTearDown(workspace.dispose);
      addTearDown(preferences.dispose);
      await tester.pumpWidget(
        app(NewSessionPage(workspace: workspace, preferences: preferences)),
      );
      await tester.pumpAndSettle();
      expect(
        tester.widget<IconButton>(find.byKey(const Key('new-send'))).onPressed,
        null,
      );
      expect(workspace.catalogLoads, 1);
      workspace.changeConnection(true);
      await tester.pumpAndSettle();
      expect(workspace.catalogLoads, 2);
      expect(
        tester.widget<IconButton>(find.byKey(const Key('new-send'))).onPressed,
        isNotNull,
      );
      workspace.changeConnection(false);
      await tester.pumpAndSettle();
      expect(
        tester.widget<IconButton>(find.byKey(const Key('new-send'))).onPressed,
        null,
      );
      await tester.pumpWidget(const SizedBox());
      await tester.pump(const Duration(milliseconds: 300));
      expect(tester.takeException(), null);
    },
  );
}
