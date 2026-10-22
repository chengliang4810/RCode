import 'package:flutter/material.dart';
import 'package:rcode_mobile/data/workspace_controller.dart';
import 'package:rcode_mobile/theme.dart';

class MemoryStore implements ConnectionStore {
  final values = <String, String>{};
  @override
  Future<String?> read(String key) async => values[key];
  @override
  Future<void> write(String key, String? value) async {
    if (value == null) {
      values.remove(key);
    } else {
      values[key] = value;
    }
  }
}

class PageWorkspace extends WorkspaceController {
  PageWorkspace() : super(store: MemoryStore(), cacheEnabled: false) {
    host = 'ws://example.test:27655/';
    deviceId = 'a';
    clientDeviceId = 'phone';
    devices = [
      {'id': 'a', 'name': 'Alpha', 'platform': 'linux'},
      {'id': 'b', 'name': 'Beta', 'platform': 'windows'},
    ];
  }
  final calls = <({String method, JsonMap params})>[];
  final connections = <({String host, String key})>[];
  Future<dynamic> Function(String, JsonMap)? handler;
  @override
  bool get connected => true;
  @override
  RpcConnection get connection => RpcConnection.connected;
  @override
  Future<void> reconnect() async {}
  @override
  Future<void> connect(
    String host,
    String apiKey, {
    bool restoring = false,
  }) async {
    connections.add((host: host, key: apiKey));
  }

  @override
  Future<dynamic> call(String method, [JsonMap params = const {}]) async {
    calls.add((method: method, params: Map.from(params)));
    if (handler != null) return handler!(method, params);
    if (method == 'ListHarnesses')
      return [
        {'id': 'native', 'name': 'Native', 'installed': true, 'enabled': true},
      ];
    if (method == 'ListModels')
      return [
        {
          'id': 'native-model',
          'label': 'Fixture Model',
          'reasoningLevels': ['low', 'medium', 'high'],
        },
      ];
    if (method == 'ListFolders') return {'path': '/workspace', 'entries': []};
    if (method == 'Mutate' && params['op'] == 'createChat') {
      chats.add({
        'id': params['chatId'],
        'deviceId': params['deviceId'] ?? 'a',
        'config': params['config'],
        'cwd': '/workspace',
        'archived': false,
      });
      notifyListeners();
    }
    return {'ok': true, 'disposition': 'executed'};
  }
}

Widget app(Widget child) =>
    MaterialApp(theme: rcodeTheme(Brightness.light), home: child);
