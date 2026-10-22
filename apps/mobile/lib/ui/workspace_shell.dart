import 'dart:async';
import 'dart:math' as math;
import 'dart:ui';

import 'package:flutter/material.dart';

import '../data/workspace_controller.dart';
import '../l10n.dart';
import '../preferences.dart';
import '../theme.dart';
import 'new_session_page.dart';
import 'sessions_page.dart';
import 'settings_page.dart';

class WorkspaceShell extends StatefulWidget {
  const WorkspaceShell({
    super.key,
    required this.workspace,
    required this.preferences,
    required this.sessionBuilder,
    this.initialChatId,
  });

  final WorkspaceController workspace;
  final AppPreferences preferences;
  final Widget Function(String chatId) sessionBuilder;
  final String? initialChatId;

  @override
  State<WorkspaceShell> createState() => WorkspaceShellState();
}

class WorkspaceShellState extends State<WorkspaceShell>
    with WidgetsBindingObserver {
  int _tab = 0;
  String? _selectedChat;
  bool _wide = false;
  bool _layoutKnown = false;
  bool _layoutSyncScheduled = false;
  bool _transferringSession = false;
  bool _coveringRoute = false;
  String? _phoneChat;
  MaterialPageRoute<void>? _phoneRoute;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _selectedChat = widget.initialChatId;
  }

  @override
  void didChangeMetrics() {
    // An opaque phone route can keep the shell offstage while the window
    // changes size. Observe the view as well as the shell's layout constraints.
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  Future<void> openSession(String id) async {
    if (!mounted) return;
    setState(() {
      _selectedChat = id;
      _tab = 0;
    });
    if (_wide || _transferringSession) {
      _scheduleLayoutSync();
      return;
    }
    await _showPhoneSession(id);
  }

  Future<void> _showPhoneSession(String id) async {
    if (_phoneChat == id && _phoneRoute?.isActive == true) return;
    final navigator = Navigator.of(context);
    final previous = _phoneRoute;
    final route = MaterialPageRoute<void>(
      builder: (_) => widget.sessionBuilder(id),
    );
    setState(() {
      _phoneChat = id;
      _phoneRoute = route;
    });
    if (previous != null && previous.isActive) {
      navigator.popUntil(
        (candidate) => candidate == previous || candidate.isFirst,
      );
      await navigator.pushReplacement<void, void>(route);
    } else {
      await navigator.push(route);
    }
    if (mounted && identical(_phoneRoute, route)) {
      setState(() {
        _phoneRoute = null;
        _phoneChat = null;
        _selectedChat = null;
      });
    }
  }

  void _scheduleLayoutSync() {
    if (_layoutSyncScheduled) return;
    _layoutSyncScheduled = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _layoutSyncScheduled = false;
      if (mounted) unawaited(_syncLayout());
    });
  }

  Future<void> _syncLayout() async {
    if (_transferringSession) return;
    final route = _phoneRoute;
    if (_wide && route != null && route.isActive) {
      final navigator = Navigator.of(context);
      setState(() {
        _transferringSession = true;
        _phoneRoute = null;
        _phoneChat = null;
      });
      navigator.popUntil(
        (candidate) => candidate == route || candidate.isFirst,
      );
      navigator.removeRoute(route);
      // The outgoing page saves its draft while disposing. Create the column's
      // replacement only after its overlay is gone, keeping one live page.
      await route.completed;
      if (!mounted) return;
      setState(() => _transferringSession = false);
      _scheduleLayoutSync();
    } else if (!_wide &&
        _selectedChat != null &&
        _phoneRoute == null &&
        !_coveringRoute) {
      unawaited(_showPhoneSession(_selectedChat!));
    }
  }

  Future<void> _newSession() async {
    if (_wide) {
      setState(() {
        _selectedChat = null;
        _tab = 0;
      });
      return;
    }
    _coveringRoute = true;
    String? id;
    try {
      id = await Navigator.of(context).push<String>(
        MaterialPageRoute(
          fullscreenDialog: true,
          builder: (_) => NewSessionPage(
            workspace: widget.workspace,
            preferences: widget.preferences,
          ),
        ),
      );
    } finally {
      _coveringRoute = false;
    }
    if (id != null && mounted) await openSession(id);
    if (mounted) _scheduleLayoutSync();
  }

  Future<void> _settings() async {
    _coveringRoute = true;
    try {
      await Navigator.of(context).push(
        MaterialPageRoute<void>(
          builder: (_) => SettingsPage(
            workspace: widget.workspace,
            preferences: widget.preferences,
            onOpen: openSession,
          ),
        ),
      );
    } finally {
      _coveringRoute = false;
    }
    if (mounted) _scheduleLayoutSync();
  }

  Widget _connection(BuildContext context) => ListenableBuilder(
    listenable: widget.workspace,
    builder: (context, _) {
      if (widget.workspace.connected) return const SizedBox.shrink();
      final state = widget.workspace.connection;
      final busy =
          state == RpcConnection.connecting ||
          state == RpcConnection.reconnecting;
      return Material(
        color: context.colors.elevated,
        child: SafeArea(
          bottom: false,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
            child: Row(
              children: [
                if (busy)
                  SizedBox(
                    width: 12,
                    height: 12,
                    child: CircularProgressIndicator(
                      strokeWidth: 1.5,
                      color: context.colors.accent,
                    ),
                  )
                else
                  Icon(
                    Icons.cloud_off_outlined,
                    size: 16,
                    color: context.colors.secondary,
                  ),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    busy
                        ? context.tr('正在重新连接', 'Reconnecting')
                        : context.tr('连接已断开', 'Disconnected'),
                    style: TextStyle(
                      fontSize: 13,
                      color: context.colors.secondary,
                    ),
                  ),
                ),
                TextButton(
                  onPressed: busy
                      ? null
                      : () => performUiAction(
                          context,
                          widget.workspace.reconnect,
                        ),
                  child: Text(context.tr('重试', 'Retry')),
                ),
              ],
            ),
          ),
        ),
      );
    },
  );

  Widget _newSessionAccessory(BuildContext context) => ListenableBuilder(
    listenable: widget.workspace,
    builder: (context, _) {
      final working = widget.workspace.sessions
          .where((session) => session['status'] == 'working')
          .length;
      final waiting = widget.workspace.sessions
          .where((session) => session['status'] == 'awaitingInput')
          .length;
      final summary = [
        if (working > 0) context.tr('$working 个工作中', '$working working'),
        if (waiting > 0) context.tr('$waiting 个等待输入', '$waiting need you'),
      ].join(' · ');
      return Padding(
        padding: const EdgeInsets.fromLTRB(16, 6, 16, 8),
        child: ClipRRect(
          borderRadius: BorderRadius.circular(28),
          child: BackdropFilter(
            filter: ImageFilter.blur(sigmaX: 18, sigmaY: 18),
            child: Material(
              color: context.colors.elevated.withValues(alpha: .82),
              child: InkWell(
                key: const Key('new-session'),
                onTap: _newSession,
                child: Container(
                  height: 50,
                  padding: const EdgeInsets.fromLTRB(7, 0, 16, 0),
                  decoration: BoxDecoration(
                    borderRadius: BorderRadius.circular(28),
                    border: Border.all(
                      color: context.colors.hairline.withValues(alpha: .5),
                    ),
                  ),
                  child: Row(
                    children: [
                      Container(
                        width: 34,
                        height: 34,
                        decoration: BoxDecoration(
                          shape: BoxShape.circle,
                          color: context.colors.accent.withValues(alpha: .12),
                        ),
                        child: Icon(
                          Icons.add,
                          size: 20,
                          color: context.colors.accent,
                        ),
                      ),
                      const SizedBox(width: 11),
                      Text(
                        context.tr('新会话', 'New session'),
                        style: const TextStyle(
                          fontSize: 16,
                          fontWeight: FontWeight.w500,
                        ),
                      ),
                      const Spacer(),
                      if (summary.isNotEmpty)
                        Flexible(
                          child: Text(
                            summary,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: TextStyle(
                              fontSize: 12,
                              color: context.colors.secondary,
                            ),
                          ),
                        ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      );
    },
  );

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      final view = View.of(context);
      final size = view.physicalSize / view.devicePixelRatio;
      final wide = size.width >= 840 && size.height >= 520;
      if (!_layoutKnown || _wide != wide) {
        _layoutKnown = true;
        _wide = wide;
        _scheduleLayoutSync();
      }
      if (_wide) {
        final width = math.min(360.0, constraints.maxWidth * .36);
        return Scaffold(
          backgroundColor: context.colors.background,
          body: Column(
            children: [
              _connection(context),
              Expanded(
                child: Row(
                  children: [
                    SizedBox(
                      width: width,
                      child: Column(
                        children: [
                          Expanded(
                            child: SessionsPage(
                              key: ValueKey('wide-sessions-${_tab == 2}'),
                              workspace: widget.workspace,
                              preferences: widget.preferences,
                              onOpen: openSession,
                              currentChatId: _selectedChat,
                              embedded: true,
                              search: _tab == 2,
                            ),
                          ),
                          SafeArea(
                            top: false,
                            child: Padding(
                              padding: const EdgeInsets.fromLTRB(12, 2, 12, 8),
                              child: Row(
                                children: [
                                  IconButton(
                                    key: const Key('sidebar-settings'),
                                    tooltip: context.tr('设置', 'Settings'),
                                    onPressed: _settings,
                                    icon: const Icon(Icons.settings_outlined),
                                  ),
                                  IconButton(
                                    key: const Key('sidebar-search'),
                                    tooltip: context.tr('搜索', 'Search'),
                                    onPressed: () => setState(
                                      () => _tab = _tab == 2 ? 0 : 2,
                                    ),
                                    isSelected: _tab == 2,
                                    icon: const Icon(Icons.search),
                                  ),
                                  const Spacer(),
                                  IconButton(
                                    key: const Key('sidebar-new'),
                                    tooltip: context.tr('新会话', 'New Session'),
                                    onPressed: _newSession,
                                    icon: const Icon(Icons.add),
                                  ),
                                ],
                              ),
                            ),
                          ),
                        ],
                      ),
                    ),
                    VerticalDivider(width: 1, color: context.colors.hairline),
                    Expanded(
                      child: _phoneRoute != null || _transferringSession
                          ? const SizedBox.shrink()
                          : _selectedChat == null
                          ? NewSessionPage(
                              key: const ValueKey('wide-new-session'),
                              workspace: widget.workspace,
                              preferences: widget.preferences,
                              embedded: true,
                              onCreated: openSession,
                            )
                          : KeyedSubtree(
                              key: ValueKey(_selectedChat),
                              child: widget.sessionBuilder(_selectedChat!),
                            ),
                    ),
                  ],
                ),
              ),
            ],
          ),
        );
      }
      return Scaffold(
        backgroundColor: context.colors.background,
        body: Column(
          children: [
            _connection(context),
            Expanded(
              child: IndexedStack(
                index: _tab,
                children: [
                  SessionsPage(
                    workspace: widget.workspace,
                    preferences: widget.preferences,
                    onOpen: openSession,
                  ),
                  SettingsPage(
                    workspace: widget.workspace,
                    preferences: widget.preferences,
                    onOpen: openSession,
                  ),
                  SessionsPage(
                    workspace: widget.workspace,
                    preferences: widget.preferences,
                    onOpen: openSession,
                    search: true,
                  ),
                ],
              ),
            ),
          ],
        ),
        bottomNavigationBar: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            _newSessionAccessory(context),
            NavigationBar(
              key: const Key('workspace-tabs'),
              selectedIndex: _tab,
              height: 66,
              backgroundColor: context.colors.background,
              indicatorColor: context.colors.elevated,
              onDestinationSelected: (index) => setState(() => _tab = index),
              destinations: [
                NavigationDestination(
                  icon: const Icon(Icons.chat_bubble_outline),
                  selectedIcon: const Icon(Icons.chat_bubble),
                  label: context.tr('会话', 'Sessions'),
                ),
                NavigationDestination(
                  icon: const Icon(Icons.settings_outlined),
                  selectedIcon: const Icon(Icons.settings),
                  label: context.tr('设置', 'Settings'),
                ),
                NavigationDestination(
                  icon: const Icon(Icons.search),
                  label: context.tr('搜索', 'Search'),
                ),
              ],
            ),
          ],
        ),
      );
    },
  );
}
