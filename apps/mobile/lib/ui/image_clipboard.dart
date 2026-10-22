import 'package:flutter/services.dart';

const maximumAttachmentBytes = 24 * 1024 * 1024;

class ClipboardImage {
  const ClipboardImage({required this.bytes, required this.name});
  final Uint8List bytes;
  final String name;
}

/// Reads only when the user invokes Paste image. No clipboard observation or
/// availability probe runs while a composer is opened or rebuilt.
Future<ClipboardImage?> readClipboardImage() async {
  const channel = MethodChannel('com.rcode.mobile/image_clipboard');
  final result = await channel.invokeMapMethod<String, dynamic>('readImage');
  if (result == null) return null;
  final bytes = result['bytes'];
  if (bytes is! Uint8List || bytes.isEmpty) {
    throw const FormatException('Invalid clipboard image');
  }
  if (bytes.length > maximumAttachmentBytes) {
    throw PlatformException(code: 'image_too_large');
  }
  final name = '${result['name'] ?? 'clipboard.png'}'
      .replaceAll('\\', '/')
      .split('/')
      .last;
  return ClipboardImage(
    bytes: bytes,
    name: name.isEmpty ? 'clipboard.png' : name,
  );
}
