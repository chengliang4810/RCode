import 'package:rcode_mobile/ui/sessions_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:rcode_mobile/data/workspace_controller.dart';
import 'package:rcode_mobile/preferences.dart';

import 'support/page_workspace.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() => SharedPreferences.setMockInitialValues({}));

  test('list projection preserves pin and section order, filters workers and archived sessions', () {
    final chats = <JsonMap>[
      {
        'id': 'recent',
        'title': 'Newest',
        'archived': false,
        'createdAt': '2026-01-04',
        'spaceId': 'p',
      },
      {'id': 'p2', 'archived': false, 'createdAt': '2026-01-02'},
      {'id': 'p1', 'archived': false, 'createdAt': '2026-01-01'},
      {'id': 's1', 'archived': false},
      {'id': 'worker', 'archived': false, 'parentChatId': 'recent'},
      {'id': 'archive', 'archived': true},
    ];
    final preferences = <String, dynamic>{
      'pinnedSessionIds': ['p1', 'p2'],
      'sections': [
        {
          'id': 'section',
          'name': 'Plan',
          'session_ids': ['s1', 'p1'],
        },
      ],
    };
    final groups = projectSessionGroups(
      chats: chats,
      spaces: [
        {'id': 'p', 'name': 'Project'},
      ],
      preferences: preferences,
    );
    expect(groups.map((g) => g.id), ['pinned', 'section', 'recent']);
    expect(groups[0].rows.map((c) => c['id']), ['p1', 'p2']);
    expect(groups[1].rows.map((c) => c['id']), ['s1']);
    expect(groups[2].rows.map((c) => c['id']), ['recent']);
    final search = projectSessionGroups(
      chats: chats,
      spaces: [
        {'id': 'p', 'name': 'Project'},
      ],
      preferences: preferences,
      query: 'project',
    );
    expect(search.single.rows.single['id'], 'recent');
    expect(
      projectSessionGroups(
        chats: chats,
        spaces: [],
        preferences: preferences,
        archived: true,
      ).single.rows.single['id'],
      'archive',
    );
  });

  testWidgets('session context actions send real pin and archive mutations', (
    tester,
  ) async {
    final workspace = PageWorkspace();
    workspace.chats = [
      {'id': 'chat', 'title': 'Actual chat', 'archived': false},
    ];
    final preferences = await AppPreferences.load();
    addTearDown(workspace.dispose);
    addTearDown(preferences.dispose);
    await tester.pumpWidget(
      app(
        SessionsPage(
          workspace: workspace,
          preferences: preferences,
          onOpen: (_) {},
        ),
      ),
    );
    await tester.longPress(find.text('Actual chat'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Pin'));
    await tester.pumpAndSettle();
    expect(
      workspace.calls.any(
        (c) =>
            c.params['op'] == 'changeSidebarPin' &&
            c.params['change']['action'] == 'pin',
      ),
      true,
    );
    await tester.longPress(find.text('Actual chat'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Archive'));
    await tester.pumpAndSettle();
    expect(
      workspace.calls.any(
        (c) =>
            c.params['op'] == 'setChatArchived' && c.params['archived'] == true,
      ),
      true,
    );
    expect(tester.takeException(), null);
  });
}
