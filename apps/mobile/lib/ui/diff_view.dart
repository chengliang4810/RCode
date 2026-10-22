import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import '../data/workspace_controller.dart';
import '../l10n.dart';
import '../theme.dart';

/// Myers line diff with a work budget; a large rewrite still shows both full files.
List<JsonMap> lineDiff(List<String> sources) {
  final a = sources[0].split('\n'), b = sources[1].split('\n');
  final trace = <Map<int, int>>[];
  var v = <int, int>{1: 0};
  for (var d = 0; d <= a.length + b.length; d++) {
    if (d > 600) {
      return [
        for (final line in a) {'kind': '-', 'text': line},
        for (final line in b) {'kind': '+', 'text': line},
      ];
    }
    trace.add(Map.of(v));
    for (var k = -d; k <= d; k += 2) {
      var x = k == -d || (k != d && (v[k - 1] ?? -1) < (v[k + 1] ?? -1))
          ? (v[k + 1] ?? 0)
          : (v[k - 1] ?? 0) + 1;
      var y = x - k;
      while (x < a.length && y < b.length && a[x] == b[y]) {
        x++;
        y++;
      }
      v[k] = x;
      if (x >= a.length && y >= b.length) {
        final result = <JsonMap>[];
        for (var depth = d; depth >= 0; depth--) {
          final previous = trace[depth];
          final diagonal = x - y;
          final previousDiagonal =
              diagonal == -depth ||
                  (diagonal != depth &&
                      (previous[diagonal - 1] ?? -1) <
                          (previous[diagonal + 1] ?? -1))
              ? diagonal + 1
              : diagonal - 1;
          final px = previous[previousDiagonal] ?? 0;
          final py = px - previousDiagonal;
          while (x > px && y > py) {
            result.add({'kind': ' ', 'text': a[--x]});
            y--;
          }
          if (depth > 0) {
            if (x == px) {
              result.add({'kind': '+', 'text': b[--y]});
            } else {
              result.add({'kind': '-', 'text': a[--x]});
            }
          }
        }
        return result.reversed.toList();
      }
    }
  }
  return [];
}

class DiffView extends StatefulWidget {
  const DiffView({super.key, required this.diff});
  final JsonMap diff;
  @override
  State<DiffView> createState() => _DiffViewState();
}

class _DiffViewState extends State<DiffView> {
  late Future<List<JsonMap>> _lines;
  @override
  void initState() {
    super.initState();
    _load();
  }

  @override
  void didUpdateWidget(DiffView old) {
    super.didUpdateWidget(old);
    if (old.diff != widget.diff) _load();
  }

  void _load() => _lines = compute(lineDiff, [
    '${widget.diff['oldText'] ?? ''}',
    '${widget.diff['newText'] ?? ''}',
  ]);
  @override
  Widget build(BuildContext context) => Scaffold(
    appBar: AppBar(
      title: Text('${widget.diff['path'] ?? context.tr('变更', 'Changes')}'),
    ),
    body: FutureBuilder<List<JsonMap>>(
      future: _lines,
      builder: (context, snapshot) {
        if (!snapshot.hasData) {
          return const Center(child: CircularProgressIndicator());
        }
        final lines = snapshot.data!;
        final c = context.colors;
        return SelectionArea(
          child: ListView.builder(
            itemCount: lines.length,
            itemBuilder: (context, i) {
              final kind = lines[i]['kind'];
              final color = kind == '+'
                  ? c.success
                  : kind == '-'
                  ? c.danger
                  : c.secondary;
              return Container(
                color: kind == ' '
                    ? Colors.transparent
                    : color.withValues(alpha: .08),
                padding: const EdgeInsets.symmetric(
                  horizontal: 12,
                  vertical: 1,
                ),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    SizedBox(
                      width: 42,
                      child: Text(
                        '${i + 1}',
                        style: TextStyle(
                          color: c.tertiary,
                          fontFamily: 'GeistMono',
                          fontSize: 11,
                        ),
                      ),
                    ),
                    SizedBox(
                      width: 18,
                      child: Text(
                        '$kind',
                        style: TextStyle(color: color, fontFamily: 'GeistMono'),
                      ),
                    ),
                    Expanded(
                      child: Text(
                        '${lines[i]['text']}',
                        style: TextStyle(
                          fontFamily: 'GeistMono',
                          fontSize: 12.5,
                          height: 1.5,
                          color: c.text,
                        ),
                      ),
                    ),
                  ],
                ),
              );
            },
          ),
        );
      },
    ),
  );
}
