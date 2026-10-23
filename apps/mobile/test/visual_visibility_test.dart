import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:rcode_mobile/visual/rcode_visual.dart';
import 'package:rcode_mobile/visual/visual_backend.dart';

import 'support/page_workspace.dart';

class _Backend implements VisualBackend {
  final commands = <String>[];
  int polls = 0;
  @override
  Map<String, dynamic>? poll() {
    polls++;
    return polls == 1
        ? {
            'display': {
              'height': 72,
              'paint': {},
              'styles': [],
              'surfaces': [],
              'controls': [],
            },
          }
        : null;
  }

  @override
  void command(Map<String, dynamic> value) =>
      commands.add(value['op'] as String);
  @override
  Uint8List takeImage(int token) => Uint8List(0);
  @override
  void dispose() {}
}

void main() {
  testWidgets(
    'covered native visuals detach and reduce polling until revealed',
    (tester) async {
      final backend = _Backend();
      late BuildContext pageContext;
      await tester.pumpWidget(
        app(
          Builder(
            builder: (context) {
              pageContext = context;
              return Scaffold(
                body: RCodeVisual(
                  endpoint: 'ws://fixture',
                  apiKey: 'Fixture123',
                  deviceId: 'test',
                  path: 'test.lua',
                  backendFactory: (_) async => backend,
                ),
              );
            },
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.pump(const Duration(milliseconds: 250));
      Navigator.of(pageContext).push(
        MaterialPageRoute<void>(
          builder: (_) => const Scaffold(body: Text('Cover')),
        ),
      );
      await tester.pumpAndSettle();
      expect(
        backend.commands.where((op) => op == 'attach' || op == 'detach').last,
        'detach',
      );
      final hiddenPolls = backend.polls;
      await tester.pump(const Duration(seconds: 1));
      expect(backend.polls - hiddenPolls, lessThanOrEqualTo(4));
      Navigator.of(pageContext).pop();
      await tester.pumpAndSettle();
      expect(
        backend.commands.where((op) => op == 'attach' || op == 'detach').last,
        'attach',
      );
      final shownPolls = backend.polls;
      await tester.pump(const Duration(milliseconds: 250));
      expect(backend.polls - shownPolls, greaterThan(10));
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );
}
