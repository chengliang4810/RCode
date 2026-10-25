import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

typedef JsonMap = Map<String, dynamic>;

enum RpcConnection { disconnected, connecting, connected, reconnecting }

class RpcException implements Exception {
  const RpcException(this.code, this.message);
  final String code;
  final String message;
  @override
  String toString() => message;
}

String normalizeServerUrl(String raw) {
  var value = raw.trim();
  if (!value.contains('://')) value = 'ws://$value';
  final uri = Uri.tryParse(value);
  if (uri == null ||
      uri.host.isEmpty ||
      uri.userInfo.isNotEmpty ||
      uri.hasQuery ||
      uri.hasFragment ||
      !const ['ws', 'wss', 'http', 'https'].contains(uri.scheme)) {
    throw const RpcException('invalid_address', 'invalid_address');
  }
  final scheme = switch (uri.scheme) {
    'http' => 'ws',
    'https' => 'wss',
    _ => uri.scheme,
  };
  return uri
      .replace(
        scheme: scheme,
        port: uri.hasPort
            ? uri.port
            : (raw.trim().contains('://') ? null : 27655),
        path: uri.path.isEmpty ? '/' : uri.path,
      )
      .toString();
}

bool validApiKey(String key) =>
    key.length >= 8 &&
    RegExp(r'^[A-Za-z0-9]+$').hasMatch(key) &&
    RegExp(r'[A-Za-z]').hasMatch(key) &&
    RegExp(r'[0-9]').hasMatch(key);

/// One authenticated socket. Only read subscriptions survive a reconnect.
class RpcClient extends ChangeNotifier {
  RpcClient({
    this.retryDelay = const Duration(seconds: 1),
    this.callTimeout = const Duration(seconds: 30),
  });

  final Duration retryDelay;
  final Duration callTimeout;
  RpcConnection connection = RpcConnection.disconnected;
  Object? error;
  String? host;
  String? _key;
  WebSocket? _socket;
  StreamSubscription<dynamic>? _socketEvents;
  Timer? _retry;
  int _generation = 0;
  int _sequence = 0;
  bool _disposed = false;
  final Map<int, Completer<dynamic>> _pending = {};
  final Map<int, _RpcWatch> _watchIds = {};
  final Set<_RpcWatch> _watches = {};

  bool get connected => connection == RpcConnection.connected;

  Future<void> connect(String address, String apiKey) async {
    if (!validApiKey(apiKey)) {
      throw const RpcException('invalid_key', 'invalid_key');
    }
    final normalized = normalizeServerUrl(address);
    final generation = ++_generation;
    connection = RpcConnection.connecting;
    _key = null;
    await _closeSocket();
    if (_disposed || generation != _generation) return;
    host = normalized;
    _key = apiKey;
    connection = RpcConnection.connecting;
    error = null;
    notifyListeners();
    await _open(generation, initial: true);
  }

  Future<void> _open(int generation, {bool initial = false}) async {
    if (_disposed ||
        generation != _generation ||
        host == null ||
        _key == null) {
      return;
    }
    try {
      var timedOut = false;
      final socket =
          await WebSocket.connect(
                host!,
                headers: {'Authorization': 'Bearer ${_key!}'},
              )
              .then((socket) {
                // Future.timeout does not cancel the underlying handshake. A socket
                // arriving after its deadline has no owner and must be closed here.
                if (timedOut) unawaited(socket.close());
                return socket;
              })
              .timeout(
                callTimeout,
                onTimeout: () {
                  timedOut = true;
                  throw TimeoutException(
                    'WebSocket connection timed out',
                    callTimeout,
                  );
                },
              );
      if (_disposed || generation != _generation) {
        await socket.close();
        return;
      }
      _socket = socket;
      socket.pingInterval = const Duration(seconds: 15);
      connection = RpcConnection.connected;
      error = null;
      _socketEvents = socket.listen(
        (raw) => _receive(raw, generation),
        onError: (Object _) => _lost(generation),
        onDone: () => _lost(generation),
        cancelOnError: true,
      );
      for (final watch in _watches.toList()) {
        _subscribe(watch);
      }
      notifyListeners();
    } catch (_) {
      if (_disposed || generation != _generation) return;
      _lost(generation);
      if (initial) {
        throw const RpcException('connection_failed', 'connection_failed');
      }
    }
  }

  void _lost(int generation) {
    if (_disposed || generation != _generation || _key == null) return;
    final socket = _socket;
    _socket = null;
    unawaited(socket?.close());
    unawaited(_socketEvents?.cancel());
    _socketEvents = null;
    for (final pending in _pending.values) {
      if (!pending.isCompleted) {
        pending.completeError(
          const RpcException('unconfirmed', 'operation_unconfirmed'),
        );
      }
    }
    _pending.clear();
    _watchIds.clear();
    for (final watch in _watches) {
      watch.id = null;
    }
    connection = RpcConnection.reconnecting;
    error = const RpcException('connection_failed', 'connection_failed');
    notifyListeners();
    _retry ??= Timer(retryDelay, () {
      _retry = null;
      unawaited(_open(generation));
    });
  }

  void _receive(dynamic raw, int generation) {
    if (_disposed || generation != _generation || raw is! String) return;
    try {
      for (final line in const LineSplitter().convert(raw)) {
        final decoded = jsonDecode(line);
        if (decoded is! Map || decoded['id'] is! int) continue;
        final frame = Map<String, dynamic>.from(decoded);
        final id = frame['id'] as int;
        final watch = _watchIds[id];
        if (watch != null) {
          if (frame.containsKey('item')) {
            watch.controller.add(frame['item']);
          } else if (frame.containsKey('err') || frame['done'] == true) {
            _watchIds.remove(id);
            watch.id = null;
            if (frame.containsKey('err')) {
              watch.controller.addError(_rpcError(frame['err']));
            }
            // A closed host stream (for example after a host reconnect) gets
            // a new read subscription, never a replayed mutation.
            watch.retry?.cancel();
            watch.retry = Timer(retryDelay, () => _subscribe(watch));
          }
          continue;
        }
        final pending = _pending.remove(id);
        if (pending == null || pending.isCompleted) continue;
        if (frame.containsKey('err')) {
          pending.completeError(_rpcError(frame['err']));
        } else if (frame.containsKey('ok')) {
          pending.complete(frame['ok']);
        }
      }
    } on FormatException {
      _lost(generation);
    }
  }

  RpcException _rpcError(dynamic raw) {
    final code = raw is Map
        ? '${raw['kind'] ?? raw['code'] ?? 'failed'}'
        : 'failed';
    var message = raw is Map
        ? '${raw['message'] ?? raw['reason'] ?? 'operation_failed'}'
        : '$raw';
    // The authenticated host is trusted, but credentials never belong in UI
    // errors or diagnostics, even if a server echoes a malformed request.
    if (_key case final key?) message = message.replaceAll(key, '[redacted]');
    return RpcException(code, message);
  }

  Future<dynamic> call(String method, [JsonMap params = const {}]) async {
    if (!connected || _socket == null) {
      throw const RpcException('disconnected', 'server_disconnected');
    }
    final id = ++_sequence;
    final pending = Completer<dynamic>();
    _pending[id] = pending;
    try {
      _socket!.add(jsonEncode({'id': id, 'method': method, 'params': params}));
      return await pending.future.timeout(
        callTimeout,
        onTimeout: () {
          throw const RpcException('unconfirmed', 'operation_unconfirmed');
        },
      );
    } finally {
      _pending.remove(id);
    }
  }

  Stream<dynamic> watch(String method, [JsonMap params = const {}]) {
    late final _RpcWatch watch;
    final controller = StreamController<dynamic>();
    watch = _RpcWatch(method, Map.of(params), controller);
    controller.onListen = () {
      _watches.add(watch);
      _subscribe(watch);
    };
    controller.onCancel = () {
      watch.retry?.cancel();
      _watches.remove(watch);
      final id = watch.id;
      if (id != null) {
        _watchIds.remove(id);
        _socket?.add(jsonEncode({'id': id, 'cancel': true}));
      }
    };
    return controller.stream;
  }

  void _subscribe(_RpcWatch watch) {
    if (_disposed ||
        !connected ||
        !_watches.contains(watch) ||
        watch.id != null) {
      return;
    }
    watch.retry?.cancel();
    final id = ++_sequence;
    watch.id = id;
    _watchIds[id] = watch;
    _socket!.add(
      jsonEncode({'id': id, 'method': watch.method, 'params': watch.params}),
    );
  }

  Future<void> reconnect() async {
    if (_key == null || host == null || _disposed) return;
    final generation = ++_generation;
    connection = RpcConnection.reconnecting;
    await _closeSocket();
    if (_disposed || generation != _generation) return;
    connection = RpcConnection.reconnecting;
    notifyListeners();
    await _open(generation);
  }

  Future<void> disconnect() async {
    ++_generation;
    connection = RpcConnection.disconnected;
    _key = null;
    await _closeSocket();
    connection = RpcConnection.disconnected;
    error = null;
    if (!_disposed) notifyListeners();
  }

  Future<void> _closeSocket() async {
    _retry?.cancel();
    _retry = null;
    final events = _socketEvents;
    final socket = _socket;
    _socketEvents = null;
    _socket = null;
    for (final pending in _pending.values) {
      if (!pending.isCompleted) {
        pending.completeError(
          const RpcException('disconnected', 'server_disconnected'),
        );
      }
    }
    _pending.clear();
    _watchIds.clear();
    for (final watch in _watches) {
      watch.id = null;
      watch.retry?.cancel();
    }
    await events?.cancel();
    await socket?.close();
  }

  @override
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    unawaited(disconnect());
    for (final watch in _watches.toList()) {
      unawaited(watch.controller.close());
    }
    _watches.clear();
    super.dispose();
  }
}

class _RpcWatch {
  _RpcWatch(this.method, this.params, this.controller);
  final String method;
  final JsonMap params;
  final StreamController<dynamic> controller;
  int? id;
  Timer? retry;
}
