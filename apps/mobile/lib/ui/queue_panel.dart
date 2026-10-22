import 'dart:async';

import 'package:flutter/material.dart';
import 'package:uuid/uuid.dart';

import '../data/workspace_controller.dart';
import '../l10n.dart';
import '../theme.dart';
import 'attachments.dart';
import 'message_text.dart';

class QueuePanel extends StatelessWidget {
  const QueuePanel({super.key, required this.session});
  final SessionController session;
  Future<void> _action(
    BuildContext context,
    JsonMap item,
    String action,
    int index,
  ) async {
    try {
      final id = '${item['id']}';
      if (action == 'edit') {
        await Navigator.push(
          context,
          MaterialPageRoute<void>(
            builder: (_) => QueueEditor(session: session, item: item),
          ),
        );
      } else if (action == 'up' || action == 'down') {
        final current = session.queue.indexWhere((row) => row['id'] == id);
        if (current < 0) return;
        await session.queueAction('MoveQueuedMessage', id, {
          'toIndex': (current + (action == 'up' ? -1 : 1)).clamp(
            0,
            session.queue.length - 1,
          ),
        });
      } else {
        await session.queueAction(action, id);
      }
    } catch (error) {
      if (context.mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text(context.errorText(error))));
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    if (session.queue.isEmpty) return const SizedBox.shrink();
    return Container(
      margin: const EdgeInsets.fromLTRB(12, 4, 12, 6),
      decoration: BoxDecoration(
        color: context.colors.elevated.withValues(alpha: .8),
        borderRadius: BorderRadius.circular(18),
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          for (final (i, item) in session.queue.take(3).indexed)
            ListTile(
              key: ValueKey('queued.${item['id']}'),
              dense: true,
              leading: Icon(
                Icons.schedule,
                size: 17,
                color: context.colors.secondary,
              ),
              subtitle: item['deliveryGate'] == null
                  ? null
                  : Text(
                      jsonMap(item['deliveryGate'])['kind'] == 'editing'
                          ? context.tr('正在编辑', 'Being edited')
                          : context.tr('需要检查后保存', 'Review and save required'),
                    ),
              title: Text(
                visiblePrompt('${item['text'] ?? ''}'),
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
              ),
              trailing: PopupMenuButton<String>(
                tooltip: context.tr('队列操作', 'Queue actions'),
                onSelected: (action) => _action(context, item, action, i),
                itemBuilder: (_) => [
                  PopupMenuItem(
                    value: 'SendQueuedMessageNow',
                    enabled: item['deliveryGate'] == null,
                    child: Text(context.tr('立即发送', 'Send now')),
                  ),
                  if (session.working)
                    PopupMenuItem(
                      value: 'SteerQueuedMessageNow',
                      enabled: item['deliveryGate'] == null,
                      child: Text(context.tr('引导当前任务', 'Steer current turn')),
                    ),
                  PopupMenuItem(
                    value: 'edit',
                    child: Text(context.tr('编辑', 'Edit')),
                  ),
                  if (i > 0)
                    PopupMenuItem(
                      value: 'up',
                      child: Text(context.tr('上移', 'Move up')),
                    ),
                  if (i + 1 < session.queue.length)
                    PopupMenuItem(
                      value: 'down',
                      child: Text(context.tr('下移', 'Move down')),
                    ),
                  PopupMenuItem(
                    value: 'RemoveQueuedMessage',
                    child: Text(context.tr('删除', 'Remove')),
                  ),
                ],
              ),
            ),
          if (session.queue.length > 3)
            TextButton(
              onPressed: () => showModalBottomSheet<void>(
                context: context,
                isScrollControlled: true,
                builder: (context) => DraggableScrollableSheet(
                  expand: false,
                  builder: (_, scroll) => ListenableBuilder(
                    listenable: session,
                    builder: (_, _) => ListView.builder(
                      controller: scroll,
                      itemCount: session.queue.length,
                      itemBuilder: (context, i) {
                        final item = session.queue[i];
                        return ListTile(
                          title: Text(
                            visiblePrompt('${item['text'] ?? ''}'),
                            maxLines: 2,
                            overflow: TextOverflow.ellipsis,
                          ),
                          onTap: () => _action(context, item, 'edit', i),
                          trailing: IconButton(
                            icon: const Icon(Icons.delete_outline),
                            tooltip: context.tr('删除', 'Remove'),
                            onPressed: () => _action(
                              context,
                              item,
                              'RemoveQueuedMessage',
                              i,
                            ),
                          ),
                        );
                      },
                    ),
                  ),
                ),
              ),
              child: Text(
                context.tr(
                  '另外 ${session.queue.length - 3} 条',
                  '${session.queue.length - 3} more queued',
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class QueueEditor extends StatefulWidget {
  const QueueEditor({super.key, required this.session, required this.item});
  final SessionController session;
  final JsonMap item;
  @override
  State<QueueEditor> createState() => _QueueEditorState();
}

class _QueueEditorState extends State<QueueEditor> {
  final _text = TextEditingController();
  String? _lease, _hash, _raw;
  String _visible = '';
  List<String> _attachments = [];
  List<String> _originalAttachments = [];
  Timer? _renew;
  bool _loading = true, _saving = false, _finished = false, _lost = false;
  Object? _error;
  String get _id => '${widget.item['id']}';
  @override
  void initState() {
    super.initState();
    _begin();
  }

  Future<void> _begin() async {
    try {
      final result = jsonMap(
        await widget.session.beginQueueEdit(_id, const Uuid().v4()),
      );
      if (!mounted) {
        if (result['outcome'] == 'acquired') {
          await widget.session.finishQueueEdit(
            _id,
            '${result['leaseId']}',
            'releaseUnchanged',
          );
        }
        return;
      }
      if (result['outcome'] != 'acquired') {
        throw StateError(
          context.tr(
            '此消息正在其他设备编辑或已发送',
            'This message is being edited elsewhere or was already sent',
          ),
        );
      }
      _lease = '${result['leaseId']}';
      _hash = '${result['baseTextHash']}';
      _raw = '${result['text'] ?? ''}';
      _visible = visiblePrompt(_raw!);
      _text.text = _visible;
      _attachments = (result['attachments'] as List? ?? []).cast<String>();
      _originalAttachments = [..._attachments];
      _renew = Timer.periodic(
        const Duration(seconds: 20),
        (_) => _renewLease(),
      );
    } catch (error) {
      _error = error;
    }
    if (mounted) setState(() => _loading = false);
  }

  Future<void> _renewLease() async {
    if (_lease == null || _finished || _lost) return;
    try {
      final result = jsonMap(await widget.session.renewQueueEdit(_id, _lease!));
      if (result['outcome'] != 'renewed') {
        throw StateError('Editing lease expired');
      }
    } catch (error) {
      if (mounted) {
        setState(() {
          _lost = true;
          _error = error;
        });
      }
    }
  }

  bool get _dirty =>
      _text.text != _visible ||
      _attachments.join('\u0000') != _originalAttachments.join('\u0000');
  Future<void> _finish(String action) async {
    if (_saving || _finished) return;
    if (_lease == null) {
      _close();
      return;
    }
    setState(() => _saving = true);
    try {
      final result = jsonMap(
        await widget.session.finishQueueEdit(
          _id,
          _lease!,
          action,
          text: action == 'commit'
              ? editedQueuedPrompt(_raw!, _text.text, _attachments)
              : null,
          attachments: action == 'commit' ? _attachments : null,
          expectedTextHash: _hash,
        ),
      );
      if (!mounted) return;
      if (action != 'commit' &&
          ['lost', 'missing'].contains(result['outcome'])) {
        _renew?.cancel();
        _close();
        return;
      }
      if (![
        'committed',
        'cancelled',
        'discarded',
        'released',
        'missing',
      ].contains(result['outcome'])) {
        throw StateError(
          context.tr(
            '消息已变化，请关闭后重新编辑',
            'The message changed. Close and reopen the editor.',
          ),
        );
      }
      _renew?.cancel();
      _close();
    } catch (error) {
      if (mounted) {
        setState(() {
          _saving = false;
          _error = error;
        });
      }
    }
  }

  void _close() {
    if (!mounted) return;
    setState(() => _finished = true);
    // PopScope must rebuild with canPop before issuing the actual pop.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) Navigator.of(context).pop();
    });
  }

  Future<void> _add() async {
    final source = await showAttachmentMenu(context);
    if (source == null || !mounted) return;
    try {
      final images = await chooseImages(context, source);
      if (!mounted || _lost) return;
      setState(() => _saving = true);
      for (final image in images) {
        final path = await widget.session.upload(image);
        if (!mounted || _lost) return;
        setState(() => _attachments.add(path));
      }
    } catch (error) {
      if (mounted) setState(() => _error = error);
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  void dispose() {
    final dirty = _dirty;
    _renew?.cancel();
    _text.dispose();
    if (!_finished && _lease != null) {
      unawaited(
        widget.session
            .finishQueueEdit(
              _id,
              _lease!,
              dirty ? 'cancel' : 'releaseUnchanged',
            )
            .catchError((_) => null),
      );
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => PopScope(
    canPop: _finished || _lease == null,
    onPopInvokedWithResult: (didPop, _) {
      if (!didPop) _finish(_dirty ? 'cancel' : 'releaseUnchanged');
    },
    child: Scaffold(
      appBar: AppBar(
        title: Text(context.tr('编辑排队消息', 'Edit queued message')),
        actions: [
          TextButton(
            onPressed: _loading || _saving || _lost
                ? null
                : () => _finish('commit'),
            child: Text(context.tr('保存', 'Save')),
          ),
        ],
      ),
      body: _loading
          ? const Center(child: CircularProgressIndicator())
          : Padding(
              padding: const EdgeInsets.all(20),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  if (_error != null)
                    Padding(
                      padding: const EdgeInsets.only(bottom: 12),
                      child: Text(
                        context.errorText(_error),
                        style: TextStyle(color: context.colors.danger),
                      ),
                    ),
                  Expanded(
                    child: TextField(
                      controller: _text,
                      autofocus: true,
                      maxLines: null,
                      expands: true,
                      readOnly: _saving || _lost || _lease == null,
                      textAlignVertical: TextAlignVertical.top,
                    ),
                  ),
                  const SizedBox(height: 12),
                  Wrap(
                    spacing: 6,
                    children: [
                      for (final path in _attachments)
                        InputChip(
                          label: Text(
                            path.split('/').last,
                            overflow: TextOverflow.ellipsis,
                          ),
                          onDeleted: _saving || _lost
                              ? null
                              : () => setState(() => _attachments.remove(path)),
                        ),
                    ],
                  ),
                  Row(
                    children: [
                      IconButton(
                        onPressed: _saving || _lost || _lease == null
                            ? null
                            : _add,
                        tooltip: context.tr('添加图片', 'Add images'),
                        icon: const Icon(Icons.add_photo_alternate_outlined),
                      ),
                      const Spacer(),
                      TextButton(
                        onPressed: _saving || _lease == null
                            ? null
                            : () => _finish('discard'),
                        child: Text(
                          context.tr('删除消息', 'Discard message'),
                          style: TextStyle(color: context.colors.danger),
                        ),
                      ),
                      TextButton(
                        onPressed: _saving
                            ? null
                            : () => _finish(
                                _dirty ? 'cancel' : 'releaseUnchanged',
                              ),
                        child: Text(context.tr('取消', 'Cancel')),
                      ),
                    ],
                  ),
                ],
              ),
            ),
    ),
  );
}
