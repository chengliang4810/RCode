import 'dart:convert';
import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';
import 'package:flutter/services.dart';

import 'visual_backend.dart';

final class _Bytes extends Struct {
  external Pointer<Uint8> data;
  @UintPtr()
  external int length;
}

typedef _FontNative = Bool Function(Uint8, Pointer<Uint8>, UintPtr);
typedef _FontDart = bool Function(int, Pointer<Uint8>, int);
typedef _CreateNative = Uint64 Function(Pointer<Utf8>);
typedef _CreateDart = int Function(Pointer<Utf8>);
typedef _PollNative = Pointer<Utf8> Function(Uint64);
typedef _PollDart = Pointer<Utf8> Function(int);
typedef _ImageNative = _Bytes Function(Uint64, Uint64);
typedef _ImageDart = _Bytes Function(int, int);
typedef _FreeBytesNative = Void Function(_Bytes);
typedef _FreeBytesDart = void Function(_Bytes);
typedef _CommandNative = Void Function(Uint64, Pointer<Utf8>);
typedef _CommandDart = void Function(int, Pointer<Utf8>);
typedef _DisposeNative = Void Function(Uint64);
typedef _DisposeDart = void Function(int);
typedef _ErrorNative = Pointer<Utf8> Function();
typedef _ErrorDart = Pointer<Utf8> Function();
typedef _FreeStringNative = Void Function(Pointer<Utf8>);
typedef _FreeStringDart = void Function(Pointer<Utf8>);

final class _Library {
  _Library() {
    final library = Platform.isIOS
        ? DynamicLibrary.process()
        : DynamicLibrary.open(
            Platform.isWindows ? 'rcode_mobile.dll' : 'librcode_mobile.so',
          );
    font = library.lookupFunction<_FontNative, _FontDart>('rcode_visual_font');
    create = library.lookupFunction<_CreateNative, _CreateDart>(
      'rcode_visual_create',
    );
    poll = library.lookupFunction<_PollNative, _PollDart>('rcode_visual_poll');
    image = library.lookupFunction<_ImageNative, _ImageDart>(
      'rcode_visual_image',
    );
    freeBytes = library.lookupFunction<_FreeBytesNative, _FreeBytesDart>(
      'rcode_visual_bytes_free',
    );
    command = library.lookupFunction<_CommandNative, _CommandDart>(
      'rcode_visual_command',
    );
    dispose = library.lookupFunction<_DisposeNative, _DisposeDart>(
      'rcode_visual_dispose',
    );
    error = library.lookupFunction<_ErrorNative, _ErrorDart>(
      'rcode_visual_error',
    );
    freeString = library.lookupFunction<_FreeStringNative, _FreeStringDart>(
      'rcode_visual_string_free',
    );
    math = library.lookupFunction<_CreateMathNative, _CreateMathDart>(
      'rcode_math_svg',
    );
  }

  late final _FontDart font;
  late final _CreateDart create;
  late final _PollDart poll;
  late final _ImageDart image;
  late final _FreeBytesDart freeBytes;
  late final _CommandDart command;
  late final _DisposeDart dispose;
  late final _ErrorDart error;
  late final _FreeStringDart freeString;
  late final _CreateMathDart math;

  Object? read(Pointer<Utf8> pointer) {
    if (pointer == nullptr) return null;
    try {
      return jsonDecode(pointer.toDartString());
    } finally {
      freeString(pointer);
    }
  }
}

typedef _CreateMathNative = Pointer<Utf8> Function(Pointer<Utf8>);
typedef _CreateMathDart = Pointer<Utf8> Function(Pointer<Utf8>);

Future<_Library>? _library;

Future<_Library> _initialize() async {
  final library = _Library();
  const faces = [
    'Geist',
    'Geist-Medium',
    'Geist-SemiBold',
    'Geist-Bold',
    'Geist-Italic',
    'Geist-MediumItalic',
    'Geist-SemiBoldItalic',
    'Geist-BoldItalic',
    'GeistMono',
    'GeistMono-Medium',
    'GeistMono-SemiBold',
    'GeistMono-Italic',
  ];
  for (var index = 0; index < faces.length; index++) {
    final data = await rootBundle.load('assets/fonts/${faces[index]}.ttf');
    final bytes = data.buffer.asUint8List(
      data.offsetInBytes,
      data.lengthInBytes,
    );
    final pointer = calloc<Uint8>(bytes.length);
    try {
      pointer.asTypedList(bytes.length).setAll(0, bytes);
      if (!library.font(index, pointer, bytes.length)) {
        throw StateError('Unable to register visual fonts');
      }
    } finally {
      calloc.free(pointer);
    }
  }
  return library;
}

Future<VisualBackend> openVisual(Map<String, dynamic> config) async {
  final library = await (_library ??= _initialize().catchError((Object error) {
    _library = null;
    throw error;
  }));
  final text = jsonEncode(config).toNativeUtf8();
  try {
    final id = library.create(text);
    if (id == 0) {
      throw StateError(
        '${library.read(library.error()) ?? 'Unable to open visual'}',
      );
    }
    return _NativeVisual(library, id);
  } finally {
    malloc.free(text);
  }
}

Future<Map<String, dynamic>?> renderNativeMath(
  Map<String, dynamic> config,
) async {
  try {
    final library = await (_library ??= _initialize().catchError((
      Object error,
    ) {
      _library = null;
      throw error;
    }));
    final text = jsonEncode(config).toNativeUtf8();
    try {
      return library.read(library.math(text)) as Map<String, dynamic>?;
    } finally {
      malloc.free(text);
    }
  } catch (_) {
    return null;
  }
}

final class _NativeVisual implements VisualBackend {
  _NativeVisual(this.library, this.id);
  final _Library library;
  int id;

  @override
  Map<String, dynamic>? poll() =>
      id == 0 ? null : library.read(library.poll(id)) as Map<String, dynamic>?;

  @override
  Uint8List takeImage(int token) {
    if (id == 0) return Uint8List(0);
    final bytes = library.image(id, token);
    if (bytes.data == nullptr) return Uint8List(0);
    try {
      return Uint8List.fromList(bytes.data.asTypedList(bytes.length));
    } finally {
      library.freeBytes(bytes);
    }
  }

  @override
  void command(Map<String, dynamic> command) {
    if (id == 0) return;
    final text = jsonEncode(command).toNativeUtf8();
    try {
      library.command(id, text);
    } finally {
      malloc.free(text);
    }
  }

  @override
  void dispose() {
    final owned = id;
    id = 0;
    if (owned != 0) library.dispose(owned);
  }
}
