import 'dart:io';
import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import 'preferences.dart';

/// A decorative backdrop: it never takes gestures or changes the page layout.
class WallpaperBackdrop extends StatefulWidget {
  const WallpaperBackdrop({
    super.key,
    required this.preferences,
    required this.child,
  });
  final AppPreferences preferences;
  final Widget child;
  @override
  State<WallpaperBackdrop> createState() => _WallpaperBackdropState();
}

class _WallpaperBackdropState extends State<WallpaperBackdrop> {
  ui.Image? _image;
  Uint8List? _pixels;
  String? _path;
  int _generation = 0;
  @override
  void initState() {
    super.initState();
    widget.preferences.addListener(_changed);
    _changed();
  }

  @override
  void didUpdateWidget(WallpaperBackdrop old) {
    super.didUpdateWidget(old);
    if (old.preferences != widget.preferences) {
      old.preferences.removeListener(_changed);
      widget.preferences.addListener(_changed);
      _path = null;
      _changed();
    }
  }

  void _changed() {
    if (_path != widget.preferences.wallpaperPath) {
      _path = widget.preferences.wallpaperPath;
      _load(_path, ++_generation);
    }
    if (mounted) setState(() {});
  }

  Future<void> _load(String? path, int generation) async {
    ui.Image? next;
    Uint8List? pixels;
    try {
      if (path != null) {
        final codec = await ui.instantiateImageCodec(
          await File(path).readAsBytes(),
          targetWidth: 256,
        );
        next = (await codec.getNextFrame()).image;
        codec.dispose();
        pixels = (await next.toByteData(format: ui.ImageByteFormat.rawRgba))
            ?.buffer
            .asUint8List();
      }
    } catch (_) {
      /* A missing decoration does not block opening conversations. */
    }
    if (!mounted || generation != _generation) {
      next?.dispose();
      return;
    }
    final previous = _image;
    setState(() {
      _image = next;
      _pixels = pixels;
    });
    previous?.dispose();
  }

  @override
  void dispose() {
    ++_generation;
    widget.preferences.removeListener(_changed);
    _image?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Stack(
    children: [
      if (_image != null)
        Positioned(
          top: 0,
          left: 0,
          right: 0,
          height: math.min(760, MediaQuery.sizeOf(context).height * .72),
          child: IgnorePointer(
            child: ShaderMask(
              blendMode: BlendMode.dstIn,
              shaderCallback: (rect) => const LinearGradient(
                begin: Alignment.topCenter,
                end: Alignment.bottomCenter,
                colors: [Colors.white, Colors.white, Colors.transparent],
                stops: [0, .3, 1],
              ).createShader(rect),
              child: Opacity(
                opacity: Theme.of(context).brightness == Brightness.dark
                    ? .23
                    : .30,
                child: RepaintBoundary(
                  child: CustomPaint(
                    painter: _WallpaperPainter(
                      _image!,
                      _pixels,
                      widget.preferences.wallpaperEffect,
                      Theme.of(context).brightness == Brightness.dark,
                    ),
                  ),
                ),
              ),
            ),
          ),
        ),
      widget.child,
    ],
  );
}

class _WallpaperPainter extends CustomPainter {
  _WallpaperPainter(this.image, this.pixels, this.effect, this.dark);
  final ui.Image image;
  final Uint8List? pixels;
  final String effect;
  final bool dark;
  @override
  void paint(Canvas canvas, Size size) {
    final source = Size(image.width.toDouble(), image.height.toDouble());
    final fitted = applyBoxFit(BoxFit.cover, source, size);
    final src = Alignment.center.inscribe(fitted.source, Offset.zero & source);
    final dest = Alignment.center.inscribe(
      fitted.destination,
      Offset.zero & size,
    );
    if (pixels == null || effect == 'none' || effect == 'scanlines') {
      canvas.drawImageRect(
        image,
        src,
        dest,
        Paint()..filterQuality = FilterQuality.medium,
      );
      if (effect == 'scanlines') {
        final paint = Paint()..color = Colors.black.withValues(alpha: .28);
        for (double y = 0; y < size.height; y += 4) {
          canvas.drawRect(Rect.fromLTWH(0, y, size.width, 1.25), paint);
        }
      }
      return;
    }
    final step = effect == 'ascii'
        ? 9.0
        : effect == 'halftone'
        ? 7.0
        : 4.0;
    final glyphs = <String, TextPainter>{};
    final pointPaint = Paint()..color = dark ? Colors.white : Colors.black;
    for (double y = 0; y < size.height; y += step) {
      for (double x = 0; x < size.width; x += step) {
        final ix = (src.left + x / size.width * src.width).floor().clamp(
          0,
          image.width - 1,
        );
        final iy = (src.top + y / size.height * src.height).floor().clamp(
          0,
          image.height - 1,
        );
        final i = (iy * image.width + ix) * 4;
        final luminance =
            (pixels![i] * .2126 +
                pixels![i + 1] * .7152 +
                pixels![i + 2] * .0722) /
            255;
        final strength = dark ? luminance : 1 - luminance;
        final color = dark ? Colors.white : Colors.black;
        if (effect == 'ascii') {
          final char = ' .:-=+*#%@'[(strength * 9).floor().clamp(0, 9)];
          final painter = glyphs.putIfAbsent(
            char,
            () => TextPainter(
              text: TextSpan(
                text: char,
                style: TextStyle(
                  fontFamily: 'GeistMono',
                  fontSize: step,
                  height: 1,
                  color: color,
                ),
              ),
              textDirection: TextDirection.ltr,
            )..layout(),
          );
          painter.paint(canvas, Offset(x, y));
        } else if (effect == 'halftone') {
          canvas.drawCircle(
            Offset(x + step / 2, y + step / 2),
            math.sqrt(strength) * step * .47,
            pointPaint,
          );
        } else {
          const matrix = [
            [0, 8, 2, 10],
            [12, 4, 14, 6],
            [3, 11, 1, 9],
            [15, 7, 13, 5],
          ];
          if (strength >
              matrix[(y / step).floor() % 4][(x / step).floor() % 4] / 16) {
            canvas.drawRect(
              Rect.fromLTWH(x, y, step * .8, step * .8),
              pointPaint,
            );
          }
        }
      }
    }
    for (final painter in glyphs.values) {
      painter.dispose();
    }
  }

  @override
  bool shouldRepaint(_WallpaperPainter old) =>
      old.image != image ||
      old.pixels != pixels ||
      old.effect != effect ||
      old.dark != dark;
}
