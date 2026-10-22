import 'package:flutter/material.dart';

import '../data/workspace_controller.dart';
import '../l10n.dart';
import '../theme.dart';

/// The same compact address/key form on both mobile platforms.
class ConnectionPage extends StatefulWidget {
  const ConnectionPage({super.key, required this.workspace});

  final WorkspaceController workspace;

  @override
  State<ConnectionPage> createState() => _ConnectionPageState();
}

class _ConnectionPageState extends State<ConnectionPage> {
  late final TextEditingController _host;
  final _key = TextEditingController();
  final _form = GlobalKey<FormState>();
  bool _visible = false;
  bool _busy = false;
  Object? _error;

  @override
  void initState() {
    super.initState();
    _host = TextEditingController(text: widget.workspace.host ?? '');
  }

  @override
  void dispose() {
    _host.dispose();
    _key.dispose();
    super.dispose();
  }

  Future<void> _connect() async {
    if (_busy || !(_form.currentState?.validate() ?? false)) return;
    FocusScope.of(context).unfocus();
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await widget.workspace.connect(_host.text.trim(), _key.text.trim());
      if (mounted) _key.clear();
    } catch (error) {
      if (mounted) {
        setState(() => _error = error);
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Scaffold(
      backgroundColor: colors.background,
      body: SafeArea(
        child: LayoutBuilder(
          builder: (context, constraints) => SingleChildScrollView(
            padding: const EdgeInsets.all(24),
            child: ConstrainedBox(
              constraints: BoxConstraints(
                minHeight: (constraints.maxHeight - 48).clamp(
                  0,
                  double.infinity,
                ),
              ),
              child: IntrinsicHeight(
                child: Center(
                  child: ConstrainedBox(
                    constraints: const BoxConstraints(maxWidth: 440),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.stretch,
                      children: [
                        const Spacer(flex: 3),
                        Icon(
                          Icons.auto_awesome_outlined,
                          size: 48,
                          color: colors.text,
                        ),
                        const SizedBox(height: 12),
                        Text(
                          'RCode',
                          textAlign: TextAlign.center,
                          style: TextStyle(
                            fontSize: 34,
                            fontWeight: FontWeight.w600,
                            color: colors.text,
                          ),
                        ),
                        const Spacer(flex: 4),
                        Form(
                          key: _form,
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.stretch,
                            children: [
                              if (_error != null) ...[
                                Text(
                                  context.errorText(_error),
                                  key: const Key('connection-error'),
                                  textAlign: TextAlign.center,
                                  style: TextStyle(color: colors.danger),
                                ),
                                const SizedBox(height: 12),
                              ],
                              TextFormField(
                                key: const Key('pair-host'),
                                controller: _host,
                                enabled: !_busy,
                                keyboardType: TextInputType.url,
                                autocorrect: false,
                                textCapitalization: TextCapitalization.none,
                                decoration: InputDecoration(
                                  labelText: context.tr(
                                    '服务器地址',
                                    'Server address',
                                  ),
                                  hintText: 'wss://host.example:27655',
                                ),
                                validator: (value) {
                                  final url = Uri.tryParse(
                                    (value ?? '').trim(),
                                  );
                                  if (url == null ||
                                      ![
                                        'http',
                                        'https',
                                        'ws',
                                        'wss',
                                      ].contains(url.scheme) ||
                                      url.host.isEmpty ||
                                      url.userInfo.isNotEmpty ||
                                      url.hasQuery ||
                                      url.hasFragment) {
                                    return context.tr(
                                      '请输入有效的服务器地址',
                                      'Enter a valid server address',
                                    );
                                  }
                                  return null;
                                },
                              ),
                              const SizedBox(height: 12),
                              TextFormField(
                                key: const Key('pair-token'),
                                controller: _key,
                                enabled: !_busy,
                                obscureText: !_visible,
                                autocorrect: false,
                                enableSuggestions: false,
                                textInputAction: TextInputAction.done,
                                onFieldSubmitted: (_) => _connect(),
                                decoration: InputDecoration(
                                  labelText: context.tr('API 密钥', 'API Key'),
                                  suffixIcon: IconButton(
                                    tooltip: _visible
                                        ? context.tr('隐藏密钥', 'Hide key')
                                        : context.tr('显示密钥', 'Show key'),
                                    icon: Icon(
                                      _visible
                                          ? Icons.visibility_off_outlined
                                          : Icons.visibility_outlined,
                                    ),
                                    onPressed: () =>
                                        setState(() => _visible = !_visible),
                                  ),
                                ),
                                validator: (value) {
                                  final key = (value ?? '').trim();
                                  return RegExp(r'^[A-Za-z0-9]{8,}$')
                                              .hasMatch(key) &&
                                          RegExp('[A-Za-z]').hasMatch(key) &&
                                          RegExp('[0-9]').hasMatch(key)
                                      ? null
                                      : context.tr(
                                          '至少 8 位，且同时包含字母和数字',
                                          'Use at least 8 letters and numbers',
                                        );
                                },
                              ),
                              const SizedBox(height: 16),
                              FilledButton(
                                key: const Key('pair'),
                                onPressed: _busy ? null : _connect,
                                style: FilledButton.styleFrom(
                                  backgroundColor: colors.text,
                                  foregroundColor: colors.background,
                                  minimumSize: const Size.fromHeight(52),
                                  shape: const StadiumBorder(),
                                ),
                                child: _busy
                                    ? SizedBox(
                                        width: 20,
                                        height: 20,
                                        child: CircularProgressIndicator(
                                          strokeWidth: 2,
                                          color: colors.background,
                                        ),
                                      )
                                    : Text(context.tr('连接', 'Connect')),
                              ),
                            ],
                          ),
                        ),
                        const SizedBox(height: 8),
                      ],
                    ),
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
