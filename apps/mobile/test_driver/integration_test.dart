import 'dart:convert';
import 'dart:io';

import 'package:integration_test/integration_test_driver_extended.dart';

Future<void> main() async {
  final directory = Directory(
    Platform.environment['RCODE_MOBILE_QA_ARTIFACTS'] ??
        'build/qa/device-integration',
  );
  await directory.create(recursive: true);
  final resultFile = File('${directory.path}/result.json');
  if (await resultFile.exists()) {
    await resultFile.delete();
  }
  await integrationDriver(
    onScreenshot: (name, bytes, [arguments]) async {
      if (bytes.isEmpty || !RegExp(r'^[a-zA-Z0-9_-]+$').hasMatch(name)) {
        return false;
      }
      await File('${directory.path}/$name.png')
          .writeAsBytes(bytes, flush: true);
      return true;
    },
    writeResponseOnFailure: true,
    responseDataCallback: (data) async {
      final report = Map<String, dynamic>.from(data ?? {});
      report['screenshots'] = (report['screenshots'] as List? ?? [])
          .map((image) => image['screenshotName'])
          .toList();
      await resultFile.writeAsString(
        const JsonEncoder.withIndent('  ').convert(report),
        flush: true,
      );
    },
  );
}
