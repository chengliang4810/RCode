import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';

import 'visual_native_stub.dart'
    if (dart.library.ffi) 'visual_native.dart'
    as native;

/// Math from the same six zatex fonts as the previous mobile renderer.
/// A streaming/incomplete formula remains readable as source until it parses.
class RCodeMath extends StatefulWidget {
  const RCodeMath({
    super.key,
    required this.source,
    this.font = 'fira',
    this.fontSize = 16,
    this.display = false,
    this.color,
  });
  final String source, font;
  final double fontSize;
  final bool display;
  final Color? color;

  @override
  State<RCodeMath> createState() => _RCodeMathState();
}

class _RCodeMathState extends State<RCodeMath> {
  late Future<Map<String, dynamic>?> _formula;

  @override
  void initState() {
    super.initState();
    _load();
  }

  void _load() {
    _formula = native.renderNativeMath({
      'source': widget.source,
      'font': widget.font,
      'fontSize': widget.fontSize,
      'display': widget.display,
    });
  }

  @override
  void didUpdateWidget(RCodeMath oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.source != widget.source ||
        oldWidget.font != widget.font ||
        oldWidget.fontSize != widget.fontSize ||
        oldWidget.display != widget.display) {
      _load();
    }
  }

  @override
  Widget build(BuildContext context) => FutureBuilder<Map<String, dynamic>?>(
    future: _formula,
    builder: (context, snapshot) {
      final formula = snapshot.connectionState == ConnectionState.done
          ? snapshot.data
          : null;
      if (formula == null) {
        return Text(
          widget.source,
          style: TextStyle(fontSize: widget.fontSize, color: widget.color),
        );
      }
      final picture = Semantics(
        label: widget.source,
        child: SvgPicture.string(
          formula['svg'] as String,
          width: (formula['width'] as num).toDouble(),
          height: (formula['height'] as num).toDouble(),
          colorFilter: ColorFilter.mode(
            widget.color ??
                DefaultTextStyle.of(context).style.color ??
                Colors.black,
            BlendMode.srcIn,
          ),
        ),
      );
      if (!widget.display) {
        return Baseline(
          baseline: (formula['ascent'] as num).toDouble(),
          baselineType: TextBaseline.alphabetic,
          child: picture,
        );
      }
      return SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: picture,
      );
    },
  );
}
