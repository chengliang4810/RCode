import 'dart:async';
import 'dart:convert';

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:uuid/uuid.dart';

import '../preferences.dart' show AppPreferences;
import 'rpc_client.dart';
import 'session_controller.dart';
import 'session_cache.dart';

export 'rpc_client.dart'
    show
        JsonMap,
        RpcClient,
        RpcConnection,
        RpcException,
        normalizeServerUrl,
        validApiKey;
export 'session_controller.dart' show SessionController, UploadAttachment;
export 'session_cache.dart'
    show SessionCache, SessionCacheEntry, FileSessionCache;

String _encodeWorkspace(JsonMap value) => jsonEncode(value);
JsonMap _decodeWorkspace(String value) => jsonMap(jsonDecode(value));

abstract interface class ConnectionStore {
  Future<String?> read(String key);
  Future<void> write(String key, String? value);
}

class SecureConnectionStore implements ConnectionStore {
  const SecureConnectionStore();
  static const _storage = FlutterSecureStorage();
  @override
  Future<String?> read(String key) => _storage.read(key: key);
  @override
  Future<void> write(String key, String? value) => value == null
      ? _storage.delete(key: key)
      : _storage.write(key: key, value: value);
}

/// The shared application model consumed by both phone and tablet pages.
class WorkspaceController extends ChangeNotifier {
  WorkspaceController({
    RpcClient? rpc,
    ConnectionStore? store,
    SessionCache? sessionCache,
    this.cacheEnabled = true,
  }) : rpc = rpc ?? RpcClient(),
       store = store ?? const SecureConnectionStore(),
       sessionCache = sessionCache ?? FileSessionCache() {
    this.rpc.addListener(_connectionChanged);
  }

  final RpcClient rpc;
  final ConnectionStore store;
  final bool cacheEnabled;
  final SessionCache sessionCache;
  List<JsonMap> devices = [];
  List<JsonMap> spaces = [];
  List<JsonMap> chats = [];
  List<JsonMap> sessions = [];
  JsonMap sidebarPreferences = {};
  String? host;
  String deviceId = '';
  String clientDeviceId = '';
  Object? error;
  bool loading = false;
  bool _paired = false;
  bool _disposed = false;
  int _generation = 0;
  final List<StreamSubscription<dynamic>> _subscriptions = [];
  final Map<String, SessionController> _sessionControllers = {};
  final Map<String, int> _sessionViews = {};
  final Map<String, int> _sessionTouches = {};
  final Map<String, VoidCallback> _sessionListeners = {};
  int _sessionClock = 0;
  bool _warmEvictionScheduled = false;
  Timer? _cacheDebounce;
  Future<void> _credentialWrites = Future.value();
  Future<void> _workspaceCacheWrites = Future.value();

  RpcConnection get connection => rpc.connection;
  bool get connected => rpc.connected;
  bool get authenticated => _paired;
  bool get initialized => clientDeviceId.isNotEmpty;
  String get cacheScope => sha256.convert(utf8.encode(host ?? '')).toString();

  Future<void> initialize() async {
    try {
      await _initialize();
    } catch (_) {
      if (_disposed) return;
      error = const RpcException(
        'secure_storage_unavailable',
        'secure_storage_unavailable',
      );
      notifyListeners();
    }
  }

  Future<void> _initialize() async {
    final generation = _generation;
    clientDeviceId =
        await store.read('rcode.mobile.device') ?? const Uuid().v4();
    if (_disposed || generation != _generation) return;
    await store.write('rcode.mobile.device', clientDeviceId);
    final saved = await store.read('rcode.mobile.host');
    if (_disposed || generation != _generation || saved == null) return;
    final normalized = normalizeServerUrl(saved);
    final key = await store.read(
      'rcode.mobile.key.${sha256.convert(utf8.encode(normalized))}',
    );
    if (_disposed || generation != _generation || key == null) return;
    try {
      await connect(normalized, key, restoring: true);
    } catch (_) {
      /* Connection state exposes the failure. */
    }
  }

  Future<void> connect(
    String address,
    String apiKey, {
    bool restoring = false,
  }) async {
    final normalized = normalizeServerUrl(address);
    if (!validApiKey(apiKey)) {
      throw const RpcException('invalid_key', 'invalid_key');
    }
    final generation = ++_generation;
    loading = true;
    error = null;
    await _cancelSubscriptions();
    if (_disposed || generation != _generation) return;
    _disposeSessions();
    _clearWorkspace();
    _paired = restoring;
    host = normalized;
    clientDeviceId = clientDeviceId.isEmpty
        ? const Uuid().v4()
        : clientDeviceId;
    notifyListeners();
    try {
      if (cacheEnabled) await _loadCache(generation);
      if (_disposed || generation != _generation) return;
      await rpc.connect(normalized, apiKey);
      if (_disposed || generation != _generation) return;
      final info = jsonMap(await rpc.call('EngineInfo'));
      if (_disposed || generation != _generation) return;
      deviceId = '${info['deviceId'] ?? ''}';
      _paired = true;
      _watchWorkspace(generation);
      await _persistCredentials(generation, normalized, apiKey);
    } catch (failure) {
      if (_disposed || generation != _generation) return;
      error = failure;
      if (restoring) {
        // A previously verified pairing can show its cache while the socket
        // retries. Install standing reads even if the first dial was offline.
        _watchWorkspace(generation);
      } else {
        _paired = false;
        await rpc.disconnect();
      }
      rethrow;
    } finally {
      if (!_disposed && generation == _generation) {
        loading = false;
        notifyListeners();
      }
    }
  }

  void _watchWorkspace(int generation) {
    void watch(String method, void Function(dynamic) apply) {
      _subscriptions.add(
        rpc
            .watch(method)
            .listen(
              (value) {
                if (_disposed || generation != _generation) return;
                apply(value);
                error = null;
                notifyListeners();
                _scheduleCache();
              },
              onError: (Object failure) {
                if (_disposed || generation != _generation) return;
                error = failure;
                notifyListeners();
              },
            ),
      );
    }

    watch('WatchDevices', (value) => devices = jsonMaps(value));
    watch('WatchSpaces', (value) => spaces = jsonMaps(value));
    watch('WatchChats', (value) {
      final next = jsonMaps(value);
      final retained = next.map((row) => row['id']).toSet();
      final removed = chats
          .map((row) => row['id'])
          .whereType<String>()
          .where((id) => !retained.contains(id))
          .toList();
      chats = next;
      for (final id in removed) {
        unawaited(
          _removeDeletedSession(
            id,
            cacheScope,
            generation,
          ).catchError((Object _) {}),
        );
      }
    });
    watch('WatchSessions', (value) => sessions = jsonMaps(value));
    watch(
      'WatchSidebarPreferences',
      (value) => sidebarPreferences = jsonMap(value),
    );
  }

  void _connectionChanged() {
    if (_disposed) return;
    if (rpc.error != null) error = rpc.error;
    if (rpc.connected && !loading) {
      final generation = _generation;
      unawaited(
        rpc
            .call('EngineInfo')
            .then((info) {
              if (_disposed || generation != _generation) return;
              deviceId = '${jsonMap(info)['deviceId'] ?? ''}';
              error = null;
              notifyListeners();
            })
            .catchError((Object failure) {
              if (_disposed || generation != _generation) return;
              error = failure;
              notifyListeners();
            }),
      );
    }
    notifyListeners();
  }

  Future<void> reconnect() => rpc.reconnect();

  Future<void> disconnect({bool forget = true}) async {
    final generation = ++_generation;
    _cacheDebounce?.cancel();
    final previous = host;
    final previousScope = cacheScope;
    final cleanup = forget && previous != null
        ? Future.wait([
            if (cacheEnabled) sessionCache.clearHost(previousScope),
            if (cacheEnabled) _clearPreferenceCache(previousScope),
            AppPreferences.forgetHostDrafts(previousScope),
          ]).catchError((Object _) => <void>[])
        : Future<void>.value();
    host = null;
    _paired = false;
    loading = false;
    error = null;
    await _cancelSubscriptions();
    if (_disposed || generation != _generation) return;
    _disposeSessions();
    await rpc.disconnect();
    if (_disposed || generation != _generation) return;
    _clearWorkspace();
    await cleanup;
    if (forget) {
      await _persistCredentials(generation, previous, null);
    }
    if (!_disposed) notifyListeners();
  }

  Future<dynamic> call(String method, [JsonMap params = const {}]) =>
      rpc.call(method, params);

  Future<void> _persistCredentials(
    int generation,
    String? address,
    String? key,
  ) {
    final task = _credentialWrites.catchError((Object _) {}).then((_) async {
      if (_disposed || generation != _generation) return;
      await store.write('rcode.mobile.host', key == null ? null : address);
      if (_disposed || generation != _generation) return;
      if (address != null) {
        final scope = sha256.convert(utf8.encode(address)).toString();
        await store.write('rcode.mobile.key.$scope', key);
      }
      if (_disposed || generation != _generation) return;
      await store.write('rcode.mobile.device', clientDeviceId);
    });
    _credentialWrites = task;
    return task;
  }

  SessionController session(String chatId) {
    _sessionTouches[chatId] = ++_sessionClock;
    final existing = _sessionControllers[chatId];
    if (existing != null && !existing.disposed) return existing;
    final controller = SessionController(this, chatId);
    _sessionControllers[chatId] = controller;
    _sessionViews.remove(chatId);
    _sessionListeners[chatId] = _scheduleWarmEviction;
    controller.addListener(_scheduleWarmEviction);
    return controller;
  }

  SessionController retainSession(String chatId) {
    final controller = session(chatId);
    _sessionViews.update(chatId, (count) => count + 1, ifAbsent: () => 1);
    return controller;
  }

  void releaseSession(SessionController controller) {
    if (!identical(_sessionControllers[controller.chatId], controller)) return;
    final count = _sessionViews[controller.chatId] ?? 0;
    if (count > 1) {
      _sessionViews[controller.chatId] = count - 1;
    } else {
      _sessionViews.remove(controller.chatId);
      _scheduleWarmEviction();
    }
  }

  void _scheduleWarmEviction() {
    if (_disposed || _warmEvictionScheduled) return;
    _warmEvictionScheduled = true;
    scheduleMicrotask(() {
      _warmEvictionScheduled = false;
      if (_disposed) return;
      final quiet =
          _sessionControllers.values
              .where(
                (session) =>
                    (_sessionViews[session.chatId] ?? 0) == 0 &&
                    !session.needsWarmSubscription,
              )
              .toList()
            ..sort(
              (a, b) => (_sessionTouches[a.chatId] ?? 0).compareTo(
                _sessionTouches[b.chatId] ?? 0,
              ),
            );
      for (final session in quiet.take(
        quiet.length > 6 ? quiet.length - 6 : 0,
      )) {
        _removeSession(session);
      }
    });
  }

  void _removeSession(SessionController session) {
    final id = session.chatId;
    final listener = _sessionListeners.remove(id);
    if (listener != null) session.removeListener(listener);
    _sessionControllers.remove(id);
    _sessionViews.remove(id);
    _sessionTouches.remove(id);
    session.dispose();
  }

  void _disposeSessions() {
    for (final session in _sessionControllers.values.toList()) {
      _removeSession(session);
    }
  }

  JsonMap? chat(String id) => chats.where((row) => row['id'] == id).firstOrNull;
  JsonMap? sessionStatus(String id) =>
      sessions.where((row) => row['chatId'] == id).firstOrNull;
  JsonMap? device(String id) =>
      devices.where((row) => row['id'] == id).firstOrNull;

  Future<JsonMap> waitForChat(String id) async {
    if (chat(id) case final row?) return row;
    final ready = Completer<JsonMap>();
    void changed() {
      if (chat(id) case final row?) {
        if (!ready.isCompleted) ready.complete(row);
      }
    }

    addListener(changed);
    try {
      changed();
      return await ready.future.timeout(
        const Duration(seconds: 15),
        onTimeout: () {
          throw const RpcException('missing_chat', 'session_unavailable');
        },
      );
    } finally {
      removeListener(changed);
    }
  }

  Future<void> mutate(JsonMap params) async {
    await call('Mutate', params);
  }

  Future<void> renameChat(String id, String title) =>
      mutate({'op': 'renameChat', 'chatId': id, 'title': title});
  Future<void> archiveChat(String id, bool archived) =>
      mutate({'op': 'setChatArchived', 'chatId': id, 'archived': archived});
  Future<void> deleteChat(String id) async {
    final scope = cacheScope;
    final generation = _generation;
    await mutate({'op': 'deleteChat', 'chatId': id});
    await _removeDeletedSession(id, scope, generation);
  }

  Future<void> _removeDeletedSession(
    String id,
    String scope,
    int generation,
  ) async {
    if (generation == _generation) {
      final session = _sessionControllers[id];
      if (session != null) _removeSession(session);
    }
    if (cacheEnabled) await sessionCache.remove(scope, id);
  }

  Future<void> changeSidebar(JsonMap change) =>
      mutate({'op': 'changeSidebarPin', 'change': change});
  Future<void> markSeen(String id) =>
      mutate({'op': 'markChatSeen', 'chatId': id});

  Future<void> flushCaches() async {
    _cacheDebounce?.cancel();
    if (cacheEnabled && !_disposed && host != null) {
      _enqueueWorkspaceCache(_generation, cacheScope);
    }
    await Future.wait(
      _sessionControllers.values.map((session) => session.flushCache()),
    );
    await _workspaceCacheWrites;
  }

  Future<void> _clearPreferenceCache(String scope) {
    final task = _workspaceCacheWrites.then((_) async {
      final preferences = await SharedPreferences.getInstance();
      final keys = preferences
          .getKeys()
          .where(
            (key) =>
                key == 'workspace.$scope' || key.startsWith('session.$scope.'),
          )
          .toList();
      for (final key in keys) {
        await preferences.remove(key);
      }
    });
    _workspaceCacheWrites = task.catchError((Object _) {});
    return task;
  }

  Future<void> _cancelSubscriptions() async {
    final subscriptions = _subscriptions.toList();
    _subscriptions.clear();
    await Future.wait(
      subscriptions.map((subscription) => subscription.cancel()),
    );
  }

  void _clearWorkspace() {
    devices = [];
    spaces = [];
    chats = [];
    sessions = [];
    sidebarPreferences = {};
    deviceId = '';
  }

  Future<void> _loadCache(int generation) async {
    try {
      final preferences = await SharedPreferences.getInstance();
      final raw = preferences.getString('workspace.$cacheScope');
      if (_disposed || generation != _generation || raw == null) return;
      final saved = await compute(_decodeWorkspace, raw);
      if (_disposed || generation != _generation) return;
      devices = jsonMaps(saved['devices']);
      spaces = jsonMaps(saved['spaces']);
      chats = jsonMaps(saved['chats']);
      sessions = jsonMaps(saved['sessions']);
      // Cached presence never makes a disconnected server appear reachable.
      for (final row in devices) {
        row['lastSeenAt'] = null;
      }
      sidebarPreferences = jsonMap(saved['sidebarPreferences']);
    } catch (_) {
      /* An incomplete cache never prevents connection. */
    }
  }

  void _scheduleCache() {
    if (!cacheEnabled) return;
    _cacheDebounce?.cancel();
    final generation = _generation;
    final scope = cacheScope;
    _cacheDebounce = Timer(const Duration(milliseconds: 400), () {
      _enqueueWorkspaceCache(generation, scope);
    });
  }

  void _enqueueWorkspaceCache(int generation, String scope) {
    final task = _workspaceCacheWrites.then((_) async {
      if (_disposed || generation != _generation) return;
      final preferences = await SharedPreferences.getInstance();
      if (_disposed || generation != _generation) return;
      final content = await compute(_encodeWorkspace, <String, dynamic>{
        'devices': devices,
        'spaces': spaces,
        'chats': chats,
        'sessions': sessions,
        'sidebarPreferences': sidebarPreferences,
      });
      if (_disposed || generation != _generation) return;
      await preferences.setString('workspace.$scope', content);
    });
    _workspaceCacheWrites = task.catchError((Object _) {});
  }

  @override
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    ++_generation;
    _cacheDebounce?.cancel();
    rpc.removeListener(_connectionChanged);
    unawaited(_cancelSubscriptions());
    _disposeSessions();
    rpc.dispose();
    super.dispose();
  }
}

JsonMap jsonMap(dynamic value) =>
    value is Map ? Map<String, dynamic>.from(value) : {};
List<JsonMap> jsonMaps(dynamic value) => value is List
    ? value
          .whereType<Map>()
          .map((row) => Map<String, dynamic>.from(row))
          .toList()
    : [];
