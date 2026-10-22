import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'package:path_provider/path_provider.dart';

import 'rpc_client.dart' show JsonMap;

abstract interface class SessionCache {
  SessionCacheEntry open(String scope, String chatId);
  Future<void> clearHost(String scope);
}

abstract interface class SessionCacheEntry {
  Future<JsonMap?> read();
  Future<void> write(JsonMap state);
}

String _encode(JsonMap value) => jsonEncode(value);
JsonMap? _decode(String text) {
  final value = jsonDecode(text);
  return value is Map ? Map<String, dynamic>.from(value) : null;
}

/// Private, full-history snapshots. Reads, atomic replacements and removal are
/// ordered; clearing a host also revokes every previously opened writer.
class FileSessionCache implements SessionCache {
  FileSessionCache({
    Future<Directory> Function()? directoryProvider,
    Future<String> Function(JsonMap)? encoder,
  }) : _directoryProvider = directoryProvider ?? _defaultDirectory,
       _encoder = encoder ?? ((state) => compute(_encode, state));

  final Future<Directory> Function() _directoryProvider;
  final Future<String> Function(JsonMap) _encoder;
  final Map<String, int> _epochs = {};
  Future<Directory>? _directory;
  Future<void> _operations = Future.value();

  static Future<Directory> _defaultDirectory() async {
    final support = await getApplicationSupportDirectory();
    return Directory('${support.path}/session-cache');
  }

  String _hash(String value) => sha256.convert(utf8.encode(value)).toString();
  Future<Directory> _host(String scope) async {
    final directory = await (_directory ??= _directoryProvider());
    return Directory('${directory.path}/${_hash(scope)}');
  }

  Future<T> _serial<T>(Future<T> Function() action) {
    final next = _operations.then((_) => action());
    _operations = next.then<void>((_) {}, onError: (Object _, StackTrace _) {});
    return next;
  }

  @override
  SessionCacheEntry open(String scope, String chatId) =>
      _FileEntry(this, scope, chatId, _epochs[scope] ?? 0);

  @override
  Future<void> clearHost(String scope) {
    _epochs.update(scope, (epoch) => epoch + 1, ifAbsent: () => 1);
    return _serial(() async {
      final directory = await _host(scope);
      if (await directory.exists()) await directory.delete(recursive: true);
    });
  }
}

class _FileEntry implements SessionCacheEntry {
  _FileEntry(this.cache, this.scope, this.chatId, this.epoch);
  final FileSessionCache cache;
  final String scope, chatId;
  final int epoch;
  bool get valid => epoch == (cache._epochs[scope] ?? 0);
  Future<File> _file() async =>
      File('${(await cache._host(scope)).path}/${cache._hash(chatId)}.json');

  @override
  Future<JsonMap?> read() => cache._serial(() async {
    if (!valid) return null;
    final file = await _file();
    if (!await file.exists()) return null;
    try {
      final envelope = await compute(_decode, await file.readAsString());
      if (!valid ||
          envelope?['version'] != 1 ||
          envelope?['scope'] != scope ||
          envelope?['chatId'] != chatId) {
        return null;
      }
      final state = envelope?['state'];
      return state is Map ? Map<String, dynamic>.from(state) : null;
    } on FormatException {
      return null;
    }
  });

  @override
  Future<void> write(JsonMap state) => cache._serial(() async {
    if (!valid) return;
    final encoded = await cache._encoder({
      'version': 1,
      'scope': scope,
      'chatId': chatId,
      'state': state,
    });
    if (!valid) return;
    final file = await _file();
    if (!valid) return;
    await file.parent.create(recursive: true);
    final temporary = File('${file.path}.tmp');
    await temporary.writeAsString(encoded, flush: true);
    if (!valid) {
      await temporary.delete();
      return;
    }
    await temporary.rename(file.path);
  });
}
