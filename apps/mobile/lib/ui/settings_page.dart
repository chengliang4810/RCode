import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:image_picker/image_picker.dart';
import 'package:path_provider/path_provider.dart';

import '../data/workspace_controller.dart';
import '../l10n.dart';
import '../preferences.dart';
import '../theme.dart';
import 'project_browser_page.dart' show fileName;
import 'sessions_page.dart';

class SettingsPage extends StatelessWidget {
  const SettingsPage({
    super.key,
    required this.workspace,
    required this.preferences,
    required this.onOpen,
    this.embedded = false,
  });
  final WorkspaceController workspace;
  final AppPreferences preferences;
  final ValueChanged<String> onOpen;
  final bool embedded;

  static const mathFonts = {
    'fira': 'Fira Math',
    'stixTwo': 'STIX Two',
    'computerModern': 'Computer Modern',
    'libertinus': 'Libertinus',
    'pagella': 'Palatino',
    'euler': 'Euler',
  };

  Future<void> _notifications(BuildContext context, bool enabled) async {
    await performUiAction(context, () async {
      if (!enabled) {
        await preferences.setValue('notifications', false);
        return;
      }
      final plugin = FlutterLocalNotificationsPlugin();
      final allowed = Platform.isIOS
          ? await plugin
                .resolvePlatformSpecificImplementation<
                  IOSFlutterLocalNotificationsPlugin
                >()
                ?.requestPermissions(alert: true, badge: true, sound: true)
          : Platform.isAndroid
          ? await plugin
                .resolvePlatformSpecificImplementation<
                  AndroidFlutterLocalNotificationsPlugin
                >()
                ?.requestNotificationsPermission()
          : true;
      await preferences.setValue('notifications', allowed == true);
      if (allowed != true && context.mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(
            content: Text(
              context.tr(
                '请在系统设置中允许通知',
                'Allow notifications in system settings',
              ),
            ),
          ),
        );
      }
    });
  }

  Future<void> _wallpaper(
    BuildContext context,
  ) => performUiAction(context, () async {
    final picked = await ImagePicker().pickImage(
      source: ImageSource.gallery,
      maxWidth: 1600,
      maxHeight: 1600,
      imageQuality: 90,
      requestFullMetadata: false,
    );
    if (picked == null) return;
    final directory = await getApplicationDocumentsDirectory();
    final destination = File('${directory.path}/rcode-wallpaper.jpg');
    await destination.writeAsBytes(await picked.readAsBytes(), flush: true);
    await preferences.setJson('wallpaperInfo', {'name': picked.name});
    // A new filename invalidates Flutter's image caches even when replacing an image.
    final revised = File(
      '${directory.path}/rcode-wallpaper-${DateTime.now().millisecondsSinceEpoch}.jpg',
    );
    await destination.rename(revised.path);
    final previous = preferences.wallpaperPath;
    await preferences.setValue('wallpaperPath', revised.path);
    if (previous != null &&
        previous.startsWith(
          '${directory.path}${Platform.pathSeparator}rcode-wallpaper-',
        )) {
      try {
        await File(previous).delete();
      } on FileSystemException {
        /* Already removed. */
      }
    }
  });

  Future<void> _signOut(BuildContext context) async {
    final leave = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text(
          context.tr('断开并忘记此主机？', 'Disconnect and forget this host?'),
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: Text(context.tr('取消', 'Cancel')),
          ),
          FilledButton(
            onPressed: () => Navigator.pop(context, true),
            child: Text(context.tr('断开连接', 'Disconnect')),
          ),
        ],
      ),
    );
    if (leave == true && context.mounted) {
      await performUiAction(context, () => workspace.disconnect());
      if (context.mounted) {
        Navigator.of(context).popUntil((route) => route.isFirst);
      }
    }
  }

  Widget _group(BuildContext context, String label, List<Widget> rows) =>
      Padding(
        padding: const EdgeInsets.fromLTRB(20, 12, 20, 12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(12, 0, 12, 7),
              child: Text(
                label,
                style: TextStyle(
                  color: context.colors.secondary,
                  fontSize: 13,
                  fontWeight: FontWeight.w500,
                ),
              ),
            ),
            Material(
              color: context.colors.elevated,
              borderRadius: BorderRadius.circular(16),
              clipBehavior: Clip.antiAlias,
              child: Column(
                children: [
                  for (var i = 0; i < rows.length; i++) ...[
                    if (i > 0)
                      Divider(
                        height: 1,
                        indent: 52,
                        color: context.colors.hairline,
                      ),
                    rows[i],
                  ],
                ],
              ),
            ),
          ],
        ),
      );

  Widget _toggle(
    BuildContext context,
    String key,
    String label,
    IconData icon,
    bool value, {
    ValueChanged<bool>? onChanged,
  }) => SwitchListTile(
    key: ValueKey('settings-$key'),
    title: Text(label, style: const TextStyle(fontSize: 16)),
    secondary: Icon(icon, size: 21, color: context.colors.secondary),
    value: value,
    activeThumbColor: context.colors.accent,
    onChanged: onChanged ?? (value) => preferences.setValue(key, value),
  );

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([preferences, workspace]),
    builder: (context, _) {
      final colors = context.colors;
      final effects = {
        'none': context.tr('无', 'None'),
        'dither': context.tr('抖色', 'Dither'),
        'ascii': context.tr('字符画', 'ASCII'),
        'halftone': context.tr('半色调', 'Halftone'),
        'scanlines': context.tr('扫描线', 'Scanlines'),
      };
      return Scaffold(
        backgroundColor: colors.background,
        appBar: AppBar(
          title: Text(context.tr('设置', 'Settings')),
          titleTextStyle: TextStyle(
            fontFamily: 'Geist',
            fontSize: embedded ? 24 : 30,
            fontWeight: FontWeight.w600,
            color: colors.text,
          ),
        ),
        body: ListView(
          padding: const EdgeInsets.only(bottom: 100),
          children: [
            _group(context, context.tr('连接', 'Connection'), [
              ListTile(
                leading: const Icon(Icons.dns_outlined),
                title: Text(
                  Uri.tryParse(workspace.host ?? '')?.host ?? 'RCode',
                ),
                subtitle: Text(
                  workspace.host ?? '',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                ),
                trailing: Icon(
                  Icons.circle,
                  size: 8,
                  color: workspace.connected ? colors.success : colors.tertiary,
                ),
              ),
              ListTile(
                key: const Key('settings-reconnect'),
                leading: const Icon(Icons.refresh),
                title: Text(context.tr('重新连接', 'Reconnect')),
                onTap: () => performUiAction(context, workspace.reconnect),
              ),
            ]),
            _group(context, context.tr('设备', 'Devices'), [
              for (final device in workspace.devices)
                ListTile(
                  leading: const Icon(Icons.computer_outlined),
                  title: Text(
                    device['name'] as String? ?? device['id'] as String,
                  ),
                  subtitle: Text(
                    context.platformLabel(device['platform'] as String? ?? ''),
                  ),
                  trailing: Icon(
                    Icons.circle,
                    size: 8,
                    color:
                        workspace.connected &&
                            (device['id'] == workspace.deviceId ||
                                device['lastSeenAt'] != null)
                        ? colors.success
                        : colors.tertiary,
                  ),
                ),
              if (workspace.devices.isEmpty)
                ListTile(
                  title: Text(context.tr('没有可用设备', 'No devices available')),
                ),
            ]),
            _group(context, context.tr('通知', 'Notifications'), [
              _toggle(
                context,
                'notifications',
                context.tr('通知', 'Notifications'),
                Icons.notifications_outlined,
                preferences.notifications,
                onChanged: (value) => _notifications(context, value),
              ),
              if (preferences.notifications) ...[
                _toggle(
                  context,
                  'notifyDone',
                  context.tr('运行完成', 'Run finished'),
                  Icons.check_circle_outline,
                  preferences.notifyDone,
                ),
                _toggle(
                  context,
                  'notifyInput',
                  context.tr('等待输入', 'Waiting on your input'),
                  Icons.chat_bubble_outline,
                  preferences.notifyInput,
                ),
                _toggle(
                  context,
                  'notifyFailed',
                  context.tr('运行失败', 'Run failed'),
                  Icons.error_outline,
                  preferences.notifyFailed,
                ),
              ],
            ]),
            _group(context, context.tr('外观', 'Appearance'), [
              for (final mode in ThemeMode.values)
                ListTile(
                  key: ValueKey('appearance-${mode.name}'),
                  leading: Icon(switch (mode) {
                    ThemeMode.system => Icons.brightness_auto_outlined,
                    ThemeMode.light => Icons.light_mode_outlined,
                    ThemeMode.dark => Icons.dark_mode_outlined,
                  }),
                  title: Text(switch (mode) {
                    ThemeMode.system => context.tr('跟随系统', 'System'),
                    ThemeMode.light => context.tr('浅色', 'Light'),
                    ThemeMode.dark => context.tr('深色', 'Dark'),
                  }),
                  trailing: preferences.themeMode == mode
                      ? Icon(Icons.check, color: colors.accent)
                      : null,
                  onTap: () => preferences.setValue('themeMode', mode),
                ),
              ListTile(
                leading: const Icon(Icons.language),
                title: Text(context.tr('语言', 'Language')),
                trailing: DropdownButton<String>(
                  key: const Key('settings-language'),
                  value: preferences.language,
                  underline: const SizedBox(),
                  items: [
                    DropdownMenuItem(
                      value: 'system',
                      child: Text(context.tr('跟随系统', 'System')),
                    ),
                    const DropdownMenuItem(value: 'zh', child: Text('中文')),
                    const DropdownMenuItem(value: 'en', child: Text('English')),
                  ],
                  onChanged: (value) => preferences.setValue('language', value),
                ),
              ),
            ]),
            _group(context, context.tr('壁纸', 'Wallpaper'), [
              ListTile(
                key: const Key('settings-wallpaper'),
                leading: const Icon(Icons.photo_outlined),
                title: Text(
                  preferences.wallpaperPath == null
                      ? context.tr('选择壁纸', 'Choose Wallpaper')
                      : context.tr('更换壁纸', 'Change Wallpaper'),
                ),
                subtitle: preferences.wallpaperPath == null
                    ? null
                    : Text(
                        preferences.getJson('wallpaperInfo')?['name']
                                as String? ??
                            fileName(preferences.wallpaperPath!),
                        maxLines: 1,
                      ),
                trailing: const Icon(Icons.chevron_right),
                onTap: () => _wallpaper(context),
              ),
              if (preferences.wallpaperPath != null) ...[
                ListTile(
                  leading: const Icon(Icons.auto_fix_high_outlined),
                  title: Text(context.tr('效果', 'Effect')),
                  trailing: DropdownButton<String>(
                    value: effects.containsKey(preferences.wallpaperEffect)
                        ? preferences.wallpaperEffect
                        : 'none',
                    underline: const SizedBox(),
                    items: effects.entries
                        .map(
                          (entry) => DropdownMenuItem(
                            value: entry.key,
                            child: Text(entry.value),
                          ),
                        )
                        .toList(),
                    onChanged: (value) =>
                        preferences.setValue('wallpaperEffect', value),
                  ),
                ),
                ListTile(
                  leading: Icon(Icons.delete_outline, color: colors.danger),
                  title: Text(
                    context.tr('移除壁纸', 'Remove Wallpaper'),
                    style: TextStyle(color: colors.danger),
                  ),
                  onTap: () => preferences.setValue('wallpaperPath', null),
                ),
              ],
            ]),
            _group(context, context.tr('数学公式', 'Math'), [
              _toggle(
                context,
                'renderMath',
                context.tr('渲染数学公式', 'Render Math'),
                Icons.functions,
                preferences.renderMath,
              ),
              if (preferences.renderMath) ...[
                ListTile(
                  leading: const Icon(Icons.font_download_outlined),
                  title: Text(context.tr('公式字体', 'Math Font')),
                  trailing: DropdownButton<String>(
                    value: mathFonts.containsKey(preferences.mathFont)
                        ? preferences.mathFont
                        : 'fira',
                    underline: const SizedBox(),
                    items: mathFonts.entries
                        .map(
                          (entry) => DropdownMenuItem(
                            value: entry.key,
                            child: Text(
                              entry.value,
                              style: const TextStyle(fontSize: 14),
                            ),
                          ),
                        )
                        .toList(),
                    onChanged: (value) =>
                        preferences.setValue('mathFont', value),
                  ),
                ),
                ListTile(
                  leading: const Icon(Icons.format_size),
                  title: Text(context.tr('公式大小', 'Math Size')),
                  trailing: DropdownButton<double>(
                    value: preferences.mathSize,
                    underline: const SizedBox(),
                    items: [.9, 1.0, 1.1, 1.2, 1.35, 1.5]
                        .map(
                          (size) => DropdownMenuItem(
                            value: size,
                            child: Text('${(size * 100).round()}%'),
                          ),
                        )
                        .toList(),
                    onChanged: (value) =>
                        preferences.setValue('mathSize', value),
                  ),
                ),
              ],
            ]),
            _group(context, context.tr('会话', 'Sessions'), [
              _toggle(
                context,
                'groupByProject',
                context.tr('按项目分组', 'Group by Project'),
                Icons.stacked_bar_chart,
                preferences.groupByProject,
              ),
              ListTile(
                leading: const Icon(Icons.archive_outlined),
                title: Text(context.tr('已归档会话', 'Archived Sessions')),
                trailing: const Icon(Icons.chevron_right),
                onTap: () => Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => SessionsPage(
                      workspace: workspace,
                      preferences: preferences,
                      onOpen: onOpen,
                      archived: true,
                    ),
                  ),
                ),
              ),
            ]),
            _group(context, ' ', [
              ListTile(
                key: const Key('settings-disconnect'),
                leading: Icon(Icons.logout, color: colors.danger),
                title: Text(
                  context.tr('断开连接', 'Disconnect'),
                  style: TextStyle(color: colors.danger),
                ),
                onTap: () => _signOut(context),
              ),
            ]),
          ],
        ),
      );
    },
  );
}
