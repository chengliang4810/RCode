import 'dart:async';

import 'package:flutter/material.dart';

import '../data/workspace_controller.dart';
import '../l10n.dart';
import '../preferences.dart';
import '../theme.dart';
import '../wallpaper.dart';
import 'attachments.dart';
import 'project_browser_page.dart';
import 'sessions_page.dart' show performUiAction;

class NewSessionPage extends StatefulWidget {
  const NewSessionPage({
    super.key,
    required this.workspace,
    required this.preferences,
    this.onCreated,
    this.embedded = false,
    this.initialText,
  });
  final WorkspaceController workspace;
  final AppPreferences preferences;
  final ValueChanged<String>? onCreated;
  final bool embedded;
  final String? initialText;

  @override
  State<NewSessionPage> createState() => _NewSessionPageState();
}

class _NewSessionPageState extends State<NewSessionPage> {
  late final TextEditingController _text;
  String? _projectId;
  String? _deviceId;
  String _harness = 'native';
  String? _model;
  String? _reasoning;
  String? _branch;
  bool _worktree = false;
  bool _loadingModels = false;
  bool _recoveringImages = true;
  bool _sending = false;
  bool _created = false;
  String? _pendingChatId;
  String? _pendingMessageId;
  SessionController? _pendingSession;
  Map<String, dynamic>? _pendingCreate;
  Map<String, dynamic>? _pendingWorktree;
  String? _pendingText;
  List<UploadAttachment>? _pendingAttachments;
  Object? _error;
  int _modelGeneration = 0;
  Timer? _draftTimer;
  List<Map<String, dynamic>> _models = [];
  final List<UploadAttachment> _attachments = [];
  static final Map<String, List<Map<String, dynamic>>> _modelCache = {};

  String get _draftKey => 'newSession.${widget.workspace.cacheScope}';
  Map<String, dynamic>? get _project => widget.workspace.spaces
      .where((space) => space['id'] == _projectId)
      .firstOrNull;
  String? get _target => _project?['deviceId'] as String? ?? _deviceId;
  Map<String, dynamic>? get _chosenModel => _models
      .where((model) => model['harness'] == _harness && model['id'] == _model)
      .firstOrNull;
  List<String> get _efforts => (_chosenModel?['reasoningLevels'] as List? ?? [])
      .whereType<String>()
      .toList();
  bool get _locked => _sending || _pendingChatId != null;

  @override
  void initState() {
    super.initState();
    final draft = widget.preferences.getJson(_draftKey) ?? {};
    _projectId = draft['projectId'] as String?;
    _deviceId =
        draft['deviceId'] as String? ??
        (widget.workspace.deviceId.isEmpty ? null : widget.workspace.deviceId);
    _harness = draft['harness'] as String? ?? 'native';
    _model = draft['model'] as String?;
    _reasoning = draft['reasoning'] as String?;
    _branch = draft['branch'] as String?;
    _worktree = draft['worktree'] == true;
    _text = TextEditingController(
      text: widget.initialText ?? draft['text'] as String? ?? '',
    );
    _text.addListener(_remember);
    _loadModels();
    WidgetsBinding.instance.addPostFrameCallback((_) => _restoreImages());
  }

  Future<void> _restoreImages() async {
    try {
      final images = await takeRecoveredImages(context);
      if (!mounted) return;
      if (images.isNotEmpty) {
        setState(() => _attachments.addAll(images));
        _remember();
      }
    } catch (error) {
      if (mounted) setState(() => _error = error);
    } finally {
      if (mounted) setState(() => _recoveringImages = false);
    }
  }

  void _remember() {
    if (_created) return;
    _draftTimer?.cancel();
    _draftTimer = Timer(const Duration(milliseconds: 250), _persistDraft);
  }

  void _persistDraft() {
    if (_created) return;
    unawaited(
      widget.preferences.setJson(_draftKey, {
        'projectId': _projectId,
        'deviceId': _deviceId,
        'harness': _harness,
        'model': _model,
        'reasoning': _reasoning,
        'branch': _branch,
        'worktree': _worktree,
        'text': _text.text,
      }, notify: false),
    );
  }

  @override
  void dispose() {
    _modelGeneration++;
    _draftTimer?.cancel();
    _persistDraft();
    _text.removeListener(_remember);
    _text.dispose();
    if (_pendingSession case final session?) {
      widget.workspace.releaseSession(session);
    }
    super.dispose();
  }

  Future<void> _loadModels() async {
    final device = _target;
    final generation = ++_modelGeneration;
    final key = '${widget.workspace.cacheScope}:$device';
    if (device == null) return;
    setState(() {
      _models = _modelCache[key] ?? [];
      _applyCatalog();
      _loadingModels = true;
      _error = null;
    });
    try {
      final harnesses = jsonRows(
        await widget.workspace.call('ListHarnesses', {
          'targetDeviceId': device,
        }),
      );
      final models = <Map<String, dynamic>>[];
      for (final harness in harnesses) {
        if (harness['enabled'] == false || harness['installed'] == false) {
          continue;
        }
        final listed = jsonRows(
          await widget.workspace.call('ListModels', {
            'targetDeviceId': device,
            'harness': harness['id'],
          }),
        );
        models.addAll(
          listed.map(
            (model) => {
              ...model,
              'harness': harness['id'],
              'harnessLabel': harness['name'],
            },
          ),
        );
      }
      _modelCache[key] = models;
      if (!mounted || generation != _modelGeneration || device != _target) {
        return;
      }
      setState(() {
        _models = models;
        if (models.isEmpty) {
          _model = null;
          _reasoning = null;
        } else {
          _applyCatalog();
        }
      });
      _remember();
    } catch (error) {
      if (mounted && generation == _modelGeneration) {
        setState(() => _error = error);
      }
    } finally {
      if (mounted && generation == _modelGeneration) {
        setState(() => _loadingModels = false);
      }
    }
  }

  void _applyCatalog() {
    if (_models.isEmpty) return;
    if (!_models.any((model) => model['harness'] == _harness)) {
      _harness = _models.first['harness'] as String;
    }
    final matching = _models
        .where((model) => model['harness'] == _harness)
        .toList();
    if (!matching.any((model) => model['id'] == _model)) {
      _model = matching.first['id'] as String;
      _reasoning = null;
    }
    final efforts = _efforts;
    if (efforts.isEmpty) {
      _reasoning = null;
    } else if (!efforts.contains(_reasoning)) {
      _reasoning = efforts[efforts.length ~/ 2];
    }
  }

  Future<T?> _choose<T>({required String title, required List<Widget> items}) =>
      showModalBottomSheet<T>(
        context: context,
        showDragHandle: true,
        isScrollControlled: true,
        builder: (context) => SafeArea(
          child: ConstrainedBox(
            constraints: BoxConstraints(
              maxHeight: MediaQuery.sizeOf(context).height * .75,
            ),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Padding(
                  padding: const EdgeInsets.fromLTRB(20, 0, 20, 8),
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: Text(
                      title,
                      style: const TextStyle(
                        fontSize: 20,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ),
                ),
                Flexible(child: ListView(shrinkWrap: true, children: items)),
              ],
            ),
          ),
        ),
      );

  Future<void> _pickProject() async {
    final groups = <String, List<Map<String, dynamic>>>{};
    for (final space in widget.workspace.spaces) {
      final key = (space['repositoryId'] ?? space['path'] ?? space['id'])
          .toString();
      groups.putIfAbsent(key, () => []).add(space);
    }
    final choice = await _choose<String>(
      title: context.tr('项目', 'Project'),
      items: [
        for (final group in groups.values)
          ListTile(
            leading: const Icon(Icons.folder_outlined),
            title: Text(
              group.first['name'] as String? ??
                  fileName(group.first['path'] as String? ?? ''),
            ),
            subtitle: Text(
              group
                  .map((space) => _deviceName(space['deviceId'] as String))
                  .toSet()
                  .join(', '),
            ),
            trailing: group.any((space) => space['id'] == _projectId)
                ? const Icon(Icons.check)
                : null,
            onTap: () {
              final chosen =
                  group
                      .where((space) => space['id'] == _projectId)
                      .firstOrNull ??
                  group
                      .where((space) => space['deviceId'] == _target)
                      .firstOrNull ??
                  group.first;
              Navigator.pop(context, chosen['id'] as String);
            },
          ),
        ListTile(
          leading: const Icon(Icons.inbox_outlined),
          title: Text(context.tr('无项目', 'No Project')),
          onTap: () => Navigator.pop(context, 'none'),
        ),
        ListTile(
          leading: const Icon(Icons.create_new_folder_outlined),
          title: Text(context.tr('新建项目', 'New Project')),
          onTap: () => Navigator.pop(context, 'new'),
        ),
      ],
    );
    if (choice == null || !mounted) return;
    String? project = choice == 'none' ? null : choice;
    if (choice == 'new') {
      project = await Navigator.of(context).push<String>(
        MaterialPageRoute(
          builder: (_) => ProjectBrowserPage(
            workspace: widget.workspace,
            initialDeviceId: _target,
          ),
        ),
      );
      if (project == null || !mounted) return;
    }
    setState(() {
      _projectId = project;
      _branch = null;
      _worktree = false;
    });
    _remember();
    await _loadModels();
  }

  String _deviceName(String id) =>
      widget.workspace.devices
              .where((device) => device['id'] == id)
              .firstOrNull?['name']
          as String? ??
      id;

  Future<void> _pickHost() async {
    final project = _project;
    final spaces = project == null
        ? <Map<String, dynamic>>[]
        : widget.workspace.spaces
              .where(
                (space) =>
                    (space['repositoryId'] ?? space['path']) ==
                    (project['repositoryId'] ?? project['path']),
              )
              .toList();
    final choice = await _choose<String>(
      title: context.tr('执行设备', 'Run on'),
      items: spaces.isNotEmpty
          ? spaces
                .map(
                  (space) => ListTile(
                    leading: const Icon(Icons.computer_outlined),
                    title: Text(_deviceName(space['deviceId'] as String)),
                    subtitle: Text(space['path'] as String? ?? ''),
                    trailing: space['id'] == _projectId
                        ? const Icon(Icons.check)
                        : null,
                    onTap: () => Navigator.pop(context, space['id'] as String),
                  ),
                )
                .toList()
          : widget.workspace.devices
                .where(
                  (device) => ![
                    'ios',
                    'ipados',
                    'android',
                  ].contains(device['platform']),
                )
                .map(
                  (device) => ListTile(
                    leading: const Icon(Icons.computer_outlined),
                    title: Text(
                      device['name'] as String? ?? device['id'] as String,
                    ),
                    trailing: device['id'] == _deviceId
                        ? const Icon(Icons.check)
                        : null,
                    onTap: () => Navigator.pop(context, device['id'] as String),
                  ),
                )
                .toList(),
    );
    if (choice == null || !mounted) return;
    setState(() {
      if (spaces.isNotEmpty) {
        _projectId = choice;
        _branch = null;
      } else {
        _deviceId = choice;
      }
    });
    _remember();
    await _loadModels();
  }

  Future<void> _pickBranch() async {
    final project = _project;
    if (project == null) return;
    await performUiAction(context, () async {
      final device = _target;
      final refs = jsonRows(
        await widget.workspace.call('ListRefs', {
          'repoPath': project['path'],
          'targetDeviceId': device,
        }),
      );
      if (!mounted || device != _target) return;
      final choice = await _choose<String>(
        title: context.tr('检出', 'Checkout'),
        items: [
          ListTile(
            leading: const Icon(Icons.account_tree_outlined),
            title: Text(context.tr('新建工作树', 'New worktree')),
            trailing: _worktree ? const Icon(Icons.check) : null,
            onTap: () => Navigator.pop(context, ':worktree'),
          ),
          ListTile(
            leading: const Icon(Icons.folder_outlined),
            title: Text(context.tr('使用现有目录', 'Use existing checkout')),
            trailing: !_worktree ? const Icon(Icons.check) : null,
            onTap: () => Navigator.pop(context, ':existing'),
          ),
          for (final ref in refs)
            ListTile(
              leading: const Icon(Icons.call_split),
              title: Text(ref['name'] as String),
              subtitle: ref['current'] == true
                  ? Text(context.tr('当前分支', 'Current branch'))
                  : null,
              trailing: ref['name'] == _branch ? const Icon(Icons.check) : null,
              onTap: () => Navigator.pop(context, ref['name'] as String),
            ),
        ],
      );
      if (choice == null || !mounted) return;
      setState(() {
        if (choice == ':worktree') {
          _worktree = true;
        } else if (choice == ':existing') {
          _worktree = false;
        } else {
          _branch = choice;
        }
      });
      _remember();
    });
  }

  Future<void> _pickModel() async {
    final choice = await _choose<int>(
      title: context.tr('模型', 'Model'),
      items: [
        for (var i = 0; i < _models.length; i++)
          ListTile(
            title: Text(
              _models[i]['label'] as String? ?? _models[i]['id'] as String,
            ),
            subtitle: _models[i]['description'] == null
                ? null
                : Text(_models[i]['description'] as String),
            trailing:
                _models[i]['id'] == _model && _models[i]['harness'] == _harness
                ? const Icon(Icons.check)
                : null,
            onTap: () => Navigator.pop(context, i),
          ),
      ],
    );
    if (choice == null || !mounted) return;
    setState(() {
      _model = _models[choice]['id'] as String;
      _harness = _models[choice]['harness'] as String;
      _reasoning = null;
      _applyCatalog();
    });
    _remember();
  }

  Future<void> _pickEffort() async {
    final choice = await _choose<String>(
      title: context.tr('推理强度', 'Reasoning effort'),
      items: [
        for (final effort in _efforts)
          ListTile(
            title: Text(context.effortLabel(effort)),
            trailing: effort == _reasoning ? const Icon(Icons.check) : null,
            onTap: () => Navigator.pop(context, effort),
          ),
      ],
    );
    if (choice == null || !mounted) return;
    setState(() => _reasoning = choice);
    _remember();
  }

  Future<void> _pickAttachments() async {
    final source = await showAttachmentMenu(context);
    if (source == null || !mounted) return;
    await performUiAction(context, () async {
      final picked = await chooseImages(context, source);
      if (mounted) setState(() => _attachments.addAll(picked));
    });
  }

  Future<void> _create() async {
    if (_sending ||
        _recoveringImages ||
        (_text.text.trim().isEmpty && _attachments.isEmpty)) {
      return;
    }
    final target = _target;
    if (_pendingChatId == null && (target == null || _chosenModel == null)) {
      setState(
        () => _error = context.tr(
          '请选择执行设备和模型',
          'Choose an execution device and model',
        ),
      );
      return;
    }
    setState(() {
      _sending = true;
      _error = null;
    });
    final id = _pendingChatId ??= mobileId('chat');
    try {
      final project = _project;
      _pendingCreate ??= {
        'op': 'createChat',
        'chatId': id,
        if (project != null) 'spaceId': project['id'] else 'deviceId': target,
        if (_branch != null) 'branch': _branch,
        'config': {
          'harness': _harness,
          'model': _model,
          'reasoning': _reasoning,
          'modelOptions': <String, dynamic>{},
        },
      };
      _pendingText ??= _text.text.trim();
      _pendingAttachments ??= List.of(_attachments);
      _pendingWorktree ??= _worktree && project != null
          ? {
              'repoPath': project['path'],
              'base': _branch ?? 'HEAD',
              'spaceId': project['id'],
            }
          : null;
      // CreateChat is idempotent: an acknowledgement lost during reconnect
      // retries the same chat instead of leaving a second empty conversation.
      if (widget.workspace.chat(id) == null) {
        await widget.workspace.call('Mutate', _pendingCreate!);
      }
      final session = _pendingSession ??= widget.workspace.retainSession(id);
      if (_pendingMessageId case final messageId?) {
        if (!session.messages.any((entry) => entry['id'] == messageId)) {
          await session.retryPending(messageId);
        }
      } else {
        _pendingMessageId = await session.send(
          _pendingText!,
          attachments: _pendingAttachments!,
          worktree: _pendingWorktree,
        );
      }
      _created = true;
      await widget.preferences.setJson(_draftKey, {
        'projectId': _projectId,
        'deviceId': _deviceId,
        'harness': _harness,
        'model': _model,
        'reasoning': _reasoning,
        'branch': _branch,
        'worktree': _worktree,
        'text': '',
      }, notify: false);
      if (!mounted) return;
      if (widget.onCreated != null) {
        widget.onCreated!(id);
      } else {
        Navigator.of(context).pop(id);
      }
    } catch (error) {
      _pendingMessageId ??=
          _pendingSession?.pendingMessages.lastOrNull?['id'] as String?;
      if (mounted) setState(() => _error = error);
    } finally {
      if (mounted) setState(() => _sending = false);
    }
  }

  Widget _chip(IconData icon, String label, VoidCallback? action, String id) =>
      ActionChip(
        key: ValueKey('new-$id'),
        avatar: Icon(icon, size: 14),
        label: Text(label, overflow: TextOverflow.ellipsis),
        onPressed: _locked ? null : action,
        padding: const EdgeInsets.symmetric(horizontal: 3),
        visualDensity: VisualDensity.compact,
        side: BorderSide.none,
      );

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final project = _project;
    return Scaffold(
      backgroundColor: colors.background,
      appBar: widget.embedded
          ? null
          : AppBar(
              title: Text(context.tr('新会话', 'New Session')),
              leading: IconButton(
                tooltip: context.tr('关闭', 'Close'),
                icon: const Icon(Icons.close),
                onPressed: _sending ? null : () => Navigator.of(context).pop(),
              ),
            ),
      body: WallpaperBackdrop(
        preferences: widget.preferences,
        child: SafeArea(
          child: LayoutBuilder(
            builder: (context, constraints) => SingleChildScrollView(
              padding: const EdgeInsets.fromLTRB(20, 20, 20, 20),
              child: ConstrainedBox(
                constraints: BoxConstraints(
                  minHeight: (constraints.maxHeight - 40).clamp(
                    0,
                    double.infinity,
                  ),
                ),
                child: IntrinsicHeight(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      const Spacer(flex: 3),
                      Icon(
                        Icons.auto_awesome_outlined,
                        color: colors.text,
                        size: 38,
                      ),
                      const SizedBox(height: 24),
                      Container(
                        padding: const EdgeInsets.fromLTRB(14, 12, 14, 8),
                        decoration: BoxDecoration(
                          color: colors.elevated.withValues(alpha: .94),
                          borderRadius: BorderRadius.circular(26),
                          border: Border.all(
                            color: colors.hairline.withValues(alpha: .8),
                          ),
                          boxShadow: [
                            BoxShadow(
                              color: Colors.black.withValues(alpha: .04),
                              blurRadius: 22,
                              offset: const Offset(0, 6),
                            ),
                          ],
                        ),
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.stretch,
                          children: [
                            Wrap(
                              spacing: 4,
                              runSpacing: 0,
                              children: [
                                _chip(
                                  Icons.folder_outlined,
                                  project?['name'] as String? ??
                                      (project == null
                                          ? context.tr('无项目', 'No project')
                                          : fileName(
                                              project['path'] as String? ?? '',
                                            )),
                                  _pickProject,
                                  'project',
                                ),
                                _chip(
                                  Icons.computer_outlined,
                                  _target == null
                                      ? context.tr('选择设备', 'Choose host')
                                      : _deviceName(_target!),
                                  _pickHost,
                                  'device',
                                ),
                                if (project?['gitDetected'] == true)
                                  _chip(
                                    Icons.call_split,
                                    _worktree
                                        ? context.tr('新工作树', 'New worktree')
                                        : _branch ??
                                              context.tr(
                                                '当前分支',
                                                'Current branch',
                                              ),
                                    _pickBranch,
                                    'branch',
                                  ),
                                _chip(
                                  Icons.auto_awesome_outlined,
                                  _chosenModel?['label'] as String? ??
                                      _model ??
                                      context.tr('选择模型', 'Choose model'),
                                  _models.isEmpty ? null : _pickModel,
                                  'model',
                                ),
                                if (_efforts.isNotEmpty)
                                  _chip(
                                    Icons.speed_outlined,
                                    context.effortLabel(_reasoning ?? ''),
                                    _pickEffort,
                                    'effort',
                                  ),
                              ],
                            ),
                            if (_loadingModels)
                              const LinearProgressIndicator(minHeight: 1),
                            TextField(
                              key: const Key('new-message'),
                              controller: _text,
                              readOnly: _locked,
                              minLines: 3,
                              maxLines: 9,
                              style: const TextStyle(
                                fontSize: 17,
                                height: 1.45,
                              ),
                              decoration: InputDecoration(
                                hintText: context.tr('想做点什么？', 'Ask anything'),
                                filled: false,
                                border: InputBorder.none,
                                enabledBorder: InputBorder.none,
                                focusedBorder: InputBorder.none,
                              ),
                            ),
                            if (_attachments.isNotEmpty)
                              Wrap(
                                spacing: 6,
                                children: [
                                  for (var i = 0; i < _attachments.length; i++)
                                    InputChip(
                                      label: Text(_attachments[i].name),
                                      onDeleted: _locked
                                          ? null
                                          : () => setState(
                                              () => _attachments.removeAt(i),
                                            ),
                                    ),
                                ],
                              ),
                            Row(
                              children: [
                                IconButton(
                                  key: const Key('new-attach'),
                                  tooltip: context.tr('添加附件', 'Add attachment'),
                                  onPressed: _locked ? null : _pickAttachments,
                                  icon: const Icon(Icons.add),
                                ),
                                const Spacer(),
                                if (_sending)
                                  const Padding(
                                    padding: EdgeInsets.only(right: 12),
                                    child: SizedBox(
                                      width: 16,
                                      height: 16,
                                      child: CircularProgressIndicator(
                                        strokeWidth: 2,
                                      ),
                                    ),
                                  ),
                                IconButton.filled(
                                  key: const Key('new-send'),
                                  tooltip: _pendingChatId == null
                                      ? context.tr('发送', 'Send')
                                      : context.tr('重试', 'Retry'),
                                  style: IconButton.styleFrom(
                                    backgroundColor: colors.text,
                                    foregroundColor: colors.background,
                                  ),
                                  onPressed:
                                      _sending ||
                                          _recoveringImages ||
                                          !widget.workspace.connected ||
                                          (_pendingChatId == null &&
                                              (_loadingModels ||
                                                  _chosenModel == null))
                                      ? null
                                      : _create,
                                  icon: Icon(
                                    _pendingChatId == null
                                        ? Icons.arrow_upward
                                        : Icons.refresh,
                                  ),
                                ),
                              ],
                            ),
                          ],
                        ),
                      ),
                      if (_error != null)
                        Padding(
                          padding: const EdgeInsets.all(12),
                          child: Text(
                            context.errorText(_error),
                            style: TextStyle(color: colors.danger),
                            textAlign: TextAlign.center,
                          ),
                        ),
                      if (!_loadingModels &&
                          _models.isEmpty &&
                          _target != null &&
                          _error == null)
                        Padding(
                          padding: const EdgeInsets.all(12),
                          child: Row(
                            mainAxisAlignment: MainAxisAlignment.center,
                            children: [
                              Flexible(
                                child: Text(
                                  context.tr(
                                    '此设备尚未配置模型',
                                    'No models configured on this device',
                                  ),
                                  style: TextStyle(color: colors.secondary),
                                ),
                              ),
                              IconButton(
                                tooltip: context.tr('刷新', 'Refresh'),
                                onPressed: _loadModels,
                                icon: const Icon(Icons.refresh, size: 18),
                              ),
                            ],
                          ),
                        ),
                      const Spacer(flex: 4),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}
