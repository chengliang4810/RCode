import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';

import '../data/workspace_controller.dart';
import '../l10n.dart';
import '../preferences.dart';
import '../theme.dart';
import '../visual/rcode_visual.dart';
import 'attachments.dart';
import 'message_text.dart';
import 'message_view.dart';
import 'question_panel.dart';
import 'queue_panel.dart';
import 'transcript_markdown.dart' show openExternalLink;

class SessionPage extends StatefulWidget {
  const SessionPage({
    super.key,
    required this.workspace,
    required this.chatId,
    required this.preferences,
  });
  final WorkspaceController workspace;
  final String chatId;
  final AppPreferences preferences;
  @override
  State<SessionPage> createState() => _SessionPageState();
}

class _SessionPageState extends State<SessionPage> {
  late SessionController _session;
  final _text = TextEditingController();
  final _focus = FocusNode();
  final _scroll = ScrollController();
  final List<UploadAttachment> _attachments = [];
  List<JsonMap> _mentions = [];
  Timer? _save, _search;
  int _searchGeneration = 0;
  String _signature = '';
  bool _following = true, _jump = false, _picking = false;
  bool _autoScrolling = false;
  int _scrollRequest = 0;
  Object? _localError;
  final Map<String, ({JsonMap entry, Object? delivery, Widget widget})> _rows =
      {};
  late final String _visualScope;
  StreamSubscription<dynamic>? _prWatch;
  String? _metadataIdentity;
  JsonMap? _pullRequest;
  bool _canSteer = false;
  Set<String>? _waitingOwnBefore, _queuedBefore;
  String? _anchorId, _tailIdentity;
  GlobalKey? _anchorKey;
  GlobalKey _tailKey = GlobalKey();
  double _runway = 0, _transcriptHeight = 0;
  bool _followScheduled = false;
  late final String _draftKey;
  @override
  void initState() {
    super.initState();
    _draftKey = 'draft.${widget.workspace.cacheScope}.${widget.chatId}';
    _visualScope = '${widget.workspace.cacheScope}:${widget.chatId}';
    _session = widget.workspace.retainSession(widget.chatId);
    widget.preferences.addListener(_preferencesChanged);
    _session.addListener(_changed);
    _text.text = '${widget.preferences.getJson(_draftKey)?['text'] ?? ''}';
    _text.addListener(_edited);
    _focus.addListener(_focusChanged);
    _scroll.addListener(_scrolled);
    unawaited(
      widget.workspace
          .mutate({'op': 'markChatSeen', 'chatId': widget.chatId})
          .catchError((_) {}),
    );
    _changed();
    WidgetsBinding.instance.addPostFrameCallback((_) => _recoverImages());
  }

  Future<void> _recoverImages() async {
    if (!mounted) return;
    await _run(() async {
      final images = await takeRecoveredImages(context);
      if (mounted && images.isNotEmpty) {
        setState(() => _attachments.addAll(images));
      }
    });
  }

  void _preferencesChanged() {
    _rows.clear();
    if (mounted) setState(() {});
  }

  Widget _message(JsonMap entry) {
    final id = '${entry['id']}';
    final previous = _rows[id];
    if (previous != null &&
        identical(previous.entry, entry) &&
        previous.delivery == entry['deliveryStatus']) {
      return previous.widget;
    }
    final row = MessageView(
      key: ValueKey(id),
      session: _session,
      entry: entry,
      preferences: widget.preferences,
    );
    _rows[id] = (entry: entry, delivery: entry['deliveryStatus'], widget: row);
    return row;
  }

  void _focusChanged() {
    if (mounted) setState(() {});
  }

  void _edited() {
    _save?.cancel();
    final draft = _text.text;
    _save = Timer(
      const Duration(milliseconds: 350),
      () =>
          widget.preferences.setJson(_draftKey, {'text': draft}, notify: false),
    );
    _search?.cancel();
    final cursor = _text.selection.baseOffset;
    final prefix = _text.text.substring(0, cursor.clamp(0, _text.text.length));
    final match = RegExp(r'(?:^|\s)@([^\s@]*)$').firstMatch(prefix);
    final generation = ++_searchGeneration;
    if (match == null) {
      if (mounted) setState(() => _mentions = []);
      return;
    }
    _search = Timer(const Duration(milliseconds: 180), () async {
      try {
        final rows = await _session.call('SearchFiles', {
          'query': match[1] ?? '',
        });
        if (mounted && generation == _searchGeneration) {
          setState(
            () => _mentions = jsonMaps(
              rows is List ? rows : jsonMap(rows)['matches'],
            ).take(8).toList(),
          );
        }
      } catch (_) {
        if (mounted && generation == _searchGeneration) {
          setState(() => _mentions = []);
        }
      }
    });
    if (mounted) setState(() {});
  }

  void _scrolled() {
    if (!_scroll.hasClients || _autoScrolling) return;
    _following = _scroll.position.maxScrollExtent - _scroll.offset < 90;
    final jump = !_following;
    if (jump != _jump && mounted) setState(() => _jump = jump);
  }

  void _changed() {
    if (!mounted) return;
    _refreshMetadata();
    final entries = _session.entries;
    final materialized = _session.messages.where(
      (row) => row['role'] == 'user',
    );
    if (_queuedBefore case final before?) {
      final landed = materialized
          .where((row) => !before.contains('${row['id']}'))
          .lastOrNull;
      if (landed != null) {
        _queuedBefore = null;
        _anchorTo('${landed['id']}');
      }
    }
    if (_waitingOwnBefore case final before?) {
      final landed = entries
          .where(
            (row) => row['role'] == 'user' && !before.contains('${row['id']}'),
          )
          .lastOrNull;
      if (landed != null) {
        _waitingOwnBefore = null;
        _anchorTo('${landed['id']}');
      }
    }
    final tailIdentity = _session.working
        ? 'working'
        : '${entries.lastOrNull?['id']}';
    if (tailIdentity != _tailIdentity) {
      _tailIdentity = tailIdentity;
      _tailKey = GlobalKey();
    }
    final next =
        '${entries.length}:${entries.lastOrNull?['id']}:${identityHashCode(entries.lastOrNull)}:${_session.openInput?['requestId']}';
    if (_signature != next && _following) {
      _scheduleFollow();
    }
    _signature = next;
    final ids = entries.map((entry) => '${entry['id']}').toSet();
    _rows.removeWhere((id, _) => !ids.contains(id));
    setState(() {});
  }

  void _anchorTo(String id) {
    _anchorId = id;
    _anchorKey = GlobalKey();
    _runway = 0;
    _following = true;
    _jump = false;
  }

  void _scheduleFollow() {
    if (_followScheduled || !_following) return;
    _followScheduled = true;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      _followScheduled = false;
      if (mounted && _following) unawaited(_bottom(animate: false));
    });
  }

  void _measureRunway() {
    if (_anchorId == null || !_scroll.hasClients) return;
    final anchor = _anchorKey?.currentContext?.findRenderObject();
    final tail = _tailKey.currentContext?.findRenderObject();
    if (anchor is! RenderBox ||
        tail is! RenderBox ||
        !anchor.attached ||
        !tail.attached) {
      return;
    }
    final viewport = RenderAbstractViewport.maybeOf(anchor);
    if (viewport == null) return;
    final top = viewport.getOffsetToReveal(anchor, 0).offset;
    final naturalEnd =
        viewport.getOffsetToReveal(tail, 0).offset + tail.size.height + 32;
    final needed = (top + _scroll.position.viewportDimension - naturalEnd)
        .clamp(0, _scroll.position.viewportDimension)
        .toDouble();
    if ((needed - _runway).abs() > .5 && mounted) {
      setState(() => _runway = needed);
    }
  }

  Widget _transcriptRow(JsonMap entry, bool tail) {
    Widget row = _message(entry);
    if (tail) row = KeyedSubtree(key: _tailKey, child: row);
    if (entry['id'] == _anchorId) {
      row = _AnchorRetention(key: _anchorKey, child: row);
    }
    return SizeChangedLayoutNotifier(child: row);
  }

  void _refreshMetadata() {
    final chat = _session.chat;
    if (chat == null) return;
    final device = chat['deviceId'] ?? widget.workspace.deviceId;
    final cwd = chat['cwd'];
    final branch = chat['branch'];
    final harness = _session.config['harness'] ?? 'native';
    final identity = '$device:$cwd:$branch:$harness';
    if (identity == _metadataIdentity) return;
    _metadataIdentity = identity;
    _pullRequest = null;
    _canSteer = false;
    unawaited(_prWatch?.cancel());
    if (cwd is String && cwd.isNotEmpty) {
      _prWatch = widget.workspace.rpc
          .watch('WatchCheckoutChangeRequest', {
            'targetDeviceId': device,
            'cwd': cwd,
            'branch': ?branch,
          })
          .listen((value) {
            if (!mounted || identity != _metadataIdentity) return;
            setState(
              () => _pullRequest = jsonMap(value)['changeRequest'] == null
                  ? null
                  : jsonMap(jsonMap(value)['changeRequest']),
            );
          }, onError: (Object _) {});
    }
    unawaited(() async {
      try {
        final harnesses = jsonMaps(
          await widget.workspace.call('ListHarnesses', {
            'targetDeviceId': device,
          }),
        );
        if (!mounted || identity != _metadataIdentity) return;
        final row = harnesses.where((row) => row['id'] == harness).firstOrNull;
        setState(
          () => _canSteer =
              row?['supportsSteering'] == true &&
              row?['steeringMode'] == 'step-boundary',
        );
      } catch (_) {
        /* Optional capabilities remain unavailable on older hosts. */
      }
    }());
  }

  Future<void> _pullRequestActions() async {
    final pr = _pullRequest;
    if (pr == null) return;
    final action = await showModalBottomSheet<String>(
      context: context,
      builder: (context) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              leading: const Icon(Icons.open_in_new),
              title: Text(context.tr('打开 Pull Request', 'Open Pull Request')),
              onTap: () => Navigator.pop(context, 'open'),
            ),
            ListTile(
              leading: const Icon(Icons.link),
              title: Text(context.tr('复制链接', 'Copy link')),
              onTap: () => Navigator.pop(context, 'copy'),
            ),
          ],
        ),
      ),
    );
    if (action == 'open') {
      await _run(() async {
        await openExternalLink('${pr['url']}');
      });
    }
    if (action == 'copy') {
      await Clipboard.setData(ClipboardData(text: '${pr['url']}'));
    }
  }

  Future<void> _bottom({bool animate = true}) async {
    if (!_scroll.hasClients || !mounted) return;
    final request = ++_scrollRequest;
    _autoScrolling = true;
    _following = true;
    _measureRunway();
    if (_jump) setState(() => _jump = false);
    if (!animate || MediaQuery.disableAnimationsOf(context)) {
      _scroll.jumpTo(_scroll.position.maxScrollExtent);
    } else {
      await _scroll.animateTo(
        _scroll.position.maxScrollExtent,
        duration: const Duration(milliseconds: 180),
        curve: Curves.easeOut,
      );
    }
    // Lazy rows and decoded images can change the extent during the animation.
    // Settle against the measured tail instead of its earlier estimate.
    for (var pass = 0; pass < 4; pass++) {
      await WidgetsBinding.instance.endOfFrame;
      if (!mounted || request != _scrollRequest || !_scroll.hasClients) return;
      _measureRunway();
      _scroll.jumpTo(_scroll.position.maxScrollExtent);
    }
    if (request == _scrollRequest) _autoScrolling = false;
  }

  Future<void> _run(Future<void> Function() action) async {
    try {
      await action();
      if (mounted) setState(() => _localError = null);
    } catch (error) {
      if (mounted) setState(() => _localError = error);
    }
  }

  Future<void> _send({bool steer = false, bool interrupt = false}) => _run(
    () async {
      final before = _text.text;
      final outgoing = [..._attachments];
      if (before.trim().isEmpty && outgoing.isEmpty) return;
      if (interrupt) await _session.interrupt();
      final users = _session.entries
          .where((row) => row['role'] == 'user')
          .map((row) => '${row['id']}')
          .toSet();
      if (_session.working && !steer && !interrupt) {
        _queuedBefore ??= users;
      } else {
        _queuedBefore = null;
        _waitingOwnBefore = users;
      }
      _following = true;
      final existing = _session.pendingMessages.map((row) => row['id']).toSet();
      try {
        await _session.send(before, attachments: outgoing, steer: steer);
      } catch (_) {
        if (_session.pendingMessages.any(
          (row) => !existing.contains(row['id']),
        )) {
          // An uncertain acknowledgement already has a stable local echo.
          // Its Retry action reuses that command instead of creating a second run.
          await _clearSentDraft(before, outgoing);
        } else {
          _waitingOwnBefore = null;
        }
        rethrow;
      }
      await _clearSentDraft(before, outgoing);
    },
  );

  Future<void> _clearSentDraft(
    String before,
    List<UploadAttachment> outgoing,
  ) async {
    if (!mounted) return;
    _focus.unfocus();
    // A late send acknowledgement cannot erase a newer draft.
    if (_text.text == before) _text.clear();
    for (final attachment in outgoing) {
      _attachments.remove(attachment);
    }
    await widget.preferences.setJson(_draftKey, {
      'text': _text.text,
    }, notify: false);
    _scheduleFollow();
  }

  Future<void> _attach() async {
    final source = await showAttachmentMenu(context);
    if (source == null || !mounted) return;
    setState(() => _picking = true);
    await _run(() async {
      final picked = await chooseImages(context, source);
      if (mounted) setState(() => _attachments.addAll(picked));
    });
    if (mounted) setState(() => _picking = false);
  }

  Future<void> _models() => _run(() async {
    final catalog = jsonMaps(
      await _session.call('ListModels', {
        'harness': _session.config['harness'] ?? 'native',
      }),
    );
    if (!mounted) return;
    final selected = await showModalBottomSheet<JsonMap>(
      context: context,
      isScrollControlled: true,
      builder: (context) => SafeArea(
        child: ConstrainedBox(
          constraints: BoxConstraints(
            maxHeight: MediaQuery.sizeOf(context).height * .7,
          ),
          child: ListView(
            shrinkWrap: true,
            children: [
              for (final model in catalog)
                ListTile(
                  title: Text(
                    '${model['label'] ?? model['name'] ?? model['id']}',
                  ),
                  trailing: model['id'] == _session.config['model']
                      ? Icon(Icons.check, color: context.colors.accent)
                      : null,
                  onTap: () => Navigator.pop(context, model),
                ),
            ],
          ),
        ),
      ),
    );
    if (selected == null || !mounted) return;
    final config = {..._session.config, 'model': selected['id']};
    final levels = (selected['reasoningLevels'] as List? ?? []).cast<String>();
    if (!levels.contains(config['reasoning'])) {
      config['reasoning'] = levels.firstOrNull;
    }
    await _session.setConfig(config);
  });
  Future<void> _effort() => _run(() async {
    final models = jsonMaps(
      await _session.call('ListModels', {
        'harness': _session.config['harness'] ?? 'native',
      }),
    );
    final model = models
        .where((m) => m['id'] == _session.config['model'])
        .firstOrNull;
    final levels = (model?['reasoningLevels'] as List? ?? []).cast<String>();
    if (!mounted || levels.isEmpty) return;
    final effort = await showModalBottomSheet<String>(
      context: context,
      builder: (context) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (final level in levels)
              ListTile(
                title: Text(context.effortLabel(level)),
                trailing: level == _session.config['reasoning']
                    ? const Icon(Icons.check)
                    : null,
                onTap: () => Navigator.pop(context, level),
              ),
          ],
        ),
      ),
    );
    if (effort != null && mounted) {
      await _session.setConfig({..._session.config, 'reasoning': effort});
    }
  });
  void _insertMention(JsonMap match) {
    final path =
        '${match['path'] ?? match['relativePath'] ?? match['name'] ?? ''}';
    final offset = _text.selection.baseOffset.clamp(0, _text.text.length);
    final before = _text.text.substring(0, offset);
    final start = before.lastIndexOf('@');
    if (start < 0) return;
    final next =
        '${_text.text.substring(0, start)}@$path ${_text.text.substring(offset)}';
    _text.value = TextEditingValue(
      text: next,
      selection: TextSelection.collapsed(offset: start + path.length + 2),
    );
    setState(() => _mentions = []);
    _focus.requestFocus();
  }

  @override
  void dispose() {
    _save?.cancel();
    _search?.cancel();
    ++_searchGeneration;
    unawaited(
      widget.preferences.setJson(_draftKey, {
        'text': _text.text,
      }, notify: false),
    );
    _session.removeListener(_changed);
    unawaited(_prWatch?.cancel());
    widget.preferences.removeListener(_preferencesChanged);
    disposeVisualScope(_visualScope);
    widget.workspace.releaseSession(_session);
    _text.dispose();
    _focus.dispose();
    _scroll.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final entries = _session.entries;
    final input = _session.openInput;
    final busy = _session.working;
    final ready = _text.text.trim().isNotEmpty || _attachments.isNotEmpty;
    final error = _localError ?? _session.error;
    final tokens = (_session.contextUsage?['tokens'] as num?)?.toDouble();
    final window = (_session.contextUsage?['window'] as num?)?.toDouble();
    final fraction = tokens != null && window != null && window > 0
        ? tokens / window
        : null;
    return Shortcuts(
      shortcuts: const {
        SingleActivator(LogicalKeyboardKey.enter, meta: true): _SendIntent(),
        SingleActivator(LogicalKeyboardKey.enter, control: true): _SendIntent(),
      },
      child: Actions(
        actions: {
          _SendIntent: CallbackAction<_SendIntent>(
            onInvoke: (_) {
              if (ready && !_session.sending) _send();
              return null;
            },
          ),
        },
        child: Scaffold(
          appBar: AppBar(
            title: Column(
              children: [
                Text(
                  _session.title.isEmpty
                      ? context.tr('新会话', 'New session')
                      : _session.title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
                if (_session.chat?['branch'] != null)
                  Text(
                    '${_session.chat!['branch']}',
                    style: TextStyle(fontSize: 11, color: c.secondary),
                  ),
              ],
            ),
            actions: [
              PopupMenuButton<String>(
                tooltip: context.tr('会话操作', 'Conversation actions'),
                onSelected: (value) {
                  if (value == 'pin') {
                    final pinned =
                        (widget.workspace.sidebarPreferences['pinnedSessionIds']
                                    as List? ??
                                [])
                            .contains(widget.chatId);
                    _run(
                      () => widget.workspace.changeSidebar({
                        'action': pinned ? 'unpin' : 'pin',
                        'sessionId': widget.chatId,
                      }),
                    );
                  }
                  if (value == 'copy') {
                    Clipboard.setData(
                      ClipboardData(
                        text: entries
                            .map(
                              (entry) => entry['role'] == 'user'
                                  ? visiblePrompt(messageText(entry))
                                  : messageText(entry),
                            )
                            .join('\n\n'),
                      ),
                    );
                  }
                  if (value == 'archive') {
                    _run(
                      () => widget.workspace.mutate({
                        'op': 'setChatArchived',
                        'chatId': widget.chatId,
                        'archived': _session.chat?['archived'] != true,
                      }),
                    );
                  }
                  if (value == 'rename') _rename();
                },
                itemBuilder: (_) => [
                  PopupMenuItem(
                    value: 'pin',
                    child: Text(
                      (widget.workspace.sidebarPreferences['pinnedSessionIds']
                                      as List? ??
                                  [])
                              .contains(widget.chatId)
                          ? context.tr('取消置顶', 'Unpin')
                          : context.tr('置顶', 'Pin'),
                    ),
                  ),
                  PopupMenuItem(
                    value: 'copy',
                    child: Text(context.tr('复制对话', 'Copy conversation')),
                  ),
                  PopupMenuItem(
                    value: 'rename',
                    child: Text(context.tr('重命名', 'Rename')),
                  ),
                  PopupMenuItem(
                    value: 'archive',
                    child: Text(
                      _session.chat?['archived'] == true
                          ? context.tr('恢复', 'Restore')
                          : context.tr('归档', 'Archive'),
                    ),
                  ),
                ],
              ),
            ],
          ),
          body: Column(
            children: [
              if (!widget.workspace.connected)
                Material(
                  color: c.warning.withValues(alpha: .1),
                  child: InkWell(
                    onTap: () => _run(widget.workspace.reconnect),
                    child: Padding(
                      padding: const EdgeInsets.symmetric(
                        horizontal: 16,
                        vertical: 8,
                      ),
                      child: Row(
                        children: [
                          Icon(Icons.wifi_off, size: 16, color: c.warning),
                          const SizedBox(width: 8),
                          Expanded(
                            child: Text(
                              context.tr(
                                '连接已断开，正在重新连接',
                                'Disconnected. Reconnecting…',
                              ),
                              style: TextStyle(fontSize: 12, color: c.warning),
                            ),
                          ),
                          const Icon(Icons.refresh, size: 16),
                        ],
                      ),
                    ),
                  ),
                ),
              Expanded(
                child: Stack(
                  children: [
                    if (!_session.hydrated && entries.isEmpty)
                      const Center(child: CircularProgressIndicator())
                    else
                      LayoutBuilder(
                        builder: (context, bounds) {
                          if ((_transcriptHeight - bounds.maxHeight).abs() >
                              .5) {
                            _transcriptHeight = bounds.maxHeight;
                            if (_anchorId != null) _scheduleFollow();
                          }
                          return NotificationListener<
                            SizeChangedLayoutNotification
                          >(
                            onNotification: (_) {
                              _scheduleFollow();
                              return false;
                            },
                            child: NotificationListener<ScrollStartNotification>(
                              onNotification: (notice) {
                                if (notice.dragDetails != null) {
                                  ++_scrollRequest;
                                  _autoScrolling = false;
                                  _scrolled();
                                }
                                return false;
                              },
                              child: ListView.builder(
                                key: PageStorageKey(
                                  'transcript.${widget.workspace.cacheScope}.${widget.chatId}',
                                ),
                                controller: _scroll,
                                padding: EdgeInsets.only(
                                  top: 10,
                                  bottom: 32 + _runway,
                                ),
                                keyboardDismissBehavior:
                                    ScrollViewKeyboardDismissBehavior.onDrag,
                                itemCount: entries.length + (busy ? 1 : 0),
                                itemBuilder: (context, i) => i == entries.length
                                    ? SizeChangedLayoutNotifier(
                                        child: KeyedSubtree(
                                          key: _tailKey,
                                          child: Padding(
                                            padding: const EdgeInsets.fromLTRB(
                                              24,
                                              8,
                                              24,
                                              14,
                                            ),
                                            child: Row(
                                              children: [
                                                SizedBox(
                                                  width: 12,
                                                  height: 12,
                                                  child:
                                                      CircularProgressIndicator(
                                                        strokeWidth: 1.5,
                                                        color: c.secondary,
                                                      ),
                                                ),
                                                const SizedBox(width: 10),
                                                Text(
                                                  context.tr('正在处理', 'Working'),
                                                  style: TextStyle(
                                                    fontSize: 12,
                                                    color: c.secondary,
                                                  ),
                                                ),
                                              ],
                                            ),
                                          ),
                                        ),
                                      )
                                    : _transcriptRow(
                                        entries[i],
                                        !busy && i == entries.length - 1,
                                      ),
                              ),
                            ),
                          );
                        },
                      ),
                    if (_jump)
                      Positioned(
                        right: 20,
                        bottom: 12,
                        child: FloatingActionButton.small(
                          heroTag: 'latest.${widget.chatId}',
                          onPressed: _bottom,
                          tooltip: context.tr('跳到最新', 'Jump to latest'),
                          backgroundColor: c.elevated,
                          foregroundColor: c.text,
                          child: const Icon(Icons.arrow_downward, size: 20),
                        ),
                      ),
                  ],
                ),
              ),
              if (error != null)
                Padding(
                  padding: const EdgeInsets.fromLTRB(18, 4, 18, 4),
                  child: Row(
                    children: [
                      Expanded(
                        child: Text(
                          context.errorText(error),
                          maxLines: 3,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(color: c.danger, fontSize: 12),
                        ),
                      ),
                      IconButton(
                        tooltip: context.tr('关闭', 'Dismiss'),
                        onPressed: () => setState(() {
                          _localError = null;
                          _session.error = null;
                        }),
                        icon: const Icon(Icons.close, size: 16),
                      ),
                    ],
                  ),
                ),
              QueuePanel(session: _session),
              SafeArea(
                top: false,
                minimum: const EdgeInsets.fromLTRB(12, 0, 12, 8),
                child: input != null
                    ? ConstrainedBox(
                        constraints: BoxConstraints(
                          maxHeight: MediaQuery.sizeOf(context).height * .6,
                        ),
                        child: SingleChildScrollView(
                          child: QuestionPanel(
                            key: ValueKey(input['requestId']),
                            input: input,
                            onSubmit: (answers) => _session.respond(
                              '${input['requestId']}',
                              answers,
                            ),
                            onDismiss: () => _session.respond(
                              '${input['requestId']}',
                              [],
                              dismissed: true,
                            ),
                          ),
                        ),
                      )
                    : Container(
                        decoration: BoxDecoration(
                          color: c.elevated.withValues(alpha: .96),
                          borderRadius: BorderRadius.circular(25),
                          border: Border.all(color: c.hairline),
                          boxShadow: [
                            BoxShadow(
                              color: Colors.black.withValues(alpha: .04),
                              blurRadius: 16,
                              offset: const Offset(0, 4),
                            ),
                          ],
                        ),
                        child: Column(
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            if (_mentions.isNotEmpty)
                              ConstrainedBox(
                                constraints: const BoxConstraints(
                                  maxHeight: 200,
                                ),
                                child: ListView(
                                  shrinkWrap: true,
                                  children: [
                                    for (final match in _mentions)
                                      ListTile(
                                        dense: true,
                                        leading: const Icon(
                                          Icons.insert_drive_file_outlined,
                                          size: 16,
                                        ),
                                        title: Text(
                                          '${match['path'] ?? match['relativePath'] ?? match['name']}',
                                          maxLines: 1,
                                          overflow: TextOverflow.ellipsis,
                                        ),
                                        onTap: () => _insertMention(match),
                                      ),
                                  ],
                                ),
                              ),
                            if (_attachments.isNotEmpty)
                              SizedBox(
                                height: 88,
                                child: ListView(
                                  scrollDirection: Axis.horizontal,
                                  padding: const EdgeInsets.all(8),
                                  children: [
                                    for (final image in _attachments)
                                      Padding(
                                        padding: const EdgeInsets.only(
                                          right: 8,
                                        ),
                                        child: Stack(
                                          children: [
                                            ClipRRect(
                                              borderRadius:
                                                  BorderRadius.circular(12),
                                              child: Image.memory(
                                                image.bytes,
                                                width: 80,
                                                height: 72,
                                                fit: BoxFit.cover,
                                              ),
                                            ),
                                            Positioned(
                                              top: 0,
                                              right: 0,
                                              child: IconButton.filledTonal(
                                                visualDensity:
                                                    VisualDensity.compact,
                                                onPressed: _session.sending
                                                    ? null
                                                    : () => setState(
                                                        () => _attachments
                                                            .remove(image),
                                                      ),
                                                tooltip: context.tr(
                                                  '移除图片',
                                                  'Remove image',
                                                ),
                                                icon: const Icon(
                                                  Icons.close,
                                                  size: 14,
                                                ),
                                              ),
                                            ),
                                          ],
                                        ),
                                      ),
                                  ],
                                ),
                              ),
                            if (_session.transferProgress != null)
                              LinearProgressIndicator(
                                value: _session.transferProgress,
                                minHeight: 2,
                              ),
                            if (_focus.hasFocus || _text.text.isNotEmpty)
                              SingleChildScrollView(
                                scrollDirection: Axis.horizontal,
                                padding: const EdgeInsets.fromLTRB(
                                  10,
                                  6,
                                  10,
                                  0,
                                ),
                                child: Row(
                                  children: [
                                    _chip(
                                      '${_session.config['model'] ?? context.tr('模型', 'Model')}',
                                      Icons.tune,
                                      _models,
                                    ),
                                    _chip(
                                      _session.config['reasoning'] == null
                                          ? context.tr('推理强度', 'Effort')
                                          : context.effortLabel(
                                              '${_session.config['reasoning']}',
                                            ),
                                      Icons.psychology_outlined,
                                      _effort,
                                    ),
                                    if (_pullRequest != null)
                                      _chip(
                                        '#${_pullRequest!['number']}',
                                        Icons.merge_type,
                                        _pullRequestActions,
                                      )
                                    else if (_session.chat?['branch'] != null)
                                      _chip(
                                        '${_session.chat!['branch']}',
                                        Icons.account_tree_outlined,
                                        null,
                                      ),
                                    if (fraction != null && fraction >= .5)
                                      Tooltip(
                                        message: '$tokens / $window',
                                        child: Padding(
                                          padding: const EdgeInsets.all(8),
                                          child: Text(
                                            '${(fraction * 100).round()}%',
                                            style: TextStyle(
                                              fontSize: 11,
                                              color: fraction >= .85
                                                  ? c.warning
                                                  : c.secondary,
                                            ),
                                          ),
                                        ),
                                      ),
                                  ],
                                ),
                              ),
                            Row(
                              crossAxisAlignment: CrossAxisAlignment.end,
                              children: [
                                IconButton(
                                  key: const Key('composer-attach'),
                                  tooltip: context.tr('添加图片', 'Add images'),
                                  onPressed: _picking || _session.sending
                                      ? null
                                      : _attach,
                                  icon: const Icon(Icons.add, size: 23),
                                ),
                                Expanded(
                                  child: TextField(
                                    key: const Key('composer-text'),
                                    controller: _text,
                                    focusNode: _focus,
                                    minLines: 1,
                                    maxLines:
                                        MediaQuery.sizeOf(context).height < 500
                                        ? 3
                                        : 8,
                                    textCapitalization:
                                        TextCapitalization.sentences,
                                    keyboardType: TextInputType.multiline,
                                    textInputAction: TextInputAction.newline,
                                    style: const TextStyle(
                                      fontSize: 16,
                                      height: 1.4,
                                    ),
                                    decoration: InputDecoration(
                                      hintText: context.tr('发送消息', 'Message'),
                                      fillColor: Colors.transparent,
                                      filled: false,
                                      border: InputBorder.none,
                                      enabledBorder: InputBorder.none,
                                      focusedBorder: InputBorder.none,
                                      contentPadding:
                                          const EdgeInsets.symmetric(
                                            vertical: 14,
                                            horizontal: 2,
                                          ),
                                    ),
                                  ),
                                ),
                                if (busy && ready)
                                  PopupMenuButton<String>(
                                    tooltip: context.tr('发送方式', 'Send options'),
                                    onSelected: (value) => _send(
                                      steer: value == 'steer',
                                      interrupt: value == 'interrupt',
                                    ),
                                    itemBuilder: (_) => [
                                      PopupMenuItem(
                                        value: 'queue',
                                        child: Text(context.tr('排队', 'Queue')),
                                      ),
                                      if (_canSteer)
                                        PopupMenuItem(
                                          value: 'steer',
                                          child: Text(
                                            context.tr('引导', 'Steer'),
                                          ),
                                        ),
                                      PopupMenuItem(
                                        value: 'interrupt',
                                        child: Text(
                                          context.tr(
                                            '中断并发送',
                                            'Interrupt and send',
                                          ),
                                        ),
                                      ),
                                    ],
                                    icon: const Icon(
                                      Icons.more_horiz,
                                      size: 20,
                                    ),
                                  ),
                                Padding(
                                  padding: const EdgeInsets.all(7),
                                  child: SizedBox(
                                    width: 36,
                                    height: 36,
                                    child: IconButton.filled(
                                      key: const Key('composer-send'),
                                      padding: EdgeInsets.zero,
                                      tooltip: busy && !ready
                                          ? context.tr('停止', 'Stop')
                                          : busy
                                          ? context.tr('排队发送', 'Queue message')
                                          : context.tr('发送', 'Send'),
                                      onPressed:
                                          _session.sending ||
                                              !widget.workspace.connected
                                          ? null
                                          : busy && !ready
                                          ? () => _run(_session.interrupt)
                                          : ready
                                          ? _send
                                          : null,
                                      icon: _session.sending
                                          ? const SizedBox(
                                              width: 16,
                                              height: 16,
                                              child: CircularProgressIndicator(
                                                strokeWidth: 1.5,
                                              ),
                                            )
                                          : Icon(
                                              busy && !ready
                                                  ? Icons.stop_rounded
                                                  : busy
                                                  ? Icons.add
                                                  : Icons.arrow_upward,
                                              size: 21,
                                            ),
                                    ),
                                  ),
                                ),
                              ],
                            ),
                          ],
                        ),
                      ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _chip(String text, IconData icon, VoidCallback? onTap) => Padding(
    padding: const EdgeInsets.only(right: 6),
    child: InkWell(
      onTap: onTap,
      borderRadius: BorderRadius.circular(12),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 6),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(icon, size: 13, color: context.colors.secondary),
            const SizedBox(width: 5),
            ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 150),
              child: Text(
                text,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(fontSize: 11, color: context.colors.secondary),
              ),
            ),
          ],
        ),
      ),
    ),
  );
  Future<void> _rename() async {
    final input = TextEditingController(text: _session.title);
    final title = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(context.tr('重命名', 'Rename')),
        content: TextField(
          controller: input,
          autofocus: true,
          onSubmitted: (value) => Navigator.pop(context, value),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: Text(context.tr('取消', 'Cancel')),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, input.text),
            child: Text(context.tr('保存', 'Save')),
          ),
        ],
      ),
    );
    input.dispose();
    if (title != null && title.trim().isNotEmpty) {
      await _run(
        () => widget.workspace.mutate({
          'op': 'renameChat',
          'chatId': widget.chatId,
          'title': title.trim(),
        }),
      );
    }
  }
}

class _SendIntent extends Intent {
  const _SendIntent();
}

class _AnchorRetention extends StatefulWidget {
  const _AnchorRetention({super.key, required this.child});
  final Widget child;
  @override
  State<_AnchorRetention> createState() => _AnchorRetentionState();
}

class _AnchorRetentionState extends State<_AnchorRetention>
    with AutomaticKeepAliveClientMixin {
  @override
  bool get wantKeepAlive => true;
  @override
  Widget build(BuildContext context) {
    super.build(context);
    return widget.child;
  }
}
