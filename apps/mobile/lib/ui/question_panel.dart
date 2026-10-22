import 'dart:async';

import 'package:flutter/material.dart';

import '../data/workspace_controller.dart';
import '../l10n.dart';
import '../theme.dart';

class QuestionPanel extends StatefulWidget {
  const QuestionPanel({
    super.key,
    required this.input,
    required this.onSubmit,
    required this.onDismiss,
  });
  final JsonMap input;
  final Future<void> Function(List<JsonMap>) onSubmit;
  final Future<void> Function() onDismiss;
  @override
  State<QuestionPanel> createState() => _QuestionPanelState();
}

class _QuestionPanelState extends State<QuestionPanel> {
  int _page = 0;
  bool _saving = false;
  String? _error;
  final Map<String, Set<String>> _picks = {};
  final Map<String, TextEditingController> _text = {};
  Timer? _advance;
  List<JsonMap> get _questions =>
      jsonMaps(jsonMap(widget.input['request'])['questions']);
  @override
  void initState() {
    super.initState();
    _configure();
  }

  @override
  void didUpdateWidget(QuestionPanel old) {
    super.didUpdateWidget(old);
    if (old.input['requestId'] != widget.input['requestId']) _configure();
  }

  void _configure() {
    _advance?.cancel();
    for (final controller in _text.values) {
      controller.dispose();
    }
    _text.clear();
    _picks.clear();
    _page = 0;
    _saving = false;
    _error = null;
    for (final q in _questions) {
      _text.putIfAbsent(
        '${q['id']}',
        () => TextEditingController(text: '${q['prefill'] ?? ''}'),
      );
    }
  }

  Future<void> _next() async {
    _advance?.cancel();
    if (_saving) return;
    if (_page < _questions.length - 1) {
      setState(() => _page++);
      return;
    }
    setState(() {
      _saving = true;
      _error = null;
    });
    try {
      await widget.onSubmit([
        for (final q in _questions)
          {
            'questionId': q['id'],
            'selectedOptionIds': (_picks['${q['id']}'] ?? {}).toList(),
            if (_text['${q['id']}']!.text.trim().isNotEmpty)
              'text': _text['${q['id']}']!.text,
          },
      ]);
    } catch (error) {
      if (mounted)
        setState(() {
          _saving = false;
          _error = '$error';
        });
    }
  }

  Future<void> _dismiss() async {
    if (_saving) return;
    setState(() => _saving = true);
    try {
      await widget.onDismiss();
    } catch (error) {
      if (mounted)
        setState(() {
          _saving = false;
          _error = '$error';
        });
    }
  }

  @override
  void dispose() {
    _advance?.cancel();
    for (final c in _text.values) {
      c.dispose();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    if (_questions.isEmpty) return const SizedBox.shrink();
    final q = _questions[_page];
    final id = '${q['id']}';
    final options = jsonMaps(q['options']);
    final picks = _picks.putIfAbsent(id, () => {});
    final c = context.colors;
    return Container(
      key: const Key('question-panel'),
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: c.elevated,
        borderRadius: BorderRadius.circular(26),
        border: Border.all(color: c.hairline),
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Expanded(
                child: Text(
                  '${q['header'] ?? context.tr('问题', 'Question')}${_questions.length > 1 ? ' · ${_page + 1}/${_questions.length}' : ''}',
                  style: TextStyle(color: c.secondary, fontSize: 12.5),
                ),
              ),
              IconButton(
                onPressed: _saving ? null : _dismiss,
                visualDensity: VisualDensity.compact,
                tooltip: context.tr('关闭问题', 'Dismiss question'),
                icon: const Icon(Icons.close, size: 18),
              ),
            ],
          ),
          Text(
            '${q['question'] ?? ''}',
            style: const TextStyle(fontSize: 17, fontWeight: FontWeight.w600),
          ),
          const SizedBox(height: 12),
          for (final option in options)
            Padding(
              padding: const EdgeInsets.only(bottom: 6),
              child: Semantics(
                selected: picks.contains('${option['id']}'),
                button: true,
                child: InkWell(
                  borderRadius: BorderRadius.circular(14),
                  onTap: _saving
                      ? null
                      : () {
                          setState(() {
                            final selected = '${option['id']}';
                            if (q['multiSelect'] == true) {
                              picks.contains(selected)
                                  ? picks.remove(selected)
                                  : picks.add(selected);
                            } else {
                              picks.clear();
                              picks.add(selected);
                            }
                          });
                          if (q['multiSelect'] != true &&
                              !MediaQuery.disableAnimationsOf(context)) {
                            _advance?.cancel();
                            _advance = Timer(
                              const Duration(milliseconds: 220),
                              _next,
                            );
                          }
                        },
                  child: Container(
                    padding: const EdgeInsets.all(12),
                    decoration: BoxDecoration(
                      borderRadius: BorderRadius.circular(14),
                      color: picks.contains('${option['id']}')
                          ? c.accent.withValues(alpha: .12)
                          : c.chip.withValues(alpha: .5),
                    ),
                    child: Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Icon(
                          picks.contains('${option['id']}')
                              ? (q['multiSelect'] == true
                                    ? Icons.check_box
                                    : Icons.check_circle)
                              : (q['multiSelect'] == true
                                    ? Icons.check_box_outline_blank
                                    : Icons.radio_button_unchecked),
                          size: 21,
                          color: picks.contains('${option['id']}')
                              ? c.accent
                              : c.secondary,
                        ),
                        const SizedBox(width: 10),
                        Expanded(
                          child: Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Text('${option['label'] ?? ''}'),
                              if ('${option['description'] ?? ''}'.isNotEmpty)
                                Text(
                                  '${option['description']}',
                                  style: TextStyle(
                                    color: c.secondary,
                                    fontSize: 13,
                                  ),
                                ),
                            ],
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          TextField(
            key: ValueKey('answer-$id'),
            controller: _text[id],
            enabled: !_saving,
            minLines: q['multiline'] == true ? 3 : 1,
            maxLines: q['multiline'] == true ? 6 : 1,
            decoration: InputDecoration(
              hintText: options.isEmpty
                  ? context.tr('输入回答', 'Your answer')
                  : context.tr('其他…', 'Other…'),
            ),
            onChanged: (_) => _advance?.cancel(),
            onSubmitted: (_) => _next(),
          ),
          if (_error != null)
            Padding(
              padding: const EdgeInsets.only(top: 8),
              child: Text(_error!, style: TextStyle(color: c.danger)),
            ),
          const SizedBox(height: 12),
          Row(
            children: [
              if (_page > 0)
                TextButton(
                  onPressed: _saving
                      ? null
                      : () {
                          _advance?.cancel();
                          setState(() => _page--);
                        },
                  child: Text(context.tr('上一步', 'Back')),
                ),
              const Spacer(),
              FilledButton(
                key: const Key('question-next'),
                onPressed: _saving ? null : _next,
                child: _saving
                    ? const SizedBox(
                        width: 18,
                        height: 18,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      )
                    : Text(
                        _page + 1 == _questions.length
                            ? context.tr('提交', 'Submit')
                            : context.tr('下一步', 'Next'),
                      ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}
