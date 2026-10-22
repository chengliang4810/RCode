import 'dart:async';
import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:url_launcher/url_launcher.dart';

import '../l10n.dart';
import 'visual_backend.dart';
import 'visual_native_stub.dart'
    if (dart.library.ffi) 'visual_native.dart'
    as native;

typedef _VisualKey = (String, String, String, String, String);

final _scopes = <String, _VisualScope>{};
final _idleVisuals = <(_VisualScope, _VisualKey), _VisualLease>{};

/// Releases the visual workers retained by a conversation, including workers
/// whose rows have been recycled by a lazy transcript list.
void disposeVisualScope(String scope) {
  final owner = _scopes.remove(scope);
  if (owner == null) return;
  owner.closed = true;
  _idleVisuals.removeWhere((key, _) => identical(key.$1, owner));
  for (final lease in owner.leases.toList()) {
    lease.close();
  }
}

final class _VisualScope {
  final leases = <_VisualLease>{};
  bool closed = false;
}

final class _VisualLease {
  _VisualLease(this.backend, this.scope, this.key) {
    scope?.leases.add(this);
  }

  final VisualBackend backend;
  final _VisualScope? scope;
  final _VisualKey key;
  Map<String, dynamic>? display;
  double height = 72;
  bool closed = false;

  void close() {
    if (closed) return;
    closed = true;
    scope?.leases.remove(this);
    backend.dispose();
  }

  void release({required bool retain}) {
    final owner = scope;
    if (closed) return;
    if (!retain || owner == null || owner.closed) {
      close();
      return;
    }
    backend.command({'op': 'detach'});
    final poolKey = (owner, key);
    _idleVisuals.remove(poolKey)?.close();
    _idleVisuals[poolKey] = this;
    while (_idleVisuals.length > 32) {
      _idleVisuals.remove(_idleVisuals.keys.first)?.close();
    }
  }
}

/// The existing Luau/Canvas/chart/scene/shader engine, painted by Flutter.
/// Scoped workers survive row recycling until [disposeVisualScope] is called.
class RCodeVisual extends StatefulWidget {
  const RCodeVisual({
    super.key,
    required this.endpoint,
    required this.apiKey,
    required this.deviceId,
    required this.path,
    this.scope,
    this.id,
    this.title = '',
    this.textScale = 1,
    this.active = true,
    this.onHeightChanged,
    this.onOpenLink,
  });

  final String endpoint, apiKey, deviceId, path, title;
  final String? id, scope;
  final double textScale;
  final bool active;
  final ValueChanged<double>? onHeightChanged;
  final ValueChanged<Uri>? onOpenLink;

  @override
  State<RCodeVisual> createState() => _RCodeVisualState();
}

class _RCodeVisualState extends State<RCodeVisual> with WidgetsBindingObserver {
  _VisualLease? _lease;
  VisualBackend? _backend;
  Timer? _timer;
  Map<String, dynamic>? _display;
  final Map<String, Map<String, dynamic>> _frames = {};
  final Map<String, ui.Image> _images = {};
  final Map<String, int> _imageRevisions = {};
  double _width = 0;
  double _height = 72;
  int _generation = 0;
  bool _opening = false;
  bool _error = false;
  bool _attached = false;
  bool _foreground = true;
  bool _dark = false;
  bool _reduceMotion = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _dark = Theme.of(context).brightness == Brightness.dark;
    _reduceMotion = MediaQuery.disableAnimationsOf(context);
    _backend?.command({'op': 'appearance', 'dark': _dark});
    if (_attached) {
      _backend?.command({
        'op': 'attach',
        'dark': _dark,
        'reduceMotion': _reduceMotion,
      });
    }
  }

  @override
  void didUpdateWidget(RCodeVisual oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.endpoint != widget.endpoint ||
        oldWidget.apiKey != widget.apiKey ||
        oldWidget.deviceId != widget.deviceId ||
        oldWidget.path != widget.path ||
        oldWidget.id != widget.id ||
        oldWidget.scope != widget.scope) {
      _close(
        retain:
            oldWidget.endpoint == widget.endpoint &&
            oldWidget.apiKey == widget.apiKey &&
            oldWidget.deviceId == widget.deviceId &&
            oldWidget.scope == widget.scope,
      );
      _display = null;
      _error = false;
      _open();
    } else if (oldWidget.textScale != widget.textScale) {
      _backend?.command({
        'op': 'layout',
        'width': _width,
        'textScale': widget.textScale,
      });
    }
    _attachment();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    _foreground = state == AppLifecycleState.resumed;
    _attachment();
  }

  void _close({bool retain = true}) {
    _generation++;
    _timer?.cancel();
    _timer = null;
    _lease?.release(retain: retain);
    _lease = null;
    _backend = null;
    _opening = false;
    _attached = false;
    _frames.clear();
    _imageRevisions.clear();
    for (final image in _images.values) {
      image.dispose();
    }
    _images.clear();
  }

  Future<void> _open() async {
    if (!mounted || _opening || _backend != null || _width <= 0) return;
    _opening = true;
    final generation = ++_generation;
    final scopeName = widget.scope;
    final scope = scopeName == null
        ? null
        : _scopes.putIfAbsent(scopeName, _VisualScope.new);
    final id =
        widget.id ??
        widget.path.split('/').last.replaceFirst('-visual.luau', '');
    final key = (
      widget.endpoint,
      widget.apiKey,
      widget.deviceId,
      widget.path,
      id,
    );
    final config = <String, dynamic>{
      'endpoint': widget.endpoint,
      'apiKey': widget.apiKey,
      'deviceId': widget.deviceId,
      'path': widget.path,
      'id': id,
      'title': widget.title,
      'width': _width,
      'textScale': widget.textScale,
      'pixelScale': MediaQuery.devicePixelRatioOf(context),
      'dark': _dark,
      'reduceMotion': _reduceMotion,
    };
    try {
      var lease = scope == null ? null : _idleVisuals.remove((scope, key));
      final reused = lease != null;
      lease ??= _VisualLease(await native.openVisual(config), scope, key);
      if (!mounted || generation != _generation || scope?.closed == true) {
        lease.close();
        return;
      }
      final backend = lease.backend;
      _lease = lease;
      _backend = backend;
      if (reused) {
        setState(() {
          _display = lease!.display;
          _height = lease.height;
        });
        backend.command({
          'op': 'layout',
          'width': _width,
          'textScale': widget.textScale,
        });
        backend.command({
          'op': 'attach',
          'dark': _dark,
          'reduceMotion': _reduceMotion,
        });
      }
      _attached =
          true; // create() initially attaches; visibility may detach it below.
      _opening = false;
      _timer = Timer.periodic(const Duration(milliseconds: 16), (_) => _pump());
      _attachment();
      _pump();
    } catch (_) {
      if (!mounted || generation != _generation) return;
      setState(() {
        _opening = false;
        _error = true;
      });
    }
  }

  bool get _onScreen {
    final object = context.findRenderObject();
    if (object is! RenderBox || !object.attached || !object.hasSize) {
      return false;
    }
    final origin = object.localToGlobal(Offset.zero);
    final viewport = MediaQuery.sizeOf(context);
    return (origin & object.size).overlaps(Offset.zero & viewport);
  }

  void _attachment() {
    final backend = _backend;
    if (backend == null || !mounted) return;
    final wanted = widget.active && _foreground && _onScreen;
    if (wanted == _attached) return;
    _attached = wanted;
    backend.command(
      wanted
          ? {'op': 'attach', 'dark': _dark, 'reduceMotion': _reduceMotion}
          : {'op': 'detach'},
    );
  }

  void _pump() {
    if (!mounted) return;
    _attachment();
    final backend = _backend;
    if (backend == null) return;
    final update = backend.poll();
    if (update == null) return;
    // Fallback text is measured by Flutter on its isolate. The Rust worker
    // waits asynchronously; no native callback enters the isolate from a worker.
    for (final request in _list(update['measurements'])) {
      final text = request['text'] as String;
      final painter = TextPainter(
        text: TextSpan(text: text, style: visualTextStyle(request)),
        textDirection: TextDirection.ltr,
      )..layout();
      final advances = <double>[];
      var offset = 0;
      for (final cluster in text.characters) {
        final end = offset + cluster.length;
        final boxes = painter.getBoxesForSelection(
          TextSelection(baseOffset: offset, extentOffset: end),
        );
        final width = boxes.fold<double>(
          0,
          (width, box) => width + (box.right - box.left).abs(),
        );
        advances.add(width);
        advances.addAll(List.filled(cluster.runes.length - 1, 0));
        offset = end;
      }
      painter.dispose();
      backend.command({
        'op': 'measured',
        'id': request['id'],
        'advances': advances,
      });
    }
    final display = _map(update['display']);
    final frames = _list(update['frames']);
    if (display == null && frames.isEmpty) return;
    setState(() {
      if (display != null) {
        _display = display;
        _lease?.display = display;
        final height = _number(display['height']);
        if (height > 0 && height != _height) {
          _height = height;
          _lease?.height = height;
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (mounted) widget.onHeightChanged?.call(height);
          });
        }
        final live = _list(display['surfaces'])
            .map((surface) => _identifier(surface['id']))
            .toSet();
        for (final id
            in _images.keys.where((id) => !live.contains(id)).toList()) {
          _images.remove(id)?.dispose();
          _imageRevisions.remove(id);
        }
        _frames.removeWhere((id, _) => !live.contains(id));
      }
      for (final frame in frames) {
        final id = _identifier(frame['surface']);
        _frames[id] = frame;
        final image = _map(frame['image']);
        if (image != null) {
          final bytes = backend.takeImage(_integer(image['token']));
          if (bytes.isNotEmpty) _decode(id, image, bytes);
        } else {
          _imageRevisions[id] = (_imageRevisions[id] ?? 0) + 1;
          _images.remove(id)?.dispose();
        }
      }
    });
  }

  void _decode(String id, Map<String, dynamic> image, Uint8List bytes) {
    final generation = _generation;
    final revision = (_imageRevisions[id] ?? 0) + 1;
    _imageRevisions[id] = revision;
    ui.decodeImageFromPixels(
      bytes,
      _integer(image['width']),
      _integer(image['height']),
      ui.PixelFormat.bgra8888,
      (decoded) {
        if (!mounted ||
            generation != _generation ||
            _imageRevisions[id] != revision) {
          decoded.dispose();
          return;
        }
        setState(() {
          _images.remove(id)?.dispose();
          _images[id] = decoded;
        });
      },
    );
  }

  void _widthChanged(double width) {
    if (!width.isFinite || width <= 0 || (width - _width).abs() < .1) return;
    _width = width;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      _backend?.command({
        'op': 'layout',
        'width': width,
        'textScale': widget.textScale,
      });
      if (!_error) _open();
    });
  }

  @override
  Widget build(BuildContext context) => LayoutBuilder(
    builder: (context, constraints) {
      _widthChanged(constraints.maxWidth);
      if (_error) {
        return Row(
          children: [
            Expanded(
              child: Text(
                context.tr('无法显示可视化', 'Unable to display visualization'),
              ),
            ),
            IconButton(
              tooltip: context.tr('重试', 'Retry'),
              icon: const Icon(Icons.refresh),
              onPressed: () {
                setState(() => _error = false);
                _open();
              },
            ),
          ],
        );
      }
      final display = _display;
      if (display == null) {
        return const SizedBox(
          height: 72,
          child: Center(
            child: SizedBox.square(
              dimension: 18,
              child: CircularProgressIndicator(strokeWidth: 1.5),
            ),
          ),
        );
      }
      return Semantics(
        label: widget.title,
        container: true,
        explicitChildNodes: true,
        child: SizedBox(
          height: _height,
          child: Stack(
            clipBehavior: Clip.none,
            children: [
              Positioned.fill(
                child: IgnorePointer(
                  child: CustomPaint(
                    painter: VisualPainter(
                      _map(display['paint']) ?? {},
                      _list(display['styles']),
                    ),
                  ),
                ),
              ),
              for (final surface in _list(display['surfaces']))
                _surface(surface),
              for (final control in _list(display['controls']))
                _control(control),
            ],
          ),
        ),
      );
    },
  );

  Widget _surface(Map<String, dynamic> surface) {
    final id = _identifier(surface['id']);
    final frame = _frames[id];
    Widget content = Stack(
      fit: StackFit.expand,
      children: [
        if (_images[id] != null) RawImage(image: _images[id], fit: BoxFit.fill),
        if (frame != null)
          CustomPaint(
            painter: VisualPainter(
              _map(frame['paint']) ?? {},
              _list(frame['styles']),
            ),
          ),
      ],
    );
    final gesture = surface['gesture'];
    void send(String phase, Offset position) => _backend?.command({
      'op': 'pointer',
      'surface': id,
      'phase': phase,
      'x': position.dx,
      'y': position.dy,
    });
    if (gesture != 'none') {
      content = GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTapDown: gesture == 'tap'
            ? (details) => send('down', details.localPosition)
            : null,
        onTapUp: gesture == 'tap'
            ? (details) => send('up', details.localPosition)
            : null,
        onTapCancel: gesture == 'tap'
            ? () => send('cancel', Offset.zero)
            : null,
        onPanStart: gesture == 'drag'
            ? (details) => send('down', details.localPosition)
            : null,
        onPanUpdate: gesture == 'drag'
            ? (details) => send('move', details.localPosition)
            : null,
        onPanEnd: gesture == 'drag'
            ? (details) => send('up', details.localPosition)
            : null,
        onPanCancel: gesture == 'drag'
            ? () => send('cancel', Offset.zero)
            : null,
        onHorizontalDragStart: gesture == 'horizontalDrag'
            ? (details) => send('down', details.localPosition)
            : null,
        onHorizontalDragUpdate: gesture == 'horizontalDrag'
            ? (details) => send('move', details.localPosition)
            : null,
        onHorizontalDragEnd: gesture == 'horizontalDrag'
            ? (details) => send('up', details.localPosition)
            : null,
        onHorizontalDragCancel: gesture == 'horizontalDrag'
            ? () => send('cancel', Offset.zero)
            : null,
        child: content,
      );
    } else {
      content = IgnorePointer(child: content);
    }
    return _position(surface, content);
  }

  Widget _control(Map<String, dynamic> control) {
    final kind = _map(control['kind']) ?? {};
    final type = kind['kind'];
    final id = _identifier(control['id']);
    void activate() {
      if (type == 'link') {
        final uri = Uri.tryParse(kind['url'] as String? ?? '');
        if (uri != null && ['https', 'http', 'mailto'].contains(uri.scheme)) {
          if (widget.onOpenLink != null) {
            widget.onOpenLink!(uri);
          } else {
            unawaited(launchUrl(uri, mode: LaunchMode.externalApplication));
          }
        }
      } else {
        _backend?.command({'op': 'activate', 'control': id});
      }
    }

    void slide(double fraction) => _backend?.command({
      'op': 'slide',
      'control': id,
      'fraction': fraction.clamp(0, 1),
    });
    void drag(Offset offset) {
      final track = _number(kind['trackW']);
      if (track > 0) {
        slide(
          (_number(control['x']) + offset.dx - _number(kind['trackX'])) / track,
        );
      }
    }

    final slider = type == 'slider';
    final step = _number(kind['step']);
    final range = _number(kind['max']) - _number(kind['min']);
    final increment = step > 0 && range > 0 ? step / range : .1;
    return _position(
      control,
      Semantics(
        label: control['label'] as String? ?? '',
        button: !slider && type != 'toggle',
        link: type == 'link',
        toggled: type == 'toggle' ? kind['on'] == true : null,
        selected: type == 'tab' ? kind['selected'] == true : null,
        slider: slider,
        value: slider ? '${kind['shown'] ?? kind['value'] ?? ''}' : null,
        onTap: slider ? null : activate,
        onIncrease: slider
            ? () => slide(_number(kind['fraction']) + increment)
            : null,
        onDecrease: slider
            ? () => slide(_number(kind['fraction']) - increment)
            : null,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: slider ? null : activate,
          onTapDown: slider ? (details) => drag(details.localPosition) : null,
          onHorizontalDragStart: slider
              ? (details) => drag(details.localPosition)
              : null,
          onHorizontalDragUpdate: slider
              ? (details) => drag(details.localPosition)
              : null,
          child: const SizedBox.expand(),
        ),
      ),
    );
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _close();
    super.dispose();
  }
}

Widget _position(Map<String, dynamic> rect, Widget child) => Positioned(
  left: _number(rect['x']),
  top: _number(rect['y']),
  width: _number(rect['w']),
  height: _number(rect['h']),
  child: child,
);

Map<String, dynamic>? _map(Object? value) =>
    value is Map ? value.cast<String, dynamic>() : null;
List<Map<String, dynamic>> _list(Object? value) => value is List
    ? value
          .whereType<Map>()
          .map((item) => item.cast<String, dynamic>())
          .toList()
    : [];
double _number(Object? value) => value is num ? value.toDouble() : 0;
String _identifier(Object? value) => '$value';
int _integer(Object? value) => value is num ? value.toInt() : 0;

Color _color(Object? value) {
  final color = _map(value) ?? {};
  int channel(String name) =>
      (_number(color[name]) * 255).round().clamp(0, 255);
  return Color.fromARGB(channel('a'), channel('r'), channel('g'), channel('b'));
}

TextStyle visualTextStyle(Map<String, dynamic> style, {Color? color}) {
  final face = '${style['face'] ?? 'sans'}'.toLowerCase();
  return TextStyle(
    fontFamily: face.startsWith('mono') ? 'GeistMono' : 'Geist',
    fontWeight: face.contains('semibold')
        ? FontWeight.w600
        : face.contains('medium')
        ? FontWeight.w500
        : face.contains('bold')
        ? FontWeight.w700
        : FontWeight.w400,
    fontStyle: face.contains('italic') ? FontStyle.italic : FontStyle.normal,
    fontSize: _number(style['size']),
    color: color,
    fontFeatures: style['ligatures'] == false
        ? const [ui.FontFeature.disable('liga'), ui.FontFeature.disable('clig')]
        : null,
  );
}

/// Paints the portable Rust display list, including the same UTF-16 run offsets.
class VisualPainter extends CustomPainter {
  VisualPainter(this.display, this.styles);
  final Map<String, dynamic> display;
  final List<Map<String, dynamic>> styles;

  @override
  void paint(Canvas canvas, Size size) {
    for (final shape in _list(display['shapes'])) {
      if (shape['kind'] == 'rect') {
        canvas.drawRRect(
          RRect.fromRectAndRadius(
            Rect.fromLTWH(
              _number(shape['x']),
              _number(shape['y']),
              _number(shape['w']),
              _number(shape['h']),
            ),
            Radius.circular(_number(shape['radius'])),
          ),
          Paint()..color = _color(shape['color']),
        );
      } else if (shape['kind'] == 'path') {
        final coordinates = (shape['points'] as List? ?? []).cast<num>();
        if (coordinates.length < 4) continue;
        final path = Path()
          ..moveTo(coordinates[0].toDouble(), coordinates[1].toDouble());
        for (var index = 2; index + 1 < coordinates.length; index += 2) {
          path.lineTo(
            coordinates[index].toDouble(),
            coordinates[index + 1].toDouble(),
          );
        }
        if (shape['closed'] == true) path.close();
        if (shape['fill'] != null) {
          canvas.drawPath(path, Paint()..color = _color(shape['fill']));
        }
        if (shape['stroke'] != null) {
          final paint = Paint()
            ..color = _color(shape['stroke'])
            ..style = PaintingStyle.stroke
            ..strokeWidth = _number(shape['lineWidth']);
          final dash = _number(shape['dash']);
          if (dash <= 0) {
            canvas.drawPath(path, paint);
          } else {
            for (final metric in path.computeMetrics()) {
              for (
                double offset = 0;
                offset < metric.length;
                offset += dash * 2
              ) {
                canvas.drawPath(
                  metric.extractPath(
                    offset,
                    math.min(offset + dash, metric.length),
                  ),
                  paint,
                );
              }
            }
          }
        }
      }
    }
    final text = display['text'] as String? ?? '';
    final descriptions = {
      for (final style in styles) _integer(style['id']): style,
    };
    for (final run in _list(display['runs'])) {
      final start = _integer(run['start']);
      final end = start + _integer(run['len']);
      if (start < 0 || end > text.length || end <= start) continue;
      final style =
          descriptions[_integer(run['style'])] ?? {'face': 'sans', 'size': 15};
      final painter = TextPainter(
        text: TextSpan(
          text: text.substring(start, end),
          style: visualTextStyle(style, color: _color(run['color'])),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      final baseline = painter.computeDistanceToActualBaseline(
        TextBaseline.alphabetic,
      );
      painter.paint(
        canvas,
        Offset(_number(run['x']), _number(run['baseline']) - baseline),
      );
      painter.dispose();
    }
  }

  @override
  bool shouldRepaint(VisualPainter oldDelegate) =>
      display != oldDelegate.display || styles != oldDelegate.styles;
}
