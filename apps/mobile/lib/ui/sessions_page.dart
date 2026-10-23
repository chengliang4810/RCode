import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../data/workspace_controller.dart';
import '../l10n.dart';
import '../preferences.dart';
import '../theme.dart';
import '../wallpaper.dart';
import 'project_browser_page.dart';

String sessionTitle(Map<String, dynamic> chat) =>
    (chat['title'] as String?)?.trim().isNotEmpty == true
    ? chat['title'] as String
    : (chat['lastMessagePreview'] as String?)?.trim().isNotEmpty == true
    ? chat['lastMessagePreview'] as String
    : '';

bool sessionUnseen(Map<String, dynamic> chat) {
  final message = DateTime.tryParse(chat['lastMessageAt'] as String? ?? '');
  final seen = DateTime.tryParse(chat['lastSeenAt'] as String? ?? '');
  return message != null && (seen == null || message.isAfter(seen));
}

List<String> sectionSessions(Map<String, dynamic> section) =>
    ((section['session_ids'] ?? section['sessionIds']) as List? ?? [])
        .whereType<String>()
        .toList();

class SessionGroup {
  const SessionGroup(
    this.id,
    this.title,
    this.rows, {
    this.userSection = false,
  });
  final String id;
  final String title;
  final List<Map<String, dynamic>> rows;
  final bool userSection;
}

/// One ordered projection shared by the phone list and tablet sidebar.
List<SessionGroup> projectSessionGroups({
  required List<Map<String, dynamic>> chats,
  required List<Map<String, dynamic>> spaces,
  required Map<String, dynamic> preferences,
  bool archived = false,
  bool byProject = false,
  bool search = false,
  String query = '',
  String? folderId,
  String pinnedTitle = 'Pinned',
  String recentTitle = 'Recent',
  String homeTitle = 'Home',
}) {
  final terms = query.trim().toLowerCase().split(RegExp(r'\s+'))
    ..removeWhere((term) => term.isEmpty);
  final rows = chats
      .where(
        (chat) =>
            (terms.isNotEmpty || (chat['archived'] == true) == archived) &&
            chat['parentChatId'] == null,
      )
      .toList();
  int recency(Map<String, dynamic> a, Map<String, dynamic> b) {
    final activity = (b['lastMessageAt'] ?? b['createdAt'] ?? '')
        .toString()
        .compareTo((a['lastMessageAt'] ?? a['createdAt'] ?? '').toString());
    return activity != 0
        ? activity
        : (a['id'] as String).compareTo(b['id'] as String);
  }

  rows.sort(recency);
  final lookup = {for (final row in rows) row['id'] as String: row};
  final pinned = (preferences['pinnedSessionIds'] as List? ?? [])
      .whereType<String>()
      .toList();
  final sections = jsonRows(preferences['sections']);
  final projects = {for (final space in spaces) space['id'] as String: space};
  if (terms.isNotEmpty) {
    final scores = <String, int>{};
    final hits = rows.where((chat) {
      final fields = <(String, int)>[
        (sessionTitle(chat).toLowerCase(), 100),
        (
          (projects[chat['spaceId']]?['name'] as String? ?? '').toLowerCase(),
          40,
        ),
        ((chat['branch'] as String? ?? '').toLowerCase(), 30),
        ((chat['lastMessagePreview'] as String? ?? '').toLowerCase(), 20),
      ];
      var total = 0;
      for (final term in terms) {
        var best = 0;
        for (final (text, weight) in fields) {
          final at = text.indexOf(term);
          if (at < 0) continue;
          final boundary =
              at == 0 || !RegExp(r'[a-z0-9]').hasMatch(text[at - 1]);
          final score = weight + (boundary ? weight ~/ 2 : 0);
          if (score > best) best = score;
        }
        if (best == 0) return false;
        total += best;
      }
      scores[chat['id'] as String] = chat['archived'] == true
          ? total ~/ 2
          : total;
      return true;
    }).toList();
    hits.sort((a, b) {
      final rank = scores[b['id']]!.compareTo(scores[a['id']]!);
      return rank != 0 ? rank : recency(a, b);
    });
    return [SessionGroup('search', '', hits.take(60).toList())];
  }
  if (search) return [SessionGroup('search', '', rows)];
  if (folderId != null) {
    final ids = folderId == 'pinned'
        ? pinned
        : sectionSessions(
            sections
                    .where((section) => section['id'] == folderId)
                    .firstOrNull ??
                {},
          );
    return [
      SessionGroup(
        folderId,
        '',
        ids.where(lookup.containsKey).map((id) => lookup[id]!).toList(),
      ),
    ];
  }
  if (archived) return [SessionGroup('archived', '', rows)];
  final groups = <SessionGroup>[];
  final placed = <String>{};
  final pinRows = pinned
      .where(lookup.containsKey)
      .map((id) => lookup[id]!)
      .toList();
  if (pinRows.isNotEmpty) {
    groups.add(SessionGroup('pinned', pinnedTitle, pinRows));
    placed.addAll(pinRows.map((row) => row['id'] as String));
  }
  for (final section in sections) {
    final members = sectionSessions(section)
        .where((id) => lookup.containsKey(id) && !placed.contains(id))
        .map((id) => lookup[id]!)
        .toList();
    groups.add(
      SessionGroup(
        section['id'] as String,
        section['name'] as String? ?? '',
        members,
        userSection: true,
      ),
    );
    placed.addAll(members.map((row) => row['id'] as String));
  }
  final recent = rows.where((row) => !placed.contains(row['id'])).toList();
  if (byProject) {
    final buckets = <String, List<Map<String, dynamic>>>{};
    for (final row in recent) {
      final project = projects[row['spaceId']];
      final key =
          (project?['repositoryId'] ??
                  project?['path'] ??
                  row['spaceId'] ??
                  'home')
              .toString();
      buckets.putIfAbsent(key, () => []).add(row);
    }
    for (final entry in buckets.entries) {
      final project = projects[entry.value.first['spaceId']];
      final name =
          project?['name'] as String? ??
          fileName(project?['path'] as String? ?? homeTitle);
      groups.add(SessionGroup('project:${entry.key}', name, entry.value));
    }
  } else if (recent.isNotEmpty || groups.isEmpty) {
    groups.add(
      SessionGroup('recent', groups.isEmpty ? '' : recentTitle, recent),
    );
  }
  return groups;
}

Future<void> performUiAction(
  BuildContext context,
  Future<void> Function() action,
) async {
  try {
    await action();
  } catch (error) {
    if (context.mounted) {
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text(context.errorText(error))));
    }
  }
}

Future<String?> promptText(
  BuildContext context, {
  required String title,
  String initial = '',
  String? submit,
}) async {
  final controller = TextEditingController(text: initial);
  final result = await showDialog<String>(
    context: context,
    builder: (context) => AlertDialog(
      title: Text(title),
      content: TextField(
        controller: controller,
        autofocus: true,
        maxLength: 120,
        onSubmitted: (value) {
          if (value.trim().isNotEmpty) Navigator.pop(context, value.trim());
        },
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: Text(context.tr('取消', 'Cancel')),
        ),
        FilledButton(
          onPressed: () {
            if (controller.text.trim().isNotEmpty) {
              Navigator.pop(context, controller.text.trim());
            }
          },
          child: Text(submit ?? context.tr('保存', 'Save')),
        ),
      ],
    ),
  );
  // The dialog can still be animating while its text field detaches.
  Future<void>.delayed(const Duration(milliseconds: 300), controller.dispose);
  return result;
}

class SessionsPage extends StatefulWidget {
  const SessionsPage({
    super.key,
    required this.workspace,
    required this.preferences,
    required this.onOpen,
    this.currentChatId,
    this.search = false,
    this.archived = false,
    this.folderId,
    this.folderTitle,
    this.embedded = false,
    this.searchFocusNode,
  });

  final WorkspaceController workspace;
  final AppPreferences preferences;
  final ValueChanged<String> onOpen;
  final String? currentChatId;
  final bool search;
  final bool archived;
  final String? folderId;
  final String? folderTitle;
  final bool embedded;
  final FocusNode? searchFocusNode;

  @override
  State<SessionsPage> createState() => _SessionsPageState();
}

class _SessionsPageState extends State<SessionsPage> {
  final _query = TextEditingController();
  Set<String> _collapsed = {};

  @override
  void initState() {
    super.initState();
    final saved = widget.preferences.getJson('collapsedSections') ?? {};
    _collapsed = saved.entries
        .where((entry) => entry.value == true)
        .map((entry) => entry.key)
        .toSet();
  }

  @override
  void dispose() {
    _query.dispose();
    super.dispose();
  }

  Future<void> _change(Map<String, dynamic> change) => widget.workspace
      .call('Mutate', {'op': 'changeSidebarPin', 'change': change})
      .then((_) {});

  Future<void> _section(Map<String, dynamic> change) =>
      _change({'action': 'section', 'change': change});

  void _collapse(String id) {
    setState(() {
      if (!_collapsed.remove(id)) _collapsed.add(id);
    });
    widget.preferences.setJson('collapsedSections', {
      for (final id in _collapsed) id: true,
    });
  }

  Future<void> _newSection({String? moveChat}) async {
    final name = await promptText(
      context,
      title: context.tr('新建分组', 'New Section'),
    );
    if (name == null || !mounted) return;
    final id = mobileId('section');
    await _section({'action': 'create', 'id': id, 'name': name});
    if (moveChat != null) {
      await _section({
        'action': 'assign',
        'sessionId': moveChat,
        'sectionId': id,
      });
    }
  }

  Future<void> _move(String id) async {
    final sections = jsonRows(widget.workspace.sidebarPreferences['sections']);
    final choice = await showModalBottomSheet<String>(
      context: context,
      showDragHandle: true,
      builder: (context) => SafeArea(
        child: ListView(
          shrinkWrap: true,
          children: [
            ListTile(
              leading: const Icon(Icons.push_pin_outlined),
              title: Text(context.tr('置顶', 'Pinned')),
              onTap: () => Navigator.pop(context, 'pinned'),
            ),
            ListTile(
              leading: const Icon(Icons.inbox_outlined),
              title: Text(context.tr('最近', 'Recent')),
              onTap: () => Navigator.pop(context, 'recent'),
            ),
            for (final section in sections)
              ListTile(
                leading: const Icon(Icons.folder_outlined),
                title: Text(section['name'] as String),
                onTap: () => Navigator.pop(context, section['id'] as String),
              ),
            ListTile(
              leading: const Icon(Icons.create_new_folder_outlined),
              title: Text(context.tr('新建分组', 'New Section')),
              onTap: () => Navigator.pop(context, 'new'),
            ),
          ],
        ),
      ),
    );
    if (choice == null || !mounted) return;
    await performUiAction(context, () async {
      if (choice == 'new') return _newSection(moveChat: id);
      await _place(id, choice);
    });
  }

  Future<void> _place(
    String id,
    String group, {
    String? before,
    String? after,
  }) async {
    if (group == 'pinned') {
      await _change({
        'action': 'pin',
        'sessionId': id,
        'before': before,
        'after': after,
      });
    } else if (group == 'recent') {
      await _section({'action': 'assign', 'sessionId': id, 'sectionId': null});
    } else if (!group.startsWith('project:') &&
        group != 'search' &&
        group != 'archived') {
      await _section({
        'action': 'assign',
        'sessionId': id,
        'sectionId': group,
        'before': before,
        'after': after,
      });
    }
  }

  Future<void> _rowMenu(Map<String, dynamic> chat) async {
    final id = chat['id'] as String;
    final archived = chat['archived'] == true;
    final pinned =
        (widget.workspace.sidebarPreferences['pinnedSessionIds'] as List? ?? [])
            .contains(id);
    final choice = await showModalBottomSheet<String>(
      context: context,
      showDragHandle: true,
      builder: (context) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (!archived)
              ListTile(
                leading: Icon(
                  pinned ? Icons.push_pin : Icons.push_pin_outlined,
                ),
                title: Text(
                  pinned
                      ? context.tr('取消置顶', 'Unpin')
                      : context.tr('置顶', 'Pin'),
                ),
                onTap: () => Navigator.pop(context, 'pin'),
              ),
            if (!archived)
              ListTile(
                leading: const Icon(Icons.drive_file_move_outline),
                title: Text(context.tr('移动到分组', 'Move to Section')),
                onTap: () => Navigator.pop(context, 'move'),
              ),
            ListTile(
              leading: const Icon(Icons.edit_outlined),
              title: Text(context.tr('重命名', 'Rename')),
              onTap: () => Navigator.pop(context, 'rename'),
            ),
            ListTile(
              leading: Icon(
                archived ? Icons.unarchive_outlined : Icons.archive_outlined,
              ),
              title: Text(
                archived
                    ? context.tr('取消归档', 'Unarchive')
                    : context.tr('归档', 'Archive'),
              ),
              onTap: () => Navigator.pop(context, 'archive'),
            ),
          ],
        ),
      ),
    );
    if (choice == null || !mounted) return;
    await performUiAction(context, () async {
      switch (choice) {
        case 'pin':
          await _change({'action': pinned ? 'unpin' : 'pin', 'sessionId': id});
        case 'move':
          await _move(id);
        case 'rename':
          final name = await promptText(
            context,
            title: context.tr('重命名会话', 'Rename Session'),
            initial: sessionTitle(chat),
          );
          if (name != null) {
            await widget.workspace.call('Mutate', {
              'op': 'renameChat',
              'chatId': id,
              'title': name,
            });
          }
        case 'archive':
          await widget.workspace.call('Mutate', {
            'op': 'setChatArchived',
            'chatId': id,
            'archived': !archived,
          });
      }
    });
  }

  Future<void> _headerMenu(SessionGroup group) async {
    final choice = await showModalBottomSheet<String>(
      context: context,
      showDragHandle: true,
      builder: (context) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              leading: const Icon(Icons.open_in_new),
              title: Text(context.tr('打开分组', 'Open Section')),
              onTap: () => Navigator.pop(context, 'open'),
            ),
            if (group.userSection) ...[
              ListTile(
                leading: const Icon(Icons.edit_outlined),
                title: Text(context.tr('重命名分组', 'Rename Section')),
                onTap: () => Navigator.pop(context, 'rename'),
              ),
              ListTile(
                leading: const Icon(Icons.delete_outline),
                title: Text(context.tr('删除分组', 'Delete Section')),
                onTap: () => Navigator.pop(context, 'delete'),
              ),
            ],
          ],
        ),
      ),
    );
    if (!mounted) return;
    await performUiAction(context, () async {
      if (choice == 'open') {
        await Navigator.of(context).push(
          MaterialPageRoute<void>(
            builder: (_) => SessionsPage(
              workspace: widget.workspace,
              preferences: widget.preferences,
              onOpen: widget.onOpen,
              folderId: group.id,
              folderTitle: group.title,
            ),
          ),
        );
      } else if (choice == 'rename') {
        final name = await promptText(
          context,
          title: context.tr('重命名分组', 'Rename Section'),
          initial: group.title,
        );
        if (name != null) {
          await _section({'action': 'rename', 'id': group.id, 'name': name});
        }
      } else if (choice == 'delete') {
        await _section({'action': 'delete', 'id': group.id});
      }
    });
  }

  Widget _row(Map<String, dynamic> chat, SessionGroup group) {
    final id = chat['id'] as String;
    final archived = chat['archived'] == true;
    final spaces = widget.workspace.spaces.where(
      (space) => space['id'] == chat['spaceId'],
    );
    final space = spaces.firstOrNull;
    final project =
        space?['name'] as String? ??
        fileName(space?['path'] as String? ?? context.tr('主页', 'Home'));
    final state = widget.workspace.sessions
        .where((session) => session['chatId'] == id)
        .firstOrNull;
    final status = state?['status'] as String? ?? 'idle';
    final unseen = sessionUnseen(chat);
    final current = widget.currentChatId == id;
    final colors = context.colors;
    final textScaler = MediaQuery.textScalerOf(context);
    final rowScale = math.max(
      1.0,
      math.max(textScaler.scale(16.5) / 16.5, textScaler.scale(13.5) / 13.5),
    );
    final label = switch (status) {
      'working' => context.tr('工作中', 'Working'),
      'awaitingInput' => context.tr('等待输入', 'Input'),
      'errored' => context.tr('失败', 'Failed'),
      _ => unseen ? context.tr('已完成', 'Done') : _timeLabel(chat),
    };
    final tone = switch (status) {
      'errored' => colors.danger,
      'working' || 'awaitingInput' => colors.accent,
      _ => unseen ? colors.success : colors.tertiary,
    };
    final row = Material(
      color: current ? colors.elevated : Colors.transparent,
      borderRadius: BorderRadius.circular(16),
      child: InkWell(
        key: ValueKey('session-$id'),
        borderRadius: BorderRadius.circular(16),
        onTap: () {
          performUiAction(
            context,
            () => widget.workspace
                .call('Mutate', {'op': 'markChatSeen', 'chatId': id})
                .then((_) {}),
          );
          widget.onOpen(id);
        },
        onLongPress: () => _rowMenu(chat),
        child: SizedBox(
          height: 62 * rowScale,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12),
            child: Row(
              children: [
                LongPressDraggable<String>(
                  data: id,
                  maxSimultaneousDrags: archived ? 0 : 1,
                  feedback: Material(
                    elevation: 6,
                    borderRadius: BorderRadius.circular(16),
                    child: SizedBox(
                      width: 280,
                      child: ListTile(
                        leading: const Icon(Icons.auto_awesome_outlined),
                        title: Text(
                          sessionTitle(chat),
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                        ),
                      ),
                    ),
                  ),
                  child: Semantics(
                    label: context.tr('拖动会话', 'Drag session'),
                    child: Icon(
                      Icons.auto_awesome_outlined,
                      size: 20,
                      color: colors.text,
                    ),
                  ),
                ),
                const SizedBox(width: 14),
                Expanded(
                  child: Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: [
                          Expanded(
                            child: Text(
                              sessionTitle(chat).isEmpty
                                  ? context.tr('新会话', 'New session')
                                  : sessionTitle(chat),
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(
                                fontSize: 16.5,
                                fontWeight: unseen
                                    ? FontWeight.w600
                                    : FontWeight.w500,
                                color: colors.text,
                              ),
                            ),
                          ),
                          const SizedBox(width: 8),
                          if (status == 'working')
                            SizedBox(
                              width: 10,
                              height: 10,
                              child: CircularProgressIndicator(
                                strokeWidth: 1.5,
                                color: tone,
                              ),
                            ),
                          if (status == 'awaitingInput')
                            Icon(Icons.circle, size: 7, color: tone),
                          if (status == 'working' || status == 'awaitingInput')
                            const SizedBox(width: 5),
                          Text(
                            label,
                            style: TextStyle(fontSize: 12, color: tone),
                          ),
                        ],
                      ),
                      const SizedBox(height: 5),
                      Row(
                        children: [
                          Icon(
                            space == null
                                ? Icons.home_outlined
                                : Icons.folder_outlined,
                            size: 13,
                            color: colors.secondary,
                          ),
                          const SizedBox(width: 5),
                          Flexible(
                            child: Text(
                              project,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(
                                fontSize: 13.5,
                                color: colors.secondary,
                              ),
                            ),
                          ),
                          if ((chat['branch'] as String?)?.isNotEmpty ==
                              true) ...[
                            const SizedBox(width: 8),
                            Icon(
                              Icons.call_split,
                              size: 12,
                              color: colors.tertiary,
                            ),
                            const SizedBox(width: 3),
                            Flexible(
                              child: Text(
                                chat['branch'] as String,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: TextStyle(
                                  fontSize: 12.5,
                                  color: colors.tertiary,
                                ),
                              ),
                            ),
                          ],
                        ],
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
    final pinned =
        (widget.workspace.sidebarPreferences['pinnedSessionIds'] as List? ?? [])
            .contains(id);
    final draggable = Dismissible(
      key: ValueKey('swipe:$id'),
      direction: archived
          ? DismissDirection.endToStart
          : DismissDirection.horizontal,
      background: Container(
        alignment: Alignment.centerLeft,
        padding: const EdgeInsets.only(left: 24),
        color: colors.accent,
        child: const Icon(Icons.push_pin_outlined, color: Colors.white),
      ),
      secondaryBackground: Container(
        alignment: Alignment.centerRight,
        padding: const EdgeInsets.only(right: 24),
        color: colors.secondary,
        child: Icon(
          archived ? Icons.unarchive_outlined : Icons.archive_outlined,
          color: Colors.white,
        ),
      ),
      confirmDismiss: (direction) async {
        await performUiAction(context, () async {
          if (!archived && direction == DismissDirection.startToEnd) {
            await _change({
              'action': pinned ? 'unpin' : 'pin',
              'sessionId': id,
            });
          } else {
            await widget.workspace.call('Mutate', {
              'op': 'setChatArchived',
              'chatId': id,
              'archived': !archived,
            });
          }
        });
        return false;
      },
      child: row,
    );
    if (archived) return draggable;
    if (group.id.startsWith('project:') || group.id == 'search') {
      return draggable;
    }
    return DragTarget<String>(
      onWillAcceptWithDetails: (details) => details.data != id,
      onAcceptWithDetails: (details) => performUiAction(
        context,
        () => _place(details.data, group.id, before: id),
      ),
      builder: (context, candidates, rejected) => DecoratedBox(
        decoration: BoxDecoration(
          border: candidates.isEmpty
              ? null
              : Border(top: BorderSide(color: colors.accent, width: 2)),
        ),
        child: draggable,
      ),
    );
  }

  String _timeLabel(Map<String, dynamic> chat) {
    final time = DateTime.tryParse(
      (chat['lastMessageAt'] ?? chat['createdAt'] ?? '') as String,
    );
    if (time == null) return '';
    final age = DateTime.now().difference(time);
    if (age.inMinutes < 1) return context.tr('刚刚', 'Now');
    if (age.inHours < 1) {
      return context.tr('${age.inMinutes}分', '${age.inMinutes}m');
    }
    if (age.inDays < 1) return context.tr('${age.inHours}时', '${age.inHours}h');
    if (age.inDays < 7) return context.tr('${age.inDays}天', '${age.inDays}d');
    return '${time.month}/${time.day}';
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([widget.workspace, widget.preferences]),
    builder: (context, _) {
      final colors = context.colors;
      final groups = projectSessionGroups(
        chats: widget.workspace.chats,
        spaces: widget.workspace.spaces,
        preferences: widget.workspace.sidebarPreferences,
        archived: widget.archived,
        byProject: widget.preferences.groupByProject,
        search: widget.search,
        query: _query.text,
        folderId: widget.folderId,
        pinnedTitle: context.tr('置顶', 'Pinned'),
        recentTitle: context.tr('最近', 'Recent'),
        homeTitle: context.tr('主页', 'Home'),
      );
      return Scaffold(
        backgroundColor: colors.background,
        appBar: AppBar(
          title: Text(
            widget.folderTitle ??
                (widget.search
                    ? context.tr('搜索', 'Search')
                    : widget.archived
                    ? context.tr('已归档会话', 'Archived Sessions')
                    : context.tr('会话', 'Sessions')),
          ),
          titleTextStyle: TextStyle(
            fontFamily: 'Geist',
            fontSize: widget.embedded ? 24 : 30,
            fontWeight: FontWeight.w600,
            color: colors.text,
          ),
          actions: widget.search || widget.archived || widget.folderId != null
              ? null
              : [
                  PopupMenuButton<String>(
                    key: const Key('sessions-options'),
                    tooltip: context.tr('更多', 'More'),
                    onSelected: (choice) => performUiAction(context, () async {
                      if (choice == 'new') await _newSection();
                      if (choice == 'collapse') {
                        setState(
                          () => _collapsed = groups
                              .map((group) => group.id)
                              .toSet(),
                        );
                        await widget.preferences.setJson('collapsedSections', {
                          for (final id in _collapsed) id: true,
                        });
                      }
                      if (choice == 'group') {
                        await widget.preferences.setValue(
                          'groupByProject',
                          !widget.preferences.groupByProject,
                        );
                      }
                      if (choice == 'archived' && context.mounted) {
                        await Navigator.of(context).push(
                          MaterialPageRoute<void>(
                            builder: (_) => SessionsPage(
                              workspace: widget.workspace,
                              preferences: widget.preferences,
                              onOpen: widget.onOpen,
                              archived: true,
                            ),
                          ),
                        );
                      }
                    }),
                    itemBuilder: (_) => [
                      PopupMenuItem(
                        value: 'new',
                        child: Text(context.tr('新建分组', 'New Section')),
                      ),
                      PopupMenuItem(
                        value: 'collapse',
                        child: Text(context.tr('全部折叠', 'Collapse All')),
                      ),
                      CheckedPopupMenuItem(
                        value: 'group',
                        checked: widget.preferences.groupByProject,
                        child: Text(context.tr('按项目分组', 'Group by Project')),
                      ),
                      PopupMenuItem(
                        value: 'archived',
                        child: Text(context.tr('已归档', 'Archived')),
                      ),
                    ],
                  ),
                ],
        ),
        body: WallpaperBackdrop(
          preferences: widget.preferences,
          child: Column(
            children: [
              if (widget.search)
                Padding(
                  padding: const EdgeInsets.fromLTRB(20, 4, 20, 12),
                  child: TextField(
                    key: const Key('session-search'),
                    controller: _query,
                    focusNode: widget.searchFocusNode,
                    onChanged: (_) => setState(() {}),
                    autofocus: !widget.embedded,
                    decoration: InputDecoration(
                      prefixIcon: const Icon(Icons.search),
                      hintText: context.tr(
                        '会话、项目、分支',
                        'Sessions, projects, branches',
                      ),
                      suffixIcon: _query.text.isEmpty
                          ? null
                          : IconButton(
                              tooltip: context.tr('清空', 'Clear'),
                              icon: const Icon(Icons.close),
                              onPressed: () => setState(_query.clear),
                            ),
                    ),
                  ),
                ),
              Expanded(
                child: RefreshIndicator(
                  onRefresh: widget.workspace.reconnect,
                  child: CustomScrollView(
                    physics: const AlwaysScrollableScrollPhysics(),
                    slivers: [
                      SliverPadding(
                        padding: const EdgeInsets.fromLTRB(8, 4, 8, 100),
                        sliver: SliverMainAxisGroup(
                          slivers: [
                            if (groups.every((group) => group.rows.isEmpty) &&
                                !groups.any((group) => group.userSection))
                              SliverToBoxAdapter(
                                child: Padding(
                                  padding: const EdgeInsets.only(top: 96),
                                  child: Center(
                                    child: Text(
                                      widget.search
                                          ? context.tr(
                                              '没有匹配的会话',
                                              'No matching sessions',
                                            )
                                          : widget.archived
                                          ? context.tr(
                                              '没有已归档会话',
                                              'No archived sessions',
                                            )
                                          : context.tr(
                                              '点击新建会话开始',
                                              'Tap New session to begin',
                                            ),
                                      style: TextStyle(color: colors.secondary),
                                    ),
                                  ),
                                ),
                              ),
                            for (final group in groups) ...[
                              if (group.title.isNotEmpty)
                                SliverToBoxAdapter(
                                  child: DragTarget<String>(
                                    onWillAcceptWithDetails: (_) =>
                                        group.id == 'pinned' ||
                                        group.id == 'recent' ||
                                        group.userSection,
                                    onAcceptWithDetails: (details) =>
                                        performUiAction(
                                          context,
                                          () => _place(details.data, group.id),
                                        ),
                                    builder: (context, candidates, rejected) =>
                                        InkWell(
                                          onTap: () => _collapse(group.id),
                                          onLongPress:
                                              group.userSection ||
                                                  group.id == 'pinned'
                                              ? () => _headerMenu(group)
                                              : null,
                                          borderRadius: BorderRadius.circular(
                                            10,
                                          ),
                                          child: Container(
                                            color: candidates.isNotEmpty
                                                ? colors.accent.withValues(
                                                    alpha: .1,
                                                  )
                                                : null,
                                            padding: const EdgeInsets.fromLTRB(
                                              12,
                                              18,
                                              12,
                                              8,
                                            ),
                                            child: Row(
                                              children: [
                                                Icon(
                                                  _collapsed.contains(group.id)
                                                      ? Icons.chevron_right
                                                      : Icons
                                                            .keyboard_arrow_down,
                                                  size: 18,
                                                  color: colors.secondary,
                                                ),
                                                const SizedBox(width: 5),
                                                Expanded(
                                                  child: Text(
                                                    group.title,
                                                    style: TextStyle(
                                                      fontSize: 13,
                                                      fontWeight:
                                                          FontWeight.w600,
                                                      color: colors.secondary,
                                                    ),
                                                  ),
                                                ),
                                                Text(
                                                  '${group.rows.length}',
                                                  style: TextStyle(
                                                    fontSize: 12,
                                                    color: colors.tertiary,
                                                  ),
                                                ),
                                              ],
                                            ),
                                          ),
                                        ),
                                  ),
                                ),
                              if (!_collapsed.contains(group.id))
                                SliverList.builder(
                                  itemCount: group.rows.length,
                                  itemBuilder: (_, index) =>
                                      _row(group.rows[index], group),
                                ),
                            ],
                          ],
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ],
          ),
        ),
      );
    },
  );
}
