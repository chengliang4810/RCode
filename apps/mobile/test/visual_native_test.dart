import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';
import 'package:flutter_test/flutter_test.dart';

import 'package:rcode_mobile/visual/rcode_visual.dart';
import 'package:rcode_mobile/visual/visual_backend.dart';
import 'package:rcode_mobile/visual/visual_native.dart' as native;

void main() {
  test('visual faces preserve weight, family and italic style', () {
    final mono = visualTextStyle({'face': 'monoSemibold', 'size': 17});
    expect(mono.fontFamily, 'GeistMono');
    expect(mono.fontWeight, FontWeight.w600);
    expect(
      visualTextStyle({'ligatures': false}).fontFeatures!
          .map((feature) => feature.feature),
      ['liga', 'clig'],
    );
    expect(
      visualTextStyle({'face': 'sansBoldItalic', 'size': 17}).fontStyle,
      FontStyle.italic,
    );
  });

  testWidgets(
    'native visual reads the host script and handles controls after reattachment',
    (tester) async {
      await tester.runAsync(() async {
        final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
        final sockets = <WebSocket>[];
        var reads = 0;
        const script = '''
        local s=ui.state {n=0}
        return function() return ui.column {
          ui.text("Count "..s.n),
          ui.button {key="native-control",label="Add",on_click=function() s.n=s.n+1 end},
          ui.canvas {height=80,draw=function(g) g:rect(0,0,20,20,{fill="danger"}) end},
        } end
      ''';
        server.listen((request) async {
          expect(
            request.headers.value(HttpHeaders.authorizationHeader),
            'Bearer Fixture12345',
          );
          final socket = await WebSocketTransformer.upgrade(request);
          sockets.add(socket);
          socket.listen((message) {
            final frame = jsonDecode(message as String) as Map<String, dynamic>;
            if (frame['method'] == null) return;
            expect(frame['method'], 'ReadAttachmentChunk');
            expect(frame['params']['targetDeviceId'], 'host-device');
            final path = frame['params']['path'] as String;
            if (path.endsWith('-assets.json')) {
              socket.add(
                jsonEncode({
                  'id': frame['id'],
                  'ok': {'data': base64Encode(utf8.encode('[]')), 'done': true},
                }),
              );
            } else {
              reads++;
              socket.add(
                jsonEncode({
                  'id': frame['id'],
                  'ok': {
                    'data': base64Encode(utf8.encode(script)),
                    'done': true,
                  },
                }),
              );
            }
          });
        });
        VisualBackend? backend;
        try {
          backend = await native.openVisual({
            'endpoint': 'ws://127.0.0.1:${server.port}',
            'apiKey': 'Fixture12345',
            'deviceId': 'host-device',
            'path': 'uploads/fixture-visual.luau',
            'id': 'fixture',
            'title': 'Fixture',
            'width': 300,
            'textScale': 1,
            'pixelScale': 1,
            'dark': false,
            'reduceMotion': true,
          });
          final surfaces = <String, Map<String, dynamic>>{};
          Future<Map<String, dynamic>> display(
            String text, {
            double? width,
          }) async {
            final deadline = DateTime.now().add(const Duration(seconds: 10));
            Map<String, dynamic>? frame;
            while (DateTime.now().isBefore(deadline)) {
              final update = backend!.poll();
              for (final surface in (update?['frames'] as List? ?? [])) {
                final row = (surface as Map).cast<String, dynamic>();
                surfaces[row['surface'] as String] = row;
              }
              for (final request in (update?['measurements'] as List? ?? [])) {
                final painter = TextPainter(
                  text: TextSpan(
                    text: request['text'] as String,
                    style: visualTextStyle(
                      (request as Map).cast<String, dynamic>(),
                    ),
                  ),
                  textDirection: TextDirection.ltr,
                )..layout();
                final runes = (request['text'] as String).runes.length;
                backend.command({
                  'op': 'measured',
                  'id': request['id'],
                  'advances': List.filled(
                    runes,
                    runes == 0 ? 0 : painter.width / runes,
                  ),
                });
                painter.dispose();
              }
              frame = update?['display'] as Map<String, dynamic>? ?? frame;
              if (frame != null &&
                  (frame['paint']['text'] as String).contains(text) &&
                  (width == null || frame['width'] == width) &&
                  surfaces.isNotEmpty) {
                return frame;
              }
              await Future<void>.delayed(const Duration(milliseconds: 10));
            }
            fail(
              'The native display did not reach $text: '
              '${jsonEncode({'display': frame, 'surfaces': surfaces, 'reads': reads})}',
            );
          }

          final first = await display('Count 0');
          expect(first['surfaces'], isNotEmpty);
          final canvas = surfaces[first['surfaces'][0]['id']]!['paint'] as Map;
          expect(canvas['text'], isEmpty, reason: 'canvas draw has no error');
          final shape = (canvas['shapes'] as List).single as Map;
          expect(shape['kind'], 'path');
          expect(shape['closed'], isTrue);
          expect(shape['points'], [0, 0, 20, 0, 20, 20, 0, 20]);
          final fill = shape['fill'] as Map;
          expect(fill['r'], closeTo(220 / 255, 0.00001));
          expect(fill['g'], closeTo(38 / 255, 0.00001));
          expect(fill['b'], closeTo(38 / 255, 0.00001));
          expect(fill['a'], 1);
          final control = first['controls'][0]['id'];
          expect(control, isA<String>());
          expect(
            BigInt.parse(control as String),
            greaterThan(BigInt.from(0x7fffffffffffffff)),
          );
          backend.command({'op': 'activate', 'control': control});
          await display('Count 1');
          backend.command({'op': 'detach'});
          backend.command({'op': 'layout', 'width': 240, 'textScale': 1});
          backend.command({'op': 'attach', 'dark': true, 'reduceMotion': true});
          final restored = await display('Count 1', width: 240);
          expect(restored['dark'], isTrue);
          expect(
            reads,
            1,
            reason: 'reattachment preserves the live script and its state',
          );
        } finally {
          backend?.dispose();
          for (final socket in sockets) {
            await socket.close();
          }
          await server.close(force: true);
        }
      });
    },
    skip: Platform.environment['RCODE_NATIVE_VISUAL_TEST'] != '1',
  );

  testWidgets(
    'scoped rows preserve controls through actual widget recycling and release on exit',
    (tester) async {
      final host = (await tester.runAsync(_VisualHost.start))!;
      final semantics = tester.ensureSemantics();
      const scope = 'widget-host:widget-chat';
      Widget tree({required bool visible}) => MaterialApp(
        home: Scaffold(
          body: visible
              ? RCodeVisual(
                  scope: scope,
                  endpoint: host.endpoint,
                  apiKey: 'Fixture12345',
                  deviceId: 'host-device',
                  path: 'uploads/fixture-visual.luau',
                )
              : const SizedBox(),
        ),
      );
      bool containsText(String text) => tester
          .widgetList<CustomPaint>(find.byType(CustomPaint))
          .any(
            (paint) =>
                paint.painter is VisualPainter &&
                ((paint.painter as VisualPainter).display['text'] as String? ??
                        '')
                    .contains(text),
          );
      Future<void> waitFor(String text) async {
        for (var attempt = 0; attempt < 200; attempt++) {
          await tester.runAsync(
            () => Future<void>.delayed(const Duration(milliseconds: 10)),
          );
          await tester.pump(const Duration(milliseconds: 16));
          if (containsText(text)) return;
        }
        fail('Widget did not paint $text');
      }

      try {
        await tester.pumpWidget(tree(visible: true));
        await waitFor('Count 0');
        await tester.tap(find.bySemanticsLabel('Add'));
        await waitFor('Count 1');
        await tester.pumpWidget(tree(visible: false));
        expect(find.byType(RCodeVisual), findsNothing);
        await tester.pumpWidget(tree(visible: true));
        await waitFor('Count 1');
        await tester.tap(find.bySemanticsLabel('Add'));
        await waitFor('Count 2');
        expect(host.reads, 1);
        disposeVisualScope(scope);
        await tester.pumpWidget(tree(visible: false));
        await tester.pumpWidget(tree(visible: true));
        await waitFor('Count 0');
        expect(
          host.reads,
          2,
          reason: 'leaving the session releases its script',
        );
      } finally {
        disposeVisualScope(scope);
        await tester.pumpWidget(const SizedBox());
        semantics.dispose();
        await tester.runAsync(host.close);
      }
    },
    skip: Platform.environment['RCODE_NATIVE_VISUAL_TEST'] != '1',
  );

  testWidgets(
    'native status follows locale changes while preserving live script state',
    (tester) async {
      final host = (await tester.runAsync(_VisualHost.start))!;
      final semantics = tester.ensureSemantics();
      const scope = 'widget-host:locale-chat';
      Widget tree(Locale locale) => MaterialApp(
        locale: locale,
        supportedLocales: const [Locale('en'), Locale('zh')],
        localizationsDelegates: GlobalMaterialLocalizations.delegates,
        home: Scaffold(
          body: RCodeVisual(
            scope: scope,
            endpoint: host.endpoint,
            apiKey: 'Fixture12345',
            deviceId: 'host-device',
            path: 'uploads/fixture-visual.luau',
          ),
        ),
      );
      bool containsText(String text) => tester
          .widgetList<CustomPaint>(find.byType(CustomPaint))
          .any(
            (paint) =>
                paint.painter is VisualPainter &&
                ((paint.painter as VisualPainter).display['text'] as String? ??
                        '')
                    .contains(text),
          );
      Future<void> waitFor(String text) async {
        for (var attempt = 0; attempt < 200; attempt++) {
          await tester.runAsync(
            () => Future<void>.delayed(const Duration(milliseconds: 10)),
          );
          await tester.pump(const Duration(milliseconds: 16));
          if (containsText(text)) return;
        }
        fail('Widget did not paint $text');
      }

      try {
        await tester.pumpWidget(tree(const Locale('en')));
        await waitFor('Count 0');
        await tester.tap(find.bySemanticsLabel('Add'));
        await waitFor('Count 1');
        await tester.tap(find.bySemanticsLabel('Fail'));
        await waitFor('Visual error:');
        expect(containsText('original error detail'), isTrue);
        await tester.pumpWidget(tree(const Locale('zh')));
        await waitFor('可视化错误：');
        expect(containsText('original error detail'), isTrue);
        expect(containsText('Count 1'), isTrue);
        await tester.pumpWidget(tree(const Locale('en')));
        await waitFor('Visual error:');
        await tester.tap(find.bySemanticsLabel('Add'));
        await waitFor('Count 2');
        expect(
          host.reads,
          1,
          reason: 'locale changes must not restart the script',
        );
      } finally {
        disposeVisualScope(scope);
        await tester.pumpWidget(const SizedBox());
        semantics.dispose();
        await tester.runAsync(host.close);
      }
    },
    skip: Platform.environment['RCODE_NATIVE_VISUAL_TEST'] != '1',
  );

  testWidgets(
    'native math renders all six faces and rejects streaming fragments',
    (tester) async {
      await tester.runAsync(() async {
        final shapes = <String>{};
        for (final font in [
          'fira',
          'stixTwo',
          'computerModern',
          'libertinus',
          'pagella',
          'euler',
        ]) {
          final math = await native.renderNativeMath({
            'source': r'\frac{x^2}{2}',
            'font': font,
            'fontSize': 18,
            'display': true,
          });
          expect(math, isNotNull, reason: font);
          expect(math!['svg'], contains('<path'));
          expect(math['width'], greaterThan(0));
          shapes.add(math['svg'] as String);
        }
        expect(shapes.length, 6);
        expect(
          await native.renderNativeMath({'source': r'\frac{', 'fontSize': 18}),
          isNull,
        );
      });
    },
    skip: Platform.environment['RCODE_NATIVE_VISUAL_TEST'] != '1',
  );
}

class _VisualHost {
  _VisualHost(this.server);
  final HttpServer server;
  final sockets = <WebSocket>[];
  var reads = 0;
  String get endpoint => 'ws://127.0.0.1:${server.port}';

  static Future<_VisualHost> start() async {
    final host = _VisualHost(
      await HttpServer.bind(InternetAddress.loopbackIPv4, 0),
    );
    host.server.listen((request) async {
      expect(
        request.headers.value(HttpHeaders.authorizationHeader),
        'Bearer Fixture12345',
      );
      final socket = await WebSocketTransformer.upgrade(request);
      host.sockets.add(socket);
      socket.listen((message) {
        final frame = jsonDecode(message as String) as Map<String, dynamic>;
        if (frame['method'] == null) return;
        expect(frame['method'], 'ReadAttachmentChunk');
        expect(frame['params']['targetDeviceId'], 'host-device');
        final manifest = (frame['params']['path'] as String).endsWith(
          '-assets.json',
        );
        if (!manifest) host.reads++;
        const script = '''
          local s=ui.state {n=0}
          return function() return ui.column {
            ui.text("Count "..s.n),
            ui.button {key="native-control",label="Add",on_click=function() s.n=s.n+1 end},
            ui.button {label="Fail",on_click=function() error("original error detail") end},
          } end
        ''';
        socket.add(
          jsonEncode({
            'id': frame['id'],
            'ok': {
              'data': base64Encode(utf8.encode(manifest ? '[]' : script)),
              'done': true,
            },
          }),
        );
      });
    });
    return host;
  }

  Future<void> close() async {
    for (final socket in sockets) {
      await socket.close();
    }
    await server.close(force: true);
  }
}
