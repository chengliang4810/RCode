import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rcode_mobile/preferences.dart';
import 'package:rcode_mobile/theme.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  test(
    'appearance and drafts survive restart without storing credentials',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await AppPreferences.load();
      await prefs.setValue('themeMode', ThemeMode.dark);
      await prefs.setValue('mathSize', 1.35);
      await prefs.setJson('draft.host', {
        'text': 'unsent',
        'model': 'model-id',
      });
      final restored = await AppPreferences.load();
      expect(restored.themeMode, ThemeMode.dark);
      expect(restored.mathSize, 1.35);
      expect(restored.getJson('draft.host')?['text'], 'unsent');
      expect(
        (await SharedPreferences.getInstance()).getKeys().any(
          (key) => key.contains('apiKey'),
        ),
        false,
      );
    },
  );
  test('theme matches the source iOS palette in both modes', () {
    expect(RCodeColors.light.background, const Color(0xfff3f3f5));
    expect(RCodeColors.dark.background, const Color(0xff060606));
    expect(RCodeColors.light.accent, const Color(0xff5b43e8));
    expect(RCodeColors.dark.accent, const Color(0xff8b7cf6));
    expect(
      rcodeTheme(Brightness.dark).textTheme.bodyMedium?.fontFamily,
      'Geist',
    );
  });
}
