import 'dart:typed_data';

import 'package:file_picker/file_picker.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_image_compress/flutter_image_compress.dart';
import 'package:image/image.dart' as image;
import 'package:image_picker/image_picker.dart';

import '../data/session_controller.dart';
import '../l10n.dart';
import 'image_clipboard.dart';

enum AttachmentSource { gallery, camera, file, clipboard }

Future<List<UploadAttachment>>? _recoveredImages;
bool _recoveredImagesClaimed = false;

/// All composers share one Android retrieval. Only the first claimant receives
/// its attachments (or failure), and recovery never submits a message.
Future<List<UploadAttachment>> takeRecoveredImages(BuildContext context) async {
  final retrieval = _recoveredImages ??= restoreLostImages(context);
  try {
    final images = await retrieval;
    if (_recoveredImagesClaimed) return [];
    _recoveredImagesClaimed = true;
    return images;
  } catch (_) {
    if (_recoveredImagesClaimed) return [];
    _recoveredImagesClaimed = true;
    rethrow;
  }
}

@visibleForTesting
void resetRecoveredImagesForTesting({
  Future<List<UploadAttachment>>? retrieval,
}) {
  _recoveredImages = retrieval;
  _recoveredImagesClaimed = false;
}

Future<List<UploadAttachment>> chooseImages(
  BuildContext context,
  AttachmentSource source,
) async {
  final tooLarge = context.tr(
    '单张图片不能超过 24 MiB',
    'Each image must be smaller than 24 MiB',
  );
  final noClipboardImage = context.tr('剪贴板中没有图片', 'No image in clipboard');
  final picked = <({String name, Uint8List bytes})>[];
  final picker = ImagePicker();
  if (source == AttachmentSource.clipboard) {
    try {
      final image = await readClipboardImage();
      if (image == null) {
        throw FormatException(noClipboardImage);
      }
      picked.add((name: image.name, bytes: image.bytes));
    } on PlatformException catch (error) {
      if (error.code == 'image_too_large') throw FormatException(tooLarge);
      rethrow;
    }
  } else if (source == AttachmentSource.file) {
    final files = await FilePicker.pickFiles(type: FileType.image);
    for (final file in files) {
      picked.add(await _readImageFile(file.xFile, tooLarge));
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
      picked.add(await _readImageFile(file, tooLarge));
    }
  }
  return _prepareImages(picked, tooLarge);
}

/// Android consumes lost picker results once. The app owns this recovery call
/// and puts returned attachments in the restored draft without sending them.
Future<List<UploadAttachment>> restoreLostImages(
  BuildContext context, {
  ImagePicker? picker,
}) async {
  if (kIsWeb || defaultTargetPlatform != TargetPlatform.android) return [];
  final tooLarge = context.tr(
    '单张图片不能超过 24 MiB',
    'Each image must be smaller than 24 MiB',
  );
  final response = await (picker ?? ImagePicker()).retrieveLostData();
  if (response.isEmpty) return [];
  if (response.exception case final error?) throw error;
  if (response.type != null && response.type != RetrieveType.image) {
    throw const FormatException('Unsupported recovered attachment');
  }
  final files =
      response.files ?? (response.file == null ? <XFile>[] : [response.file!]);
  final picked = <({String name, Uint8List bytes})>[];
  for (final file in files) {
    picked.add(await _readImageFile(file, tooLarge));
  }
  return _prepareImages(picked, tooLarge);
}

Future<({String name, Uint8List bytes})> _readImageFile(
  XFile file,
  String tooLarge,
) async {
  if (await file.length() > maximumAttachmentBytes) {
    throw FormatException(tooLarge);
  }
  final bytes = BytesBuilder(copy: false);
  await for (final chunk in file.openRead()) {
    if (chunk.length > maximumAttachmentBytes - bytes.length) {
      throw FormatException(tooLarge);
    }
    bytes.add(chunk);
  }
  return (name: file.name, bytes: bytes.takeBytes());
}

Future<List<UploadAttachment>> _prepareImages(
  List<({String name, Uint8List bytes})> picked,
  String tooLarge,
) async {
  final attachments = <UploadAttachment>[];
  for (final file in picked) {
    if (file.bytes.length > maximumAttachmentBytes) {
      throw FormatException(tooLarge);
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
    if (bytes.isEmpty || bytes.length > maximumAttachmentBytes) {
      throw const FormatException('Image conversion failed');
    }
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
              (
                AttachmentSource.clipboard,
                Icons.content_paste_outlined,
                context.tr('粘贴图片', 'Paste image'),
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
