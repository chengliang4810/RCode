import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as image;
import 'package:image_picker/image_picker.dart';
import 'package:rcode_mobile/data/session_controller.dart';
import 'package:rcode_mobile/ui/attachments.dart';
import 'package:rcode_mobile/ui/image_clipboard.dart';

class _LostPicker extends ImagePicker {
  _LostPicker(this.response);
  final LostDataResponse response;
  int calls = 0;
  @override
  Future<LostDataResponse> retrieveLostData() async {
    calls++;
    return response;
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  const clipboard = MethodChannel('com.rcode.mobile/image_clipboard');
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
  final png = Uint8List.fromList(
    image.encodePng(image.Image(width: 32, height: 16)),
  );
  setUp(() => resetRecoveredImagesForTesting());
  tearDown(() {
    messenger.setMockMethodCallHandler(clipboard, null);
    debugDefaultTargetPlatformOverride = null;
    resetRecoveredImagesForTesting();
  });

  test(
    'clipboard byte bridge reads explicitly and bounds the native result',
    () async {
      int calls = 0;
      messenger.setMockMethodCallHandler(clipboard, (call) async {
        expect(call.method, 'readImage');
        calls++;
        return {'bytes': png, 'name': r'C:\untrusted\clipboard.png'};
      });
      expect(calls, 0);
      final result = await readClipboardImage();
      expect(calls, 1);
      expect(result!.bytes, png);
      expect(result.name, 'clipboard.png');
      messenger.setMockMethodCallHandler(clipboard, (_) async => null);
      expect(await readClipboardImage(), isNull);
      messenger.setMockMethodCallHandler(
        clipboard,
        (_) async => {'bytes': Uint8List(maximumAttachmentBytes + 1)},
      );
      await expectLater(
        readClipboardImage(),
        throwsA(
          isA<PlatformException>().having(
            (e) => e.code,
            'code',
            'image_too_large',
          ),
        ),
      );
    },
  );

  testWidgets(
    'attachment menu never probes clipboard and the chosen image is normalized',
    (tester) async {
      int reads = 0;
      messenger.setMockMethodCallHandler(clipboard, (_) async {
        reads++;
        return {'bytes': png, 'name': 'clipboard.png'};
      });
      late BuildContext composer;
      AttachmentSource? selected;
      await tester.pumpWidget(
        MaterialApp(
          home: Builder(
            builder: (context) {
              composer = context;
              return Scaffold(
                body: TextButton(
                  onPressed: () async {
                    selected = await showAttachmentMenu(context);
                  },
                  child: const Text('Attach'),
                ),
              );
            },
          ),
        ),
      );
      await tester.tap(find.text('Attach'));
      await tester.pumpAndSettle();
      expect(find.text('Paste image'), findsOneWidget);
      expect(reads, 0);
      await tester.tap(find.text('Paste image'));
      await tester.pumpAndSettle();
      expect(selected, AttachmentSource.clipboard);
      expect(reads, 0);
      final attachments = await tester.runAsync(
        () => chooseImages(composer, selected!),
      );
      expect(reads, 1);
      expect(attachments!.single.name, 'clipboard.jpg');
      expect(attachments.single.mimeType, 'image/jpeg');
      expect(image.decodeJpg(attachments.single.bytes)!.width, 32);
    },
  );

  testWidgets(
    'lost Android picks use the image conversion path and skip other platforms',
    (tester) async {
      late BuildContext context;
      await tester.pumpWidget(
        MaterialApp(
          home: Builder(
            builder: (value) {
              context = value;
              return const SizedBox();
            },
          ),
        ),
      );
      final picker = _LostPicker(
        LostDataResponse(
          type: RetrieveType.image,
          files: [XFile.fromData(png, path: 'recovered.png')],
        ),
      );
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      expect(await restoreLostImages(context, picker: picker), isEmpty);
      expect(picker.calls, 0);
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      final restored = await tester.runAsync(
        () => restoreLostImages(context, picker: picker),
      );
      expect(picker.calls, 1);
      expect(restored!.single.name, 'recovered.jpg');
      expect(image.decodeJpg(restored.single.bytes), isNotNull);
      final oversized = _LostPicker(
        LostDataResponse(
          type: RetrieveType.image,
          files: [
            XFile.fromData(
              Uint8List(maximumAttachmentBytes + 1),
              path: 'large.png',
            ),
          ],
        ),
      );
      await expectLater(
        restoreLostImages(context, picker: oversized),
        throwsFormatException,
      );
      final error = _LostPicker(
        LostDataResponse(exception: PlatformException(code: 'picker_failed')),
      );
      await expectLater(
        restoreLostImages(context, picker: error),
        throwsA(isA<PlatformException>()),
      );
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets(
    'concurrent composers claim a recovered attachment or recovery error once',
    (tester) async {
      late BuildContext context;
      await tester.pumpWidget(
        MaterialApp(
          home: Builder(
            builder: (value) {
              context = value;
              return const SizedBox();
            },
          ),
        ),
      );
      final ready = Completer<List<UploadAttachment>>();
      resetRecoveredImagesForTesting(retrieval: ready.future);
      final first = takeRecoveredImages(context);
      final second = takeRecoveredImages(context);
      ready.complete([UploadAttachment(name: 'restored.jpg', bytes: png)]);
      expect((await first).single.name, 'restored.jpg');
      expect(await second, isEmpty);
      expect(await takeRecoveredImages(context), isEmpty);
      final failure = Completer<List<UploadAttachment>>();
      resetRecoveredImagesForTesting(retrieval: failure.future);
      final firstError = expectLater(
        takeRecoveredImages(context),
        throwsFormatException,
      );
      final later = takeRecoveredImages(context);
      failure.completeError(const FormatException('picker_failed'));
      await firstError;
      expect(await later, isEmpty);
    },
  );
}
