import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:share_plus/share_plus.dart';

import '../data/workspace_controller.dart';
import '../l10n.dart';
import '../preferences.dart';
import '../theme.dart';
import '../visual/rcode_visual.dart';
import 'diff_view.dart';
import 'message_text.dart';
import 'transcript_markdown.dart';

class MessageView extends StatelessWidget {
  const MessageView({
    super.key,
    required this.session,
    required this.entry,
    required this.preferences,
  });
  final SessionController session;
  final JsonMap entry;
  final AppPreferences preferences;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final parts = jsonMaps(entry['parts']);
    final user = entry['role'] == 'user';
    final body = messageText(entry);
    final images = <String>{
      ...promptImages(body),
      for (final part in parts)
        if (part['kind'] == 'image' && part['path'] is String)
          part['path'] as String,
    };
    final content = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        if (user) ...[
          if (visiblePrompt(body).isNotEmpty)
            SelectableText(
              visiblePrompt(body),
              style: const TextStyle(fontSize: 16, height: 1.5),
            ),
          if (images.isNotEmpty)
            Wrap(
              spacing: 8,
              runSpacing: 8,
              children: [
                for (final path in images)
                  RemoteImage(session: session, path: path),
              ],
            ),
        ] else
          for (final part in _groupTools(parts))
            switch (part['kind']) {
              'toolGroup' => _ToolGroup(
                key: ValueKey(part['id']),
                session: session,
                parts: jsonMaps(part['parts']),
                preferences: preferences,
              ),
              'text' => Padding(
                padding: const EdgeInsets.only(bottom: 8),
                child: TranscriptMarkdown(
                  text: '${part['text'] ?? ''}',
                  preferences: preferences,
                  imageBuilder: (uri, _, _) =>
                      RemoteImage(session: session, path: uri.toString()),
                ),
              ),
              'reasoning' => _Reasoning(
                text: '${part['text'] ?? ''}',
                preferences: preferences,
              ),
              'tool' => ToolPartView(
                key: ValueKey(part['id']),
                session: session,
                part: part,
                preferences: preferences,
              ),
              'image' => Align(
                alignment: Alignment.centerLeft,
                child: RemoteImage(
                  session: session,
                  path: '${part['path']}',
                  name: '${part['name'] ?? ''}',
                ),
              ),
              'visual' => _RemoteVisual(
                key: ValueKey(
                  '${session.workspace.cacheScope}:${part['path']}',
                ),
                session: session,
                part: part,
              ),
              'error' => Padding(
                padding: const EdgeInsets.symmetric(vertical: 8),
                child: SelectableText(
                  '${part['message'] ?? ''}',
                  style: TextStyle(color: c.danger),
                ),
              ),
              'input' => Padding(
                padding: const EdgeInsets.symmetric(vertical: 8),
                child: Row(
                  children: [
                    Icon(
                      part['resolved'] == true
                          ? Icons.check_circle_outline
                          : Icons.help_outline,
                      size: 16,
                      color: part['resolved'] == true ? c.secondary : c.accent,
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: Text(
                        jsonMaps(jsonMap(part['request'])['questions'])
                            .map((q) => q['question'])
                            .join(' · '),
                        style: TextStyle(color: c.secondary, fontSize: 13),
                      ),
                    ),
                  ],
                ),
              ),
              'fork' => Padding(
                padding: const EdgeInsets.symmetric(vertical: 12),
                child: Text(
                  context.tr(
                    '从 ${part['sourceTitle'] ?? ''} 分支',
                    'Forked from ${part['sourceTitle'] ?? ''}',
                  ),
                  style: TextStyle(color: c.secondary, fontSize: 12),
                ),
              ),
              _ => const SizedBox.shrink(),
            },
        if (entry['deliveryStatus'] == 'failed' ||
            entry['deliveryStatus'] == 'rejected' ||
            entry['deliveryStatus'] == 'unconfirmed')
          Row(
            children: [
              Expanded(
                child: Text(
                  '${entry['deliveryError'] ?? (entry['deliveryStatus'] == 'unconfirmed' ? context.tr('发送结果尚未确认', 'Delivery is unconfirmed') : context.tr('未送达', 'Not delivered'))}',
                  style: TextStyle(color: c.danger, fontSize: 12),
                ),
              ),
              TextButton(
                onPressed: !session.connected
                    ? null
                    : () async {
                        try {
                          await session.retryPending('${entry['id']}');
                        } catch (error) {
                          if (context.mounted) {
                            ScaffoldMessenger.of(context).showSnackBar(
                              SnackBar(content: Text(context.errorText(error))),
                            );
                          }
                        }
                      },
                child: Text(context.tr('重试', 'Retry')),
              ),
            ],
          ),
        if (!user && (body.isNotEmpty || entry['status'] == 'complete'))
          Row(
            children: [
              if (entry['status'] == 'streaming')
                Padding(
                  padding: const EdgeInsets.only(right: 8),
                  child: SizedBox(
                    width: 10,
                    height: 10,
                    child: CircularProgressIndicator(
                      strokeWidth: 1.5,
                      color: c.secondary,
                    ),
                  ),
                ),
              if (entry['durationMs'] is num)
                Text(
                  '${((entry['durationMs'] as num) / 1000).toStringAsFixed(1)}s',
                  style: TextStyle(fontSize: 11, color: c.tertiary),
                ),
              const Spacer(),
              IconButton(
                visualDensity: VisualDensity.compact,
                tooltip: context.tr('复制消息', 'Copy message'),
                icon: Icon(Icons.copy_outlined, size: 16, color: c.tertiary),
                onPressed: () => Clipboard.setData(ClipboardData(text: body)),
              ),
            ],
          ),
      ],
    );
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 10, horizontal: 20),
      child: Align(
        alignment: user ? Alignment.centerRight : Alignment.centerLeft,
        child: ConstrainedBox(
          constraints: BoxConstraints(
            maxWidth: user
                ? MediaQuery.sizeOf(context).width.clamp(280, 760) * .86
                : 820,
          ),
          child: user
              ? Container(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 16,
                    vertical: 12,
                  ),
                  decoration: BoxDecoration(
                    color: c.userBubble,
                    borderRadius: BorderRadius.circular(22),
                  ),
                  child: content,
                )
              : content,
        ),
      ),
    );
  }
}

List<JsonMap> _groupTools(List<JsonMap> parts) {
  final result = <JsonMap>[];
  var index = 0;
  while (index < parts.length) {
    if (parts[index]['kind'] != 'tool') {
      result.add(parts[index++]);
      continue;
    }
    final tools = <JsonMap>[];
    while (index < parts.length && parts[index]['kind'] == 'tool') {
      tools.add(parts[index++]);
    }
    result.add(
      tools.length == 1
          ? tools.single
          : {'kind': 'toolGroup', 'id': tools.first['id'], 'parts': tools},
    );
  }
  return result;
}

class _ToolGroup extends StatelessWidget {
  const _ToolGroup({
    super.key,
    required this.session,
    required this.parts,
    required this.preferences,
  });
  final SessionController session;
  final List<JsonMap> parts;
  final AppPreferences preferences;
  @override
  Widget build(BuildContext context) => ExpansionTile(
    tilePadding: EdgeInsets.zero,
    dense: true,
    title: Text(
      context.tr('${parts.length} 个工具调用', '${parts.length} tool calls'),
      style: TextStyle(fontSize: 13, color: context.colors.secondary),
    ),
    children: [
      for (final part in parts)
        ToolPartView(
          key: ValueKey(part['id']),
          session: session,
          part: part,
          preferences: preferences,
        ),
    ],
  );
}

class _Reasoning extends StatelessWidget {
  const _Reasoning({required this.text, required this.preferences});
  final String text;
  final AppPreferences preferences;
  @override
  Widget build(BuildContext context) => ExpansionTile(
    tilePadding: EdgeInsets.zero,
    childrenPadding: const EdgeInsets.only(bottom: 10),
    dense: true,
    visualDensity: VisualDensity.compact,
    title: Text(
      context.tr('思考', 'Thinking'),
      style: TextStyle(color: context.colors.secondary, fontSize: 13),
    ),
    children: [TranscriptMarkdown(text: text, preferences: preferences)],
  );
}

class RemoteImage extends StatefulWidget {
  const RemoteImage({
    super.key,
    required this.session,
    required this.path,
    this.name = '',
    this.share,
  });
  final SessionController session;
  final String path, name;
  @visibleForTesting
  final Future<ShareResult> Function(ShareParams)? share;
  @override
  State<RemoteImage> createState() => _RemoteImageState();
}

class _RemoteImageState extends State<RemoteImage> {
  late Future<Uint8List> _bytes;
  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void didUpdateWidget(RemoteImage old) {
    super.didUpdateWidget(old);
    if (old.path != widget.path || old.session != widget.session) _load();
  }

  void _load() => _bytes = widget.session.readAttachment(widget.path);
  @override
  Widget build(BuildContext context) => FutureBuilder<Uint8List>(
    future: _bytes,
    builder: (context, snapshot) {
      if (snapshot.hasError) {
        return IconButton(
          tooltip: context.tr('重新加载图片', 'Reload image'),
          icon: const Icon(Icons.broken_image_outlined),
          onPressed: () => setState(_load),
        );
      }
      if (!snapshot.hasData) {
        return const SizedBox(
          width: 120,
          height: 100,
          child: Center(child: CircularProgressIndicator(strokeWidth: 2)),
        );
      }
      return Semantics(
        label: widget.name.isEmpty
            ? context.tr('图片附件', 'Image attachment')
            : widget.name,
        button: true,
        child: InkWell(
          onTap: () => Navigator.of(context).push(
            MaterialPageRoute<void>(
              builder: (_) => _ImagePreview(
                bytes: snapshot.data!,
                path: widget.path,
                name: widget.name,
                share: widget.share,
              ),
            ),
          ),
          child: ClipRRect(
            borderRadius: BorderRadius.circular(12),
            child: Image.memory(
              snapshot.data!,
              width: 180,
              height: 140,
              fit: BoxFit.cover,
              errorBuilder: (_, _, _) =>
                  const Icon(Icons.broken_image_outlined),
            ),
          ),
        ),
      );
    },
  );
}

class _ImagePreview extends StatefulWidget {
  const _ImagePreview({
    required this.bytes,
    required this.path,
    required this.name,
    this.share,
  });
  final Uint8List bytes;
  final String path, name;
  final Future<ShareResult> Function(ShareParams)? share;
  @override
  State<_ImagePreview> createState() => _ImagePreviewState();
}

class _ImagePreviewState extends State<_ImagePreview> {
  bool _sharing = false;

  Future<void> _share(BuildContext buttonContext) async {
    if (_sharing) return;
    final box = buttonContext.findRenderObject() as RenderBox?;
    if (box == null || !box.hasSize) return;
    final origin = box.localToGlobal(Offset.zero) & box.size;
    final format = _sharedImageFormat(widget.bytes);
    final suggested = (widget.name.isEmpty ? widget.path : widget.name)
        .replaceAll('\\', '/')
        .split('/')
        .last;
    final stem = suggested.replaceFirst(RegExp(r'\.[^.]+$'), '');
    final name = '${stem.isEmpty ? 'image' : stem}.${format.extension}';
    setState(() => _sharing = true);
    try {
      final params = ShareParams(
        files: [XFile.fromData(widget.bytes, mimeType: format.mimeType)],
        fileNameOverrides: [name],
        sharePositionOrigin: origin,
      );
      await (widget.share?.call(params) ?? SharePlus.instance.share(params));
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text(context.errorText(error))));
      }
    } finally {
      if (mounted) setState(() => _sharing = false);
    }
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(
      title: Text(widget.name),
      actions: [
        Builder(
          builder: (buttonContext) => IconButton(
            key: const Key('image-share'),
            tooltip: context.tr('分享图片', 'Share image'),
            icon: const Icon(Icons.ios_share),
            onPressed: _sharing ? null : () => _share(buttonContext),
          ),
        ),
      ],
    ),
    body: Center(
      child: InteractiveViewer(
        minScale: .5,
        maxScale: 8,
        child: Image.memory(widget.bytes, fit: BoxFit.contain),
      ),
    ),
  );
}

({String mimeType, String extension}) _sharedImageFormat(Uint8List bytes) {
  bool starts(List<int> prefix) =>
      bytes.length >= prefix.length &&
      List.generate(
        prefix.length,
        (i) => bytes[i] == prefix[i],
      ).every((same) => same);
  if (starts([0x89, 0x50, 0x4e, 0x47])) {
    return (mimeType: 'image/png', extension: 'png');
  }
  if (starts([0xff, 0xd8])) return (mimeType: 'image/jpeg', extension: 'jpg');
  if (starts([0x47, 0x49, 0x46])) {
    return (mimeType: 'image/gif', extension: 'gif');
  }
  if (starts([0x42, 0x4d])) return (mimeType: 'image/bmp', extension: 'bmp');
  if (bytes.length >= 12 &&
      ascii.decode(bytes.sublist(0, 4), allowInvalid: true) == 'RIFF' &&
      ascii.decode(bytes.sublist(8, 12), allowInvalid: true) == 'WEBP') {
    return (mimeType: 'image/webp', extension: 'webp');
  }
  if (starts([0x49, 0x49, 0x2a, 0]) || starts([0x4d, 0x4d, 0, 0x2a])) {
    return (mimeType: 'image/tiff', extension: 'tiff');
  }
  if (bytes.length >= 12 &&
      ascii.decode(bytes.sublist(4, 8), allowInvalid: true) == 'ftyp') {
    final brand = ascii.decode(bytes.sublist(8, 12), allowInvalid: true);
    if (brand == 'avif' || brand == 'avis') {
      return (mimeType: 'image/avif', extension: 'avif');
    }
    if (['heic', 'heix', 'hevc', 'hevx'].contains(brand)) {
      return (mimeType: 'image/heic', extension: 'heic');
    }
    if (brand == 'mif1' || brand == 'msf1') {
      return (mimeType: 'image/heif', extension: 'heif');
    }
  }
  return (mimeType: 'application/octet-stream', extension: 'bin');
}

class ToolPartView extends StatefulWidget {
  const ToolPartView({
    super.key,
    required this.session,
    required this.part,
    required this.preferences,
  });
  final SessionController session;
  final JsonMap part;
  final AppPreferences preferences;
  @override
  State<ToolPartView> createState() => _ToolPartViewState();
}

class _ToolPartViewState extends State<ToolPartView> {
  bool _expanded = false;
  Future<String>? _output;
  String get _summary {
    final call = jsonMap(widget.part['call']);
    for (final field in [
      'title',
      'description',
      'command',
      'path',
      'query',
      'name',
    ]) {
      if ('${call[field] ?? ''}'.isNotEmpty) return '${call[field]}';
    }
    return '${call['kind'] ?? ''}';
  }

  @override
  Widget build(BuildContext context) {
    final p = widget.part, c = context.colors;
    return Container(
      margin: const EdgeInsets.only(bottom: 6),
      decoration: BoxDecoration(
        border: Border(left: BorderSide(color: c.hairline, width: 2)),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          InkWell(
            onTap: () => setState(() => _expanded = !_expanded),
            child: Padding(
              padding: const EdgeInsets.fromLTRB(12, 8, 4, 8),
              child: Row(
                children: [
                  Icon(
                    p['isError'] == true
                        ? Icons.error_outline
                        : p['resolved'] == true
                        ? Icons.check
                        : Icons.more_horiz,
                    size: 16,
                    color: p['isError'] == true ? c.danger : c.secondary,
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      _summary,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontFamily: 'GeistMono',
                        fontSize: 12,
                        color: c.secondary,
                      ),
                    ),
                  ),
                  Icon(
                    _expanded ? Icons.expand_less : Icons.expand_more,
                    size: 17,
                    color: c.tertiary,
                  ),
                ],
              ),
            ),
          ),
          if (_expanded)
            Padding(
              padding: const EdgeInsets.fromLTRB(12, 0, 0, 8),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  if (p['output'] != null)
                    SelectableText(
                      '${p['output']}',
                      style: TextStyle(
                        fontFamily: 'GeistMono',
                        fontSize: 12,
                        color: c.secondary,
                      ),
                    ),
                  if (p['outputRef'] != null)
                    TextButton(
                      onPressed: () {
                        setState(
                          () => _output = widget.session.toolBlob(
                            '${p['outputRef']}',
                          ),
                        );
                      },
                      child: Text(context.tr('完整输出', 'Full output')),
                    ),
                  if (_output != null)
                    FutureBuilder<String>(
                      future: _output,
                      builder: (context, snapshot) => snapshot.hasError
                          ? Text(
                              context.errorText(snapshot.error),
                              style: TextStyle(color: c.danger),
                            )
                          : snapshot.hasData
                          ? SizedBox(
                              height: 320,
                              child: SingleChildScrollView(
                                child: SelectableText(
                                  snapshot.data!,
                                  style: const TextStyle(
                                    fontFamily: 'GeistMono',
                                    fontSize: 12,
                                  ),
                                ),
                              ),
                            )
                          : const LinearProgressIndicator(),
                    ),
                  if (p['diff'] != null || p['diffRef'] != null)
                    TextButton(
                      onPressed: _showDiff,
                      child: Text(context.tr('查看变更', 'View changes')),
                    ),
                  if (p['subagentRef'] != null)
                    TextButton(
                      onPressed: () => Navigator.push(
                        context,
                        MaterialPageRoute<void>(
                          builder: (_) => _SubagentPage(
                            parent: widget.session,
                            part: p,
                            preferences: widget.preferences,
                          ),
                        ),
                      ),
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          const Icon(Icons.account_tree_outlined, size: 16),
                          const SizedBox(width: 8),
                          Text(context.tr('子智能体', 'Subagent')),
                          if (p['subagentStatus'] != null)
                            Text(
                              ' · ${context.statusLabel('${p['subagentStatus']}')}',
                              style: TextStyle(color: c.secondary),
                            ),
                        ],
                      ),
                    ),
                  if ('${p['subagentTail'] ?? ''}'.isNotEmpty)
                    Text(
                      '${p['subagentTail']}',
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(color: c.secondary, fontSize: 12),
                    ),
                ],
              ),
            ),
        ],
      ),
    );
  }

  Future<void> _showDiff() async {
    try {
      dynamic diff = widget.part['diff'];
      diff ??= jsonDecode(
        await widget.session.toolBlob('${widget.part['diffRef']}'),
      );
      if (mounted) {
        await Navigator.push(
          context,
          MaterialPageRoute<void>(
            builder: (_) => DiffView(diff: jsonMap(diff)),
          ),
        );
      }
    } catch (error) {
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text(context.errorText(error))));
      }
    }
  }
}

class _SubagentPage extends StatefulWidget {
  const _SubagentPage({
    required this.parent,
    required this.part,
    required this.preferences,
  });
  final SessionController parent;
  final JsonMap part;
  final AppPreferences preferences;
  @override
  State<_SubagentPage> createState() => _SubagentPageState();
}

class _SubagentPageState extends State<_SubagentPage> {
  SessionController? _live;
  List<JsonMap>? _frozen;
  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final ref = '${widget.part['subagentRef']}';
    if (widget.part['subagentStatus'] != 'running') {
      try {
        final source = ref.split('--sub--').first;
        final blobRef = ref.contains('/') ? ref : '$source/$ref';
        final value = jsonDecode(await widget.parent.toolBlob(blobRef));
        final entries = value is List
            ? jsonMaps(value)
            : jsonMaps(jsonMap(value)['entries'] ?? jsonMap(value)['messages']);
        if (mounted) setState(() => _frozen = entries);
        return;
      } catch (_) {
        /* Older hosts keep the child as a live read-only document. */
      }
    }
    if (mounted) {
      setState(() => _live = widget.parent.workspace.retainSession(ref));
    }
  }

  @override
  void dispose() {
    if (_live case final controller?) {
      disposeVisualScope(
        '${controller.workspace.cacheScope}:${controller.chatId}',
      );
      controller.workspace.releaseSession(controller);
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(title: Text(context.tr('子智能体', 'Subagent'))),
    body: _frozen != null
        ? _list(_frozen!)
        : _live != null
        ? ListenableBuilder(
            listenable: _live!,
            builder: (_, _) => _list(_live!.entries),
          )
        : const Center(child: CircularProgressIndicator()),
  );
  Widget _list(List<JsonMap> entries) => ListView.builder(
    itemCount: entries.length,
    itemBuilder: (_, index) => MessageView(
      session: _live ?? widget.parent,
      entry: entries[index],
      preferences: widget.preferences,
    ),
  );
}

class _RemoteVisual extends StatefulWidget {
  const _RemoteVisual({super.key, required this.session, required this.part});
  final SessionController session;
  final JsonMap part;
  @override
  State<_RemoteVisual> createState() => _RemoteVisualState();
}

class _RemoteVisualState extends State<_RemoteVisual> {
  late Future<String?> _key;
  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void didUpdateWidget(_RemoteVisual old) {
    super.didUpdateWidget(old);
    if (old.session.workspace.cacheScope !=
        widget.session.workspace.cacheScope) {
      _load();
    }
  }

  void _load() => _key = widget.session.workspace.store.read(
    'rcode.mobile.key.${widget.session.workspace.cacheScope}',
  );
  @override
  Widget build(BuildContext context) => FutureBuilder<String?>(
    future: _key,
    builder: (context, snapshot) {
      if (!snapshot.hasData || widget.session.workspace.host == null) {
        return const SizedBox(
          height: 72,
          child: Center(child: CircularProgressIndicator()),
        );
      }
      return RCodeVisual(
        scope:
            '${widget.session.workspace.cacheScope}:${widget.session.chatId}',
        endpoint: widget.session.workspace.host!,
        apiKey: snapshot.data!,
        deviceId:
            '${widget.session.chat?['deviceId'] ?? widget.session.workspace.deviceId}',
        path: '${widget.part['path']}',
        title: '${widget.part['title'] ?? ''}',
        id: '${widget.part['id']}',
        textScale: MediaQuery.textScalerOf(context).scale(1),
      );
    },
  );
}
