import 'dart:math';

import 'package:flutter/material.dart';

import '../data/workspace_controller.dart';
import '../l10n.dart';
import '../theme.dart';

String mobileId(String prefix) =>
    '$prefix-${DateTime.now().microsecondsSinceEpoch.toRadixString(36)}-${Random.secure().nextInt(1 << 32).toRadixString(36)}';

List<Map<String, dynamic>> jsonRows(dynamic value) => value is List
    ? value
          .whereType<Map>()
          .map((row) => Map<String, dynamic>.from(row))
          .toList()
    : const [];

String fileName(String path) {
  final parts = path
      .replaceAll('\\', '/')
      .split('/')
      .where((p) => p.isNotEmpty)
      .toList();
  return parts.isEmpty ? path : parts.last;
}

String? parentFolder(String path) {
  final normalized = path
      .replaceAll('\\', '/')
      .replaceFirst(RegExp(r'/+$'), '');
  if (normalized.isEmpty || RegExp(r'^[A-Za-z]:$').hasMatch(normalized)) {
    return null;
  }
  final slash = normalized.lastIndexOf('/');
  if (slash < 0) return null;
  if (slash == 0) return '/';
  final parent = normalized.substring(0, slash);
  return RegExp(r'^[A-Za-z]:$').hasMatch(parent) ? '$parent/' : parent;
}

class ProjectBrowserPage extends StatefulWidget {
  const ProjectBrowserPage({
    super.key,
    required this.workspace,
    this.initialDeviceId,
  });

  final WorkspaceController workspace;
  final String? initialDeviceId;

  @override
  State<ProjectBrowserPage> createState() => _ProjectBrowserPageState();
}

class _ProjectBrowserPageState extends State<ProjectBrowserPage> {
  String? _deviceId;
  Map<String, dynamic>? _listing;
  String? _listingDevice;
  bool _loading = true;
  bool _saving = false;
  Object? _error;
  int _generation = 0;

  List<Map<String, dynamic>> get _devices => widget.workspace.devices
      .where(
        (device) => !['ios', 'ipados', 'android'].contains(device['platform']),
      )
      .toList();

  @override
  void initState() {
    super.initState();
    _deviceId =
        widget.initialDeviceId ??
        (_devices.isNotEmpty ? _devices.first['id'] as String : null);
    _load(null);
  }

  @override
  void dispose() {
    _generation++;
    super.dispose();
  }

  Future<void> _load(String? path) async {
    final device = _deviceId;
    final generation = ++_generation;
    if (device == null) {
      setState(() => _loading = false);
      return;
    }
    setState(() {
      _loading = true;
      _error = null;
    });
    try {
      final result = Map<String, dynamic>.from(
        await widget.workspace.call('ListFolders', {
          'targetDeviceId': device,
          'path': ?path,
        }) as Map,
      );
      if (!mounted || generation != _generation || _deviceId != device) return;
      setState(() {
        _listing = result;
        _listingDevice = device;
      });
    } catch (error) {
      if (mounted && generation == _generation) {
        setState(() => _error = error);
      }
    } finally {
      if (mounted && generation == _generation) {
        setState(() => _loading = false);
      }
    }
  }

  Future<void> _useFolder() async {
    if (_loading ||
        _saving ||
        _listing == null ||
        _listingDevice != _deviceId) {
      return;
    }
    final device = _deviceId!;
    final path = _listing!['path'] as String;
    final existing = widget.workspace.spaces.where(
      (space) => space['deviceId'] == device && space['path'] == path,
    );
    if (existing.isNotEmpty) {
      Navigator.of(context).pop(existing.first['id'] as String);
      return;
    }
    setState(() => _saving = true);
    final id = mobileId('space');
    try {
      await widget.workspace.call('Mutate', {
        'op': 'createSpace',
        'spaceId': id,
        'deviceId': device,
        'path': path,
        'gitDetected': jsonRows(_listing!['entries'])
            .any((row) => row['name'] == '.git'),
      });
      if (mounted) Navigator.of(context).pop(id);
    } catch (error) {
      if (mounted) setState(() => _error = error);
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final path = _listing?['path'] as String?;
    final entries = jsonRows(_listing?['entries'])
        .where((entry) => entry['isDir'] == true)
        .toList();
    final selected = _devices
        .where((device) => device['id'] == _deviceId)
        .firstOrNull;
    return Scaffold(
      backgroundColor: colors.background,
      appBar: AppBar(
        title: Text(context.tr('新建项目', 'New Project')),
        actions: [
          PopupMenuButton<String>(
            key: const Key('project-device'),
            tooltip: context.tr('设备', 'Device'),
            initialValue: _deviceId,
            onSelected: (device) {
              setState(() {
                _deviceId = device;
                _listing = null;
                _listingDevice = null;
              });
              _load(null);
            },
            itemBuilder: (_) => _devices
                .map(
                  (device) => PopupMenuItem<String>(
                    value: device['id'] as String,
                    child: Text(
                      device['name'] as String? ?? device['id'] as String,
                    ),
                  ),
                )
                .toList(),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 16),
              child: Center(
                child: Text(
                  selected?['name'] as String? ??
                      context.tr('选择设备', 'Choose device'),
                ),
              ),
            ),
          ),
        ],
      ),
      body: Column(
        children: [
          if (_loading) const LinearProgressIndicator(minHeight: 2),
          if (_error != null)
            Padding(
              padding: const EdgeInsets.all(16),
              child: Row(
                children: [
                  Expanded(
                    child: Text(
                      context.errorText(_error),
                      style: TextStyle(color: colors.danger),
                    ),
                  ),
                  IconButton(
                    tooltip: context.tr('重试', 'Retry'),
                    onPressed: () => _load(path),
                    icon: const Icon(Icons.refresh),
                  ),
                ],
              ),
            ),
          Expanded(
            child: _devices.isEmpty
                ? Center(
                    child: Text(
                      context.tr('没有可用的执行设备', 'No execution devices available'),
                    ),
                  )
                : ListView(
                    padding: const EdgeInsets.symmetric(
                      vertical: 8,
                      horizontal: 16,
                    ),
                    children: [
                      if (path != null && parentFolder(path) != null)
                        ListTile(
                          leading: const Icon(
                            Icons.drive_folder_upload_outlined,
                          ),
                          title: Text(context.tr('上级目录', 'Parent Folder')),
                          onTap: _loading
                              ? null
                              : () => _load(parentFolder(path)),
                        ),
                      for (final entry in entries)
                        ListTile(
                          key: ValueKey('folder-${entry['name']}'),
                          leading: Icon(
                            entry['isRepo'] == true
                                ? Icons.folder_special_outlined
                                : Icons.folder_outlined,
                            color: entry['isRepo'] == true
                                ? colors.accent
                                : colors.secondary,
                          ),
                          title: Text(entry['name'] as String),
                          trailing: const Icon(Icons.chevron_right),
                          onTap: _loading || path == null
                              ? null
                              : () => _load(
                                  '${path.replaceFirst(RegExp(r'[/\\]+$'), '')}/${entry['name']}',
                                ),
                        ),
                      if (!_loading && entries.isEmpty && _listing != null)
                        Padding(
                          padding: const EdgeInsets.all(24),
                          child: Text(
                            context.tr('没有子目录', 'No subfolders'),
                            textAlign: TextAlign.center,
                            style: TextStyle(color: colors.secondary),
                          ),
                        ),
                      if (_listing?['truncated'] == true)
                        Padding(
                          padding: const EdgeInsets.all(16),
                          child: Text(
                            context.tr(
                              '此目录仅显示部分内容',
                              'Only part of this folder is shown',
                            ),
                            style: TextStyle(color: colors.secondary),
                          ),
                        ),
                    ],
                  ),
          ),
          SafeArea(
            top: false,
            minimum: const EdgeInsets.fromLTRB(20, 12, 20, 12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                if (path != null)
                  Padding(
                    padding: const EdgeInsets.only(bottom: 8),
                    child: Text(
                      path,
                      textAlign: TextAlign.center,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(fontSize: 13, color: colors.secondary),
                    ),
                  ),
                FilledButton(
                  key: const Key('use-folder'),
                  onPressed:
                      _loading ||
                          _saving ||
                          _listingDevice != _deviceId ||
                          path == null
                      ? null
                      : _useFolder,
                  style: FilledButton.styleFrom(
                    backgroundColor: colors.text,
                    foregroundColor: colors.background,
                    minimumSize: const Size.fromHeight(48),
                    shape: const StadiumBorder(),
                  ),
                  child: _saving
                      ? const SizedBox(
                          width: 20,
                          height: 20,
                          child: CircularProgressIndicator(strokeWidth: 2),
                        )
                      : Text(
                          path == null
                              ? context.tr('使用此目录', 'Use Folder')
                              : context.tr(
                                  '使用 ${fileName(path)}',
                                  'Use ${fileName(path)}',
                                ),
                        ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
