import 'package:rcode_mobile/ui/sessions_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:rcode_mobile/data/workspace_controller.dart';
import 'package:rcode_mobile/preferences.dart';
import 'package:rcode_mobile/wallpaper.dart';

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

  test('search matches every term across fields, ranks matches and includes archived sessions', () {
    final chats = <JsonMap>[
      {'id': 'title', 'title': 'build fix', 'createdAt': '2026-01-01'},
      {'id': 'archive', 'title': 'build fix', 'archived': true},
      {'id': 'preview', 'title': 'Other', 'lastMessagePreview': 'build fix'},
      {
        'id': 'mixed',
        'title': 'build',
        'branch': 'fix',
        'createdAt': '2026-01-02',
      },
      {'id': 'missing', 'title': 'build'},
      {'id': 'worker', 'title': 'build fix', 'parentChatId': 'title'},
    ];
    final results = projectSessionGroups(
      chats: chats,
      spaces: [],
      preferences: {},
      query: '  BUILD  fix ',
      search: true,
    ).single.rows;
    expect(results.map((row) => row['id']), [
      'title',
      'mixed',
      'archive',
      'preview',
    ]);
    final empty = projectSessionGroups(
      chats: chats,
      spaces: [],
      preferences: {
        'pinnedSessionIds': ['title'],
      },
      search: true,
    );
    expect(empty.single.id, 'search');
    expect(
      empty.single.rows.map((row) => row['id']),
      isNot(contains('archive')),
    );
    expect(
      projectSessionGroups(
        chats: List.generate(70, (index) => {'id': '$index', 'title': 'match'}),
        spaces: [],
        preferences: {},
        query: 'match',
      ).single.rows.length,
      60,
    );
  });

  testWidgets(
    '200 percent text scales the session row and keeps both lines inside it',
    (tester) async {
      final workspace = PageWorkspace();
      workspace.chats = [
        {'id': 'large', 'title': 'Large conversation', 'spaceId': 'project'},
        {'id': 'next', 'title': 'Next conversation', 'spaceId': 'project'},
      ];
      workspace.spaces = [
        {'id': 'project', 'name': 'Project name', 'path': '/project'},
      ];
      final preferences = await AppPreferences.load();
      addTearDown(workspace.dispose);
      addTearDown(preferences.dispose);
      await tester.pumpWidget(
        app(
          Builder(
            builder: (context) => MediaQuery(
              data: MediaQuery.of(context)
                  .copyWith(textScaler: const TextScaler.linear(2)),
              child: SessionsPage(
                workspace: workspace,
                preferences: preferences,
                onOpen: (_) {},
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      final row = find.byKey(const ValueKey('session-large'));
      final next = find.byKey(const ValueKey('session-next'));
      final bounds = tester.getRect(row);
      expect(bounds.height, greaterThanOrEqualTo(124));
      for (final label in ['Large conversation', 'Project name']) {
        final textBounds = tester.getRect(
          find.descendant(of: row, matching: find.text(label)),
        );
        expect(textBounds.top, greaterThanOrEqualTo(bounds.top));
        expect(textBounds.bottom, lessThanOrEqualTo(bounds.bottom));
      }
      expect(tester.getRect(next).top, greaterThanOrEqualTo(bounds.bottom));
      expect(tester.takeException(), null);
    },
  );

  testWidgets(
    'archived search result offers restore and uses the shared wallpaper',
    (tester) async {
      final workspace = PageWorkspace();
      workspace.chats = [
        {'id': 'archive', 'title': 'Old change', 'archived': true},
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
            search: true,
          ),
        ),
      );
      expect(find.byType(WallpaperBackdrop), findsOneWidget);
      await tester.enterText(find.byKey(const Key('session-search')), 'old');
      await tester.pumpAndSettle();
      await tester.longPress(find.text('Old change'));
      await tester.pumpAndSettle();
      expect(find.text('Pin'), findsNothing);
      await tester.tap(find.text('Unarchive'));
      await tester.pumpAndSettle();
      expect(
        workspace.calls.any(
          (call) =>
              call.params['op'] == 'setChatArchived' &&
              call.params['archived'] == false,
        ),
        true,
      );
      expect(tester.takeException(), null);
    },
  );

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
