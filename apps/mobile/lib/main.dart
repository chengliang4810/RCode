import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';

import 'data/workspace_controller.dart';
import 'notifications.dart';
import 'preferences.dart';
import 'theme.dart';
import 'ui/connection_page.dart';
import 'ui/session_page.dart';
import 'ui/workspace_shell.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final preferences = await AppPreferences.load();
  final workspace = WorkspaceController();
  runApp(RCodeApp(workspace: workspace, preferences: preferences));
  unawaited(workspace.initialize());
}

class RCodeApp extends StatefulWidget {
  const RCodeApp({
    super.key,
    required this.workspace,
    required this.preferences,
    this.enableNotifications = true,
  });
  final WorkspaceController workspace;
  final AppPreferences preferences;
  final bool enableNotifications;
  @override
  State<RCodeApp> createState() => _RCodeAppState();
}

class _RCodeAppState extends State<RCodeApp> with WidgetsBindingObserver {
  final _shell = GlobalKey<WorkspaceShellState>();
  MobileNotifications? _notifications;
  Object? _presentation;
  Widget? _application;
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    if (widget.enableNotifications) {
      _notifications = MobileNotifications(
        widget.workspace,
        widget.preferences,
        (id) => _shell.currentState?.openSession(id),
      );
      unawaited(_notifications!.initialize().catchError((_) {}));
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    _notifications?.foreground = state == AppLifecycleState.resumed;
    if (state == AppLifecycleState.resumed && widget.workspace.authenticated) {
      unawaited(widget.workspace.reconnect().catchError((_) {}));
    }
    if (state == AppLifecycleState.paused ||
        state == AppLifecycleState.inactive) {
      unawaited(widget.workspace.flushCaches().catchError((_) {}));
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _notifications?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: Listenable.merge([widget.preferences, widget.workspace]),
    builder: (_, _) {
      final presentation = (
        widget.workspace,
        widget.preferences,
        widget.workspace.authenticated,
        widget.workspace.host,
        widget.preferences.themeMode,
        widget.preferences.language,
      );
      if (presentation == _presentation && _application != null) {
        return _application!;
      }
      _presentation = presentation;
      return _application = MaterialApp(
        key: ValueKey(
          widget.workspace.authenticated
              ? widget.workspace.cacheScope
              : 'signedOut',
        ),
        title: 'RCode',
        debugShowCheckedModeBanner: false,
        theme: rcodeTheme(Brightness.light),
        darkTheme: rcodeTheme(Brightness.dark),
        themeMode: widget.preferences.themeMode,
        locale: widget.preferences.language == 'system'
            ? null
            : Locale(widget.preferences.language),
        supportedLocales: const [Locale('en'), Locale('zh')],
        localizationsDelegates: GlobalMaterialLocalizations.delegates,
        home: widget.workspace.authenticated
            ? WorkspaceShell(
                key: _shell,
                workspace: widget.workspace,
                preferences: widget.preferences,
                sessionBuilder: (id) => SessionPage(
                  key: ValueKey('${widget.workspace.cacheScope}:$id'),
                  workspace: widget.workspace,
                  chatId: id,
                  preferences: widget.preferences,
                ),
              )
            : ConnectionPage(workspace: widget.workspace),
      );
    },
  );
}
