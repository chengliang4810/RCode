import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:rcode_mobile/data/workspace_controller.dart';
import 'package:rcode_mobile/main.dart' as app;
import 'package:rcode_mobile/visual/native_math.dart';
import 'package:rcode_mobile/visual/rcode_visual.dart';

// Run only on a dedicated simulator/emulator or the isolated Android QA package
// against crates/engine/examples/mobile_smoke.rs. All storage, plugins, RPC and
// native rendering below use the shipping implementation; the fixture never
// calls a model provider.
const _endpoint = String.fromEnvironment(
  'RCODE_MOBILE_QA_URL',
  defaultValue: 'ws://127.0.0.1:27657',
);
const _fixtureKey = 'MobileFixture123';
const _chat = 'mobile-qa-chat';

String _text(JsonMap entry) =>
    jsonMaps(entry['parts'])
        .where((part) => part['kind'] == 'text')
        .map((part) => part['text'])
        .join();

Future<void> _until(
  WidgetTester tester,
  bool Function() ready,
  String description, {
  Duration timeout = const Duration(seconds: 30),
}) async {
  final deadline = DateTime.now().add(timeout);
  while (!ready()) {
    if (DateTime.now().isAfter(deadline)) {
      fail('Timed out waiting for $description');
    }
    await tester.pump(const Duration(milliseconds: 100));
  }
  await tester.pump();
}

Iterable<VisualPainter> _paints(WidgetTester tester, {Finder? within}) => tester
    .widgetList<CustomPaint>(
      find.descendant(
        of: within ?? find.byType(RCodeVisual),
        matching: find.byType(CustomPaint),
      ),
    )
    .map((paint) => paint.painter)
    .whereType<VisualPainter>();

bool _painted(WidgetTester tester, String text) =>
    _paints(tester)
        .any((paint) => '${paint.display['text'] ?? ''}'.contains(text));

Future<void> _settleText(WidgetTester tester, Finder text) async {
  final editable = find.descendant(
    of: text,
    matching: find.byType(EditableText),
  );
  final deadline = DateTime.now().add(const Duration(seconds: 5));
  var stableFrames = 0;
  // These short fixture paragraphs fit their own SelectableText viewport.
  // Its scroll correction can outlive the surrounding transcript's layout.
  while (stableFrames < 3) {
    if (DateTime.now().isAfter(deadline)) {
      fail('Timed out waiting for the visible text scroll correction');
    }
    await tester.pump(const Duration(milliseconds: 100));
    final states = tester.stateList<EditableTextState>(editable);
    stableFrames =
        states.isNotEmpty &&
            states.every(
              (state) => state.renderEditable.offset.pixels.abs() < .01,
            )
        ? stableFrames + 1
        : 0;
  }
}

Future<void> _send(WidgetTester tester, String prompt) async {
  final composer = find.byKey(const Key('composer-text'));
  await tester.ensureVisible(composer);
  await tester.enterText(composer, prompt);
  await tester.pump();
  await tester.tap(find.byKey(const Key('composer-send')));
  await tester.pump();
}

void main() {
  final binding = IntegrationTestWidgetsFlutterBinding.ensureInitialized()
    // Native workers, scroll anchoring and screenshot presentation request
    // frames independently of tester.pump, just as in the shipping app.
    ..framePolicy = LiveTestWidgetsFlutterBindingFramePolicy.fullyLive;

  testWidgets(
    'shipping app pairs, renders native content and restores an offline turn',
    (tester) async {
      final checks = <String>[];
      binding.reportData = {
        'status': 'running',
        'platform': Platform.operatingSystem,
        'checks': checks,
      };
      WorkspaceController? workspace;
      final semantics = tester.ensureSemantics();
      // Keep text entry deterministic while retaining all other real platform
      // plugins. The emulator IME can otherwise echo its previous editing state
      // after a failed pairing disables and then re-enables the key field.
      binding.testTextInput.register();
      addTearDown(binding.testTextInput.unregister);
      var screenshotSurfaceReady = false;
      var passed = false;

      Future<void> screenshot(String name) async {
        if (Platform.isAndroid && !screenshotSurfaceReady) {
          await binding.convertFlutterSurfaceToImage();
          await tester.pump();
          screenshotSurfaceReady = true;
        }
        // State notifications and native display decoding can finish between
        // frames. Capture the composed frame after the visible-content checks.
        await tester.pump(const Duration(milliseconds: 100));
        await tester.pump();
        await binding.takeScreenshot(name);
      }

      try {
        await app.main();
        await tester.pump();
        workspace = tester
            .widget<app.RCodeApp>(find.byType(app.RCodeApp))
            .workspace;
        await _until(
          tester,
          () => workspace!.initialized,
          'secure storage startup',
        );
        expect(
          find.byKey(const Key('pair-host')),
          findsOneWidget,
          reason:
              'Use a fresh, dedicated simulator or clear this test app first',
        );

        await tester.enterText(find.byKey(const Key('pair-host')), _endpoint);
        await tester.enterText(
          find.byKey(const Key('pair-token')),
          'WrongFixture123',
        );
        await tester.tap(find.byKey(const Key('pair')));
        await _until(
          tester,
          () => find.byKey(const Key('connection-error')).evaluate().isNotEmpty,
          'invalid key rejection',
        );
        expect(workspace.authenticated, isFalse);
        checks.add(
          'invalid API key rejected by the real authenticated listener',
        );

        await tester.enterText(
          find.byKey(const Key('pair-token')),
          _fixtureKey,
        );
        await tester.pump();
        expect(
          tester
              .widget<TextFormField>(find.byKey(const Key('pair-token')))
              .controller!
              .text,
          _fixtureKey,
        );
        expect(
          tester.widget<FilledButton>(find.byKey(const Key('pair'))).onPressed,
          isNotNull,
        );
        await tester.tap(find.byKey(const Key('pair')));
        await _until(
          tester,
          () =>
              workspace!.authenticated &&
              workspace.chats.any((chat) => chat['id'] == _chat),
          'authenticated workspace',
        );
        expect(
          await workspace.store.read('rcode.mobile.host'),
          normalizeServerUrl(_endpoint),
        );
        checks.add(
          'valid pairing persisted through the native secure storage plugin',
        );
        final preferences = tester
            .widget<app.RCodeApp>(find.byType(app.RCodeApp))
            .preferences;
        await preferences.setValue('language', 'en');
        await tester.pump();
        await screenshot('01-authenticated-workspace');

        await tester.tap(find.text('Mobile QA · real engine').first);
        final session = workspace.session(_chat);
        await _until(
          tester,
          () =>
              session.hydrated &&
              find.byType(RCodeVisual).evaluate().isNotEmpty,
          'native visual in the real transcript',
        );
        await tester.ensureVisible(find.byType(RCodeVisual).first);
        await _until(
          tester,
          () => _painted(tester, 'Count 0'),
          'native Luau display',
        );
        await _until(
          tester,
          () => find
              .descendant(
                of: find.byType(RCodeMath),
                matching: find.byType(SvgPicture),
              )
              .evaluate()
              .isNotEmpty,
          'math SVG produced by the native math renderer',
        );
        await _until(
          tester,
          () => _paints(tester).any(
            (paint) => (paint.display['shapes'] as List? ?? []).any(
              (shape) => shape['kind'] == 'path' && shape['closed'] == true,
            ),
          ),
          'native canvas path',
        );
        final images = find.descendant(
          of: find.byType(RCodeVisual),
          matching: find.byType(RawImage),
        );
        await _until(
          tester,
          () =>
              tester
                  .widgetList<RawImage>(images)
                  .where((image) => image.image != null)
                  .length >=
              2,
          'native scene and shader pixel surfaces',
          timeout: const Duration(seconds: 60),
        );
        // Keep our own handles while rendering replaces the live images. The
        // 120-point scene is taller than the 48-point shader in this fixture.
        final rendered =
            tester
                .widgetList<RawImage>(images)
                .map((image) => image.image)
                .whereType<ui.Image>()
                .map((image) => image.clone())
                .toList()
              ..sort((a, b) => a.height.compareTo(b.height));
        var shaderPixels = false;
        try {
          expect(rendered.length, 2);
          expect(rendered.last.height, greaterThan(rendered.first.height));
          final pixels = await rendered.first.toByteData(
            format: ui.ImageByteFormat.rawRgba,
          );
          expect(pixels, isNotNull);
          final bytes = pixels!.buffer.asUint8List(
            pixels.offsetInBytes,
            pixels.lengthInBytes,
          );
          for (var offset = 0; offset + 3 < bytes.length; offset += 4) {
            if ((bytes[offset] - 64).abs() <= 2 &&
                (bytes[offset + 1] - 128).abs() <= 2 &&
                bytes[offset + 2] >= 253 &&
                bytes[offset + 3] >= 253) {
              shaderPixels = true;
              break;
            }
          }
          final scene = await rendered.last.toByteData(
            format: ui.ImageByteFormat.rawRgba,
          );
          expect(scene, isNotNull);
          final sceneBytes = scene!.buffer.asUint8List(
            scene.offsetInBytes,
            scene.lengthInBytes,
          );
          final colors = <int>{};
          var opaque = 0;
          for (var offset = 0; offset + 3 < sceneBytes.length; offset += 4) {
            if (sceneBytes[offset + 3] < 200) continue;
            opaque++;
            colors.add(
              (sceneBytes[offset] << 16) |
                  (sceneBytes[offset + 1] << 8) |
                  sceneBytes[offset + 2],
            );
          }
          expect(
            opaque,
            greaterThan(rendered.last.width * rendered.last.height ~/ 100),
          );
          expect(
            colors.length,
            greaterThan(3),
            reason: 'The scene must contain shaded geometry',
          );
        } finally {
          for (final image in rendered) {
            image.dispose();
          }
        }
        expect(
          shaderPixels,
          isTrue,
          reason: 'The GLSL fixture must render actual blue pixels',
        );
        checks.add(
          'native math SVG, Luau display, canvas path, shaded scene and blue shader pixels',
        );
        await screenshot('02-native-renderers');
        await tester.ensureVisible(find.bySemanticsLabel('Add').first);
        await tester.tap(find.bySemanticsLabel('Add').first);
        await _until(
          tester,
          () => _painted(tester, 'Count 1'),
          'native control state change',
        );
        checks.add('native Luau Add control changes Count 0 to Count 1');
        await screenshot('03-native-control');

        final beforeQuestion = session.messages
            .map((entry) => entry['id'])
            .toSet();
        await _send(tester, 'question');
        await _until(
          tester,
          () =>
              session.openInput != null &&
              find.text('Beta').evaluate().isNotEmpty,
          'typed user question',
        );
        await tester.tap(find.text('Beta'));
        await tester.pump(const Duration(milliseconds: 500));
        // Single-choice selection normally advances automatically. Respect the
        // platform's reduced-motion setting, which leaves submission explicit.
        if (session.openInput != null &&
            find.byKey(const Key('question-next')).evaluate().isNotEmpty) {
          await tester.tap(find.byKey(const Key('question-next')));
        }
        await _until(
          tester,
          () =>
              session.openInput == null &&
              !session.working &&
              session.messages.any(
                (entry) =>
                    !beforeQuestion.contains(entry['id']) &&
                    _text(entry).contains('Typed answer received:'),
              ),
          'accepted typed answer',
        );
        final answered = session.messages
            .where((entry) => !beforeQuestion.contains(entry['id']))
            .map(_text)
            .firstWhere((text) => text.contains('Typed answer received:'));
        final answers = jsonDecode(
          answered.split('Typed answer received: ').last.split('\n').first,
        ) as List;
        expect(answers.single['questionId'], 'qa-choice');
        expect(answers.single['selectedOptionIds'], ['Beta']);
        checks.add(
          'structured user question answered using the shipping controls',
        );

        final completedBefore = session.messages
            .where(
              (entry) => _text(entry).contains('Fixture response completed.'),
            )
            .length;
        final usersBefore = session.messages
            .where((entry) => entry['role'] == 'user')
            .length;
        await _send(tester, 'slow offline integration');
        await _until(
          tester,
          () =>
              session.working &&
              session.messages.any(
                (entry) => _text(
                  entry,
                ).contains('Mobile QA mock received: slow offline integration'),
              ),
          'server-accepted slow turn',
        );
        final acceptedEntry = session.messages.lastWhere(
          (entry) => _text(entry)
              .contains('Mobile QA mock received: slow offline integration'),
        );
        final acceptedRow = find.byKey(ValueKey('${acceptedEntry['id']}'));
        final acceptedText = find.descendant(
          of: acceptedRow,
          matching: find.textContaining(
            'Mobile QA mock received: slow offline integration',
            findRichText: true,
          ),
        );
        await _until(
          tester,
          () => acceptedText.evaluate().isNotEmpty,
          'accepted slow turn in the transcript',
        );
        await tester.ensureVisible(acceptedText.last);
        await _until(
          tester,
          () => acceptedText.hitTestable().evaluate().isNotEmpty,
          'visible accepted slow turn',
        );
        await _settleText(tester, acceptedRow);
        await screenshot('04-accepted-slow-turn');
        expect(
          session.working,
          isTrue,
          reason: 'Disconnect while the accepted turn is still running',
        );
        expect(
          session.messages
              .where(
                (entry) => _text(entry).contains('Fixture response completed.'),
              )
              .length,
          completedBefore,
        );
        await workspace.flushCaches();
        await tester.pumpWidget(const SizedBox.shrink());
        await workspace.disconnect(forget: false);
        workspace.dispose();
        workspace = null;
        expect(session.disposed, isTrue);
        // All app subscriptions and its WebSocket are now closed. The engine must
        // finish the accepted turn before a new app model restores its Keychain /
        // Keystore credentials and reads the authoritative result.
        await tester.pump(const Duration(seconds: 16));
        await app.main();
        await tester.pump();
        workspace = tester
            .widget<app.RCodeApp>(find.byType(app.RCodeApp))
            .workspace;
        await _until(
          tester,
          () =>
              workspace!.authenticated &&
              workspace.connected &&
              workspace.chats.any((chat) => chat['id'] == _chat),
          'automatic secure credential restore',
        );
        expect(find.byKey(const Key('pair-token')), findsNothing);
        await tester.tap(find.text('Mobile QA · real engine').first);
        final restored = workspace.session(_chat);
        await _until(
          tester,
          () =>
              restored.hydrated &&
              !restored.working &&
              restored.messages
                      .where(
                        (entry) =>
                            _text(entry)
                                .contains('Fixture response completed.'),
                      )
                      .length ==
                  completedBefore + 1,
          'completed offline turn',
        );
        expect(
          restored.messages.where((entry) => entry['role'] == 'user').length,
          usersBefore + 1,
        );
        expect(
          restored.messages
              .where(
                (entry) =>
                    entry['role'] == 'user' &&
                    _text(entry) == 'slow offline integration',
              )
              .length,
          1,
        );
        expect(restored.pendingMessages, isEmpty);
        final completedEntry = restored.messages.lastWhere(
          (entry) => _text(entry).contains('Fixture response completed.'),
        );
        final completedRow = find.byKey(ValueKey('${completedEntry['id']}'));
        final restoredVisual = find.descendant(
          of: completedRow,
          matching: find.byType(RCodeVisual),
        );
        await _until(
          tester,
          () => restoredVisual.evaluate().isNotEmpty,
          'restored turn in the transcript',
        );
        await tester.ensureVisible(restoredVisual);
        await _until(
          tester,
          () =>
              _paints(tester, within: restoredVisual).any(
                (paint) => '${paint.display['text'] ?? ''}'.contains('Count 0'),
              ) &&
              tester
                      .widgetList<RawImage>(
                        find.descendant(
                          of: restoredVisual,
                          matching: find.byType(RawImage),
                        ),
                      )
                      .where((image) => image.image != null)
                      .length ==
                  2,
          'restored native display, scene and shader',
          timeout: const Duration(seconds: 60),
        );
        final completedText = find.descendant(
          of: completedRow,
          matching: find.textContaining(
            'Fixture response completed.',
            findRichText: true,
          ),
        );
        await _until(
          tester,
          () => completedText.evaluate().isNotEmpty,
          'restored completion text',
        );
        await tester.ensureVisible(completedText.last);
        await _until(
          tester,
          () => completedText.hitTestable().evaluate().isNotEmpty,
          'visible restored completion text',
        );
        await _settleText(tester, completedRow);
        checks.add(
          'accepted turn finishes offline and restores visible text and native content once without resending',
        );
        await screenshot('05-restored-offline-turn');
        passed = true;
      } catch (_) {
        binding.reportData!['failureState'] = {
          'authenticated': workspace?.authenticated,
          'connected': workspace?.connected,
          'loading': workspace?.loading,
          'chats': workspace?.chats.length,
          'errorType': workspace?.error.runtimeType.toString(),
          'errorCode': workspace?.error is RpcException
              ? (workspace!.error as RpcException).code
              : null,
        };
        rethrow;
      } finally {
        final runtimeError = tester.takeException();
        binding.reportData!['status'] = passed && runtimeError == null
            ? 'passed'
            : 'failed';
        await tester.pumpWidget(const SizedBox.shrink());
        workspace?.dispose();
        semantics.dispose();
        expect(
          runtimeError,
          isNull,
          reason: 'Flutter must not report a runtime exception',
        );
      }
    },
    timeout: const Timeout(Duration(minutes: 4)),
  );
}
