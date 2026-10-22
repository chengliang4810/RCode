import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// Non-secret presentation preferences; credentials belong in secure storage.
class AppPreferences extends ChangeNotifier {
  AppPreferences(this._storage);
  final SharedPreferences _storage;
  static Future<AppPreferences> load() async =>
      AppPreferences(await SharedPreferences.getInstance());
  static const _prefix = 'rcode.flutter.';

  ThemeMode get themeMode =>
      switch (_storage.getString('${_prefix}themeMode')) {
        'light' => ThemeMode.light,
        'dark' => ThemeMode.dark,
        _ => ThemeMode.system,
      };
  String get language => _storage.getString('${_prefix}language') ?? 'system';
  bool get groupByProject => _bool('groupByProject', false);
  bool get notifications => _bool('notifications', true);
  bool get notifyDone => _bool('notifyDone', true);
  bool get notifyInput => _bool('notifyInput', true);
  bool get notifyFailed => _bool('notifyFailed', true);
  bool get renderMath => _bool('renderMath', true);
  String? get wallpaperPath => _storage.getString('${_prefix}wallpaperPath');
  String get wallpaperEffect =>
      _storage.getString('${_prefix}wallpaperEffect') ?? 'none';
  String get mathFont => _storage.getString('${_prefix}mathFont') ?? 'fira';
  double get mathSize =>
      (_storage.getDouble('${_prefix}mathSize') ?? 1).clamp(.9, 1.5);
  bool _bool(String key, bool fallback) =>
      _storage.getBool('$_prefix$key') ?? fallback;

  Map<String, dynamic>? getJson(String key) {
    final value = _storage.getString('$_prefix$key');
    if (value == null) return null;
    try {
      return (jsonDecode(value) as Map).cast<String, dynamic>();
    } on FormatException {
      return null;
    } on TypeError {
      return null;
    }
  }

  Future<void> setJson(
    String key,
    Map<String, dynamic>? value, {
    bool notify = true,
  }) async {
    if (value == null) {
      await _storage.remove('$_prefix$key');
    } else {
      await _storage.setString('$_prefix$key', jsonEncode(value));
    }
    if (notify) notifyListeners();
  }

  Future<void> setValue(String key, dynamic value) async {
    final full = '$_prefix$key';
    if (value is ThemeMode) value = value.name;
    if (value == null) {
      await _storage.remove(full);
    } else if (value is bool) {
      await _storage.setBool(full, value);
    } else if (value is double) {
      await _storage.setDouble(full, value);
    } else if (value is int) {
      await _storage.setInt(full, value);
    } else if (value is String) {
      await _storage.setString(full, value);
    } else {
      throw ArgumentError.value(key, 'key', 'unsupported preference');
    }
    notifyListeners();
  }
}
