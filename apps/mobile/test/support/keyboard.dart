import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

Future<void> pressShortcut(
  WidgetTester tester,
  LogicalKeyboardKey key, {
  bool control = false,
  bool shift = false,
}) async {
  final modifier = control
      ? LogicalKeyboardKey.controlLeft
      : LogicalKeyboardKey.metaLeft;
  await tester.sendKeyDownEvent(modifier);
  if (shift) await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyEvent(key);
  if (shift) await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyUpEvent(modifier);
}
