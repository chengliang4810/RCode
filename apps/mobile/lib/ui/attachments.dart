import 'package:file_picker/file_picker.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_image_compress/flutter_image_compress.dart';
import 'package:image/image.dart' as image;
import 'package:image_picker/image_picker.dart';

import '../data/session_controller.dart';
import '../l10n.dart';

enum AttachmentSource { gallery, camera, file }

Future<List<UploadAttachment>> chooseImages(
  BuildContext context,
  AttachmentSource source,
) async {
  final picked = <({String name, Uint8List bytes})>[];
  final picker = ImagePicker();
  if (source == AttachmentSource.file) {
    final files = await FilePicker.pickFiles(
      type: FileType.image,
    );
    for (final file in files) {
      picked.add((name: file.name, bytes: await file.readAsBytes()));
    }
  } else {
    final files = source == AttachmentSource.camera
        ? [
            await picker.pickImage(
              source: ImageSource.camera,
              maxWidth: 2560,
              maxHeight: 2560,
              imageQuality: 86,
              requestFullMetadata: false,
            ),
          ].whereType<XFile>().toList()
        : await picker.pickMultiImage(
            maxWidth: 2560,
            maxHeight: 2560,
            imageQuality: 86,
            requestFullMetadata: false,
          );
    for (final file in files) {
      picked.add((name: file.name, bytes: await file.readAsBytes()));
    }
  }
  final attachments = <UploadAttachment>[];
  for (final file in picked) {
    if (file.bytes.length > 24 * 1024 * 1024) {
      throw FormatException(
        context.mounted
            ? context.tr(
                '单张图片不能超过 24 MiB',
                'Each image must be smaller than 24 MiB',
              )
            : 'Image exceeds 24 MiB',
      );
    }
    Uint8List bytes;
    try {
      // The native codec handles HEIC as well as ordinary camera/library images.
      bytes = await FlutterImageCompress.compressWithList(
        file.bytes,
        minWidth: 2560,
        minHeight: 2560,
        quality: 86,
        format: CompressFormat.jpeg,
        keepExif: false,
      );
    } catch (_) {
      bytes = await compute(normalizeImage, file.bytes);
    }
    bytes = await compute(normalizeImage, bytes);
    if (bytes.isEmpty || bytes.length > 24 * 1024 * 1024)
      throw const FormatException('Image conversion failed');
    final name = file.name.replaceFirst(RegExp(r'\.[^.]+$'), '');
    attachments.add(
      UploadAttachment(
        name: '${name.isEmpty ? 'image' : name}.jpg',
        bytes: bytes,
        mimeType: 'image/jpeg',
      ),
    );
  }
  return attachments;
}

Uint8List normalizeImage(Uint8List bytes) {
  final decoded = image.decodeImage(bytes);
  if (decoded == null) throw const FormatException('Unsupported image format');
  var normalized = image.bakeOrientation(decoded);
  if (normalized.width > 2560 || normalized.height > 2560) {
    normalized = image.copyResize(
      normalized,
      width: normalized.width >= normalized.height ? 2560 : null,
      height: normalized.height > normalized.width ? 2560 : null,
    );
  }
  return Uint8List.fromList(image.encodeJpg(normalized, quality: 86));
}

Future<AttachmentSource?> showAttachmentMenu(BuildContext context) =>
    showModalBottomSheet<AttachmentSource>(
      context: context,
      builder: (context) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            for (final item in [
              (
                AttachmentSource.gallery,
                Icons.photo_library_outlined,
                context.tr('照片图库', 'Photo library'),
              ),
              (
                AttachmentSource.camera,
                Icons.photo_camera_outlined,
                context.tr('拍照', 'Take photo'),
              ),
              (
                AttachmentSource.file,
                Icons.image_outlined,
                context.tr('选择图片文件', 'Choose image file'),
              ),
            ])
              ListTile(
                leading: Icon(item.$2),
                title: Text(item.$3),
                onTap: () => Navigator.pop(context, item.$1),
              ),
          ],
        ),
      ),
    );
