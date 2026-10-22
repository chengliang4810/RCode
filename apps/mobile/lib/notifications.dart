import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';

import 'data/workspace_controller.dart';
import 'preferences.dart';

/// Converts live transitions into local notices. Initial/reconnect snapshots are quiet.
class NotificationTracker {
  String? _scope;
  final Map<String, JsonMap> _previous = {};
  List<({String chatId, String kind})> update(
    String scope,
    List<JsonMap> sessions,
  ) {
    final events = <({String chatId, String kind})>[];
    final fresh = scope != _scope;
    if (fresh) {
      _previous.clear();
      _scope = scope;
    }
    for (final row in sessions) {
      final id = '${row['chatId']}';
      final before = _previous[id];
      if (!fresh && before != null) {
        if (row['status'] == 'awaitingInput' &&
            before['status'] != 'awaitingInput') {
          events.add((chatId: id, kind: 'input'));
        } else if (row['status'] == 'errored' &&
            before['status'] != 'errored') {
          events.add((chatId: id, kind: 'failed'));
        } else if ((row['lastCompletedTurn'] != null &&
                row['lastCompletedTurn'] != before['lastCompletedTurn']) ||
            (!row.containsKey('lastCompletedTurn') &&
                row['status'] == 'idle' &&
                before['status'] == 'working')) {
          events.add((chatId: id, kind: 'done'));
        }
      }
      _previous[id] = {...row};
    }
    _previous.removeWhere(
      (id, _) => !sessions.any((row) => row['chatId'] == id),
    );
    return events;
  }

  void reset() {
    _scope = null;
    _previous.clear();
  }
}

class MobileNotifications {
  MobileNotifications(this.workspace, this.preferences, this.openChat);
  final WorkspaceController workspace;
  final AppPreferences preferences;
  final void Function(String chatId) openChat;
  final _plugin = FlutterLocalNotificationsPlugin();
  final _tracker = NotificationTracker();
  bool foreground = true, _ready = false, _disposed = false;
  String? _pendingChat, _pendingScope;

  Future<void> initialize() async {
    if (!Platform.isAndroid && !Platform.isIOS) return;
    await _plugin.initialize(
      settings: const InitializationSettings(
        android: AndroidInitializationSettings('@mipmap/ic_launcher'),
        iOS: DarwinInitializationSettings(
          requestAlertPermission: false,
          requestBadgePermission: false,
          requestSoundPermission: false,
        ),
      ),
      onDidReceiveNotificationResponse: (response) => _route(response.payload),
    );
    if (_disposed) return;
    _ready = true;
    workspace.addListener(_changed);
    final launch = await _plugin.getNotificationAppLaunchDetails();
    if (!_disposed && launch?.didNotificationLaunchApp == true) {
      _route(launch?.notificationResponse?.payload);
    }
    _changed();
  }

  void _route(String? payload) {
    if (payload == null || _disposed) return;
    try {
      final data = jsonMap(jsonDecode(payload));
      _pendingChat = data['chatId'] as String?;
      _pendingScope = data['scope'] as String?;
      _navigate();
    } catch (_) {
      /* Stale or foreign notifications cannot navigate the app. */
    }
  }

  void _navigate() {
    final id = _pendingChat;
    if (id == null ||
        !workspace.authenticated ||
        workspace.chat(id) == null ||
        _pendingScope != workspace.cacheScope) {
      return;
    }
    _pendingChat = null;
    final scope = _pendingScope;
    _pendingScope = null;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!_disposed &&
          workspace.authenticated &&
          workspace.cacheScope == scope &&
          workspace.chat(id) != null) {
        openChat(id);
      }
    });
  }

  void _changed() {
    _navigate();
    if (!workspace.authenticated || !workspace.connected) {
      _tracker.reset();
      return;
    }
    final events = _tracker.update(workspace.cacheScope, workspace.sessions);
    if (!_ready || foreground || !preferences.notifications) return;
    for (final event in events) {
      final allowed = switch (event.kind) {
        'input' => preferences.notifyInput,
        'failed' => preferences.notifyFailed,
        _ => preferences.notifyDone,
      };
      if (!allowed) continue;
      final zh =
          preferences.language == 'zh' ||
          (preferences.language == 'system' &&
              WidgetsBinding.instance.platformDispatcher.locale.languageCode ==
                  'zh');
      final body = switch (event.kind) {
        'input' => zh ? '需要你的回答' : 'Your input is needed',
        'failed' => zh ? '任务失败' : 'Task failed',
        _ => zh ? '任务已完成' : 'Task completed',
      };
      unawaited(
        _plugin
            .show(
              id: event.chatId.hashCode & 0x7fffffff,
              title: '${workspace.chat(event.chatId)?['title'] ?? 'RCode'}',
              body: body,
              notificationDetails: const NotificationDetails(
                android: AndroidNotificationDetails(
                  'rcode_runs',
                  'RCode',
                  importance: Importance.high,
                  priority: Priority.high,
                ),
                iOS: DarwinNotificationDetails(),
              ),
              payload: jsonEncode({
                'chatId': event.chatId,
                'scope': workspace.cacheScope,
              }),
            )
            .catchError((_) {}),
      );
    }
  }

  void dispose() {
    _disposed = true;
    workspace.removeListener(_changed);
  }
}
