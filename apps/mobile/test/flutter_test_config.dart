import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

/// Exercise the same font faces as the mobile package. Flutter's tester disables
/// asset-font registration by default and otherwise substitutes its Ahem face.
Future<void> testExecutable(FutureOr<void> Function() testMain) async {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(() async {
    const faces = [
      '',
      '-Medium',
      '-SemiBold',
      '-Bold',
      '-Italic',
      '-MediumItalic',
      '-SemiBoldItalic',
      '-BoldItalic',
    ];
    for (final family in ['Geist', 'GeistMono']) {
      final loader = FontLoader(family);
      for (final face in faces) {
        loader.addFont(rootBundle.load('assets/fonts/$family$face.ttf'));
      }
      await loader.load();
    }
    await (FontLoader(
      'MaterialIcons',
    )..addFont(rootBundle.load('fonts/MaterialIcons-Regular.otf'))).load();
  });
  await testMain();
}
