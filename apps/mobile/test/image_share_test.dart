import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as image;
import 'package:rcode_mobile/ui/message_view.dart';
import 'package:share_plus/share_plus.dart';

import 'support/page_workspace.dart';

void main() {
  testWidgets(
    'preview shares actual image bytes, matching MIME and filename with an iPad anchor',
    (tester) async {
      final workspace = PageWorkspace();
      final bytes = Uint8List.fromList(
        image.encodePng(image.Image(width: 24, height: 12)),
      );
      workspace.handler = (method, params) async =>
          method == 'ReadAttachmentChunk'
          ? {
              'data': base64Encode(bytes),
              'nextOffset': bytes.length,
              'done': true,
            }
          : <String, dynamic>{};
      ShareParams? shared;
      bool fail = false;
      await tester.pumpWidget(
        app(
          Scaffold(
            body: RemoteImage(
              session: workspace.session('chat'),
              path: '/images/report.jpg',
              name: 'report.jpg',
              share: (params) async {
                if (fail) {
                  throw PlatformException(
                    code: 'share_failed',
                    message: 'Share unavailable',
                  );
                }
                shared = params;
                return const ShareResult('', ShareResultStatus.dismissed);
              },
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byType(Image));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('image-share')));
      await tester.pumpAndSettle();
      expect(shared, isNotNull);
      expect(shared!.files!.single.mimeType, 'image/png');
      expect(shared!.fileNameOverrides, ['report.png']);
      expect(await shared!.files!.single.readAsBytes(), bytes);
      final origin = shared!.sharePositionOrigin!;
      expect(origin.width, greaterThan(0));
      expect(origin.height, greaterThan(0));
      expect(origin.left, greaterThanOrEqualTo(0));
      expect(origin.top, greaterThanOrEqualTo(0));
      fail = true;
      await tester.tap(find.byKey(const Key('image-share')));
      await tester.pumpAndSettle();
      expect(find.text('Share unavailable'), findsOneWidget);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
      workspace.dispose();
    },
  );
}
