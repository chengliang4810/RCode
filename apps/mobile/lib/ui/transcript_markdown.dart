import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:highlight/highlight.dart' as hl;
import 'package:markdown/markdown.dart' as md;
import 'package:url_launcher/url_launcher.dart';

import '../l10n.dart';
import '../preferences.dart';
import '../theme.dart';
import '../visual/native_math.dart';

class TranscriptMarkdown extends StatelessWidget {
  const TranscriptMarkdown({
    super.key,
    required this.text,
    required this.preferences,
    this.imageBuilder,
  });
  final String text;
  final AppPreferences preferences;
  final MarkdownImageBuilder? imageBuilder;
  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return MarkdownBody(
      data: text,
      selectable: true,
      imageBuilder: imageBuilder,
      onTapLink: (_, href, _) {
        if (href != null) openExternalLink(href);
      },
      extensionSet: md.ExtensionSet.gitHubFlavored,
      inlineSyntaxes: preferences.renderMath ? [_MathInlineSyntax()] : const [],
      blockSyntaxes: preferences.renderMath ? [_MathBlockSyntax()] : const [],
      builders: {
        'pre': _CodeBuilder(),
        if (preferences.renderMath)
          'math-inline': _MathBuilder(preferences, false),
        if (preferences.renderMath)
          'math-block': _MathBuilder(preferences, true),
      },
      styleSheet: MarkdownStyleSheet.fromTheme(Theme.of(context)).copyWith(
        p: TextStyle(
          fontFamily: 'Geist',
          fontSize: 16,
          height: 1.55,
          color: c.text,
        ),
        a: TextStyle(color: c.accent),
        code: TextStyle(
          fontFamily: 'GeistMono',
          fontSize: 13.5,
          backgroundColor: c.chip.withValues(alpha: .6),
          color: c.text,
        ),
        blockSpacing: 12,
        listIndent: 24,
        tableBorder: TableBorder.all(color: c.hairline),
        tableCellsPadding: const EdgeInsets.all(8),
        blockquoteDecoration: BoxDecoration(
          border: Border(
            left: BorderSide(color: c.accent.withValues(alpha: .45), width: 3),
          ),
        ),
        blockquotePadding: const EdgeInsets.only(left: 14),
        codeblockPadding: EdgeInsets.zero,
        codeblockDecoration: const BoxDecoration(),
        horizontalRuleDecoration: BoxDecoration(
          border: Border(top: BorderSide(color: c.hairline)),
        ),
      ),
    );
  }
}

Future<bool> openExternalLink(String raw) async {
  final uri = Uri.tryParse(raw);
  if (uri == null || !const ['http', 'https', 'mailto'].contains(uri.scheme)) {
    return false;
  }
  return launchUrl(uri, mode: LaunchMode.externalApplication);
}

class _CodeBuilder extends MarkdownElementBuilder {
  @override
  bool isBlockElement() => true;
  @override
  Widget? visitElementAfterWithContext(
    BuildContext context,
    md.Element element,
    TextStyle? preferredStyle,
    TextStyle? parentStyle,
  ) {
    final code = element.children?.whereType<md.Element>().firstOrNull;
    final language =
        code?.attributes['class']?.replaceFirst('language-', '') ?? '';
    final text = element.textContent.replaceFirst(RegExp(r'\n$'), '');
    return CodeBlock(text: text, language: language);
  }
}

class CodeBlock extends StatelessWidget {
  const CodeBlock({super.key, required this.text, this.language = ''});
  final String text, language;
  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    const known = {
      'dart',
      'rust',
      'swift',
      'kotlin',
      'java',
      'javascript',
      'typescript',
      'python',
      'json',
      'yaml',
      'xml',
      'html',
      'css',
      'sql',
      'bash',
      'shell',
      'c',
      'cpp',
      'go',
      'markdown',
      'ruby',
      'diff',
      'lua',
    };
    final lang = switch (language.toLowerCase()) {
      'js' => 'javascript',
      'ts' => 'typescript',
      'py' => 'python',
      'sh' => 'bash',
      'rs' => 'rust',
      'yml' => 'yaml',
      final value => value,
    };
    return Container(
      decoration: BoxDecoration(
        color: c.codeBackground,
        border: Border.all(color: c.hairline),
        borderRadius: BorderRadius.circular(12),
      ),
      clipBehavior: Clip.antiAlias,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.only(left: 12, right: 4),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    language,
                    style: TextStyle(
                      fontFamily: 'GeistMono',
                      fontSize: 11,
                      color: c.secondary,
                    ),
                  ),
                ),
                IconButton(
                  visualDensity: VisualDensity.compact,
                  tooltip: context.tr('复制代码', 'Copy code'),
                  onPressed: () => Clipboard.setData(ClipboardData(text: text)),
                  icon: const Icon(Icons.copy_outlined, size: 16),
                ),
              ],
            ),
          ),
          SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            padding: const EdgeInsets.fromLTRB(12, 0, 12, 12),
            child: SelectableText.rich(
              known.contains(lang)
                  ? _highlightCode(text, lang, {
                      'keyword': TextStyle(color: c.accent),
                      'string': TextStyle(color: c.success),
                      'comment': TextStyle(color: c.tertiary),
                      'number': TextStyle(color: c.warning),
                      'title': TextStyle(color: c.accent),
                      'built_in': TextStyle(color: c.accent),
                    })
                  : TextSpan(text: text),
              textScaler: MediaQuery.textScalerOf(context),
              style: TextStyle(
                fontFamily: 'GeistMono',
                fontSize: 13,
                height: 1.5,
                color: c.text,
              ),
            ),
          ),
        ],
      ),
    );
  }
}

TextSpan _highlightCode(
  String text,
  String language,
  Map<String, TextStyle> styles,
) {
  TextSpan span(hl.Node node) => TextSpan(
    text: node.value,
    style: styles[node.className],
    children: node.children?.map(span).toList(),
  );
  final nodes = hl.highlight.parse(text, language: language).nodes;
  return nodes == null
      ? TextSpan(text: text)
      : TextSpan(children: nodes.map(span).toList());
}

class _MathInlineSyntax extends md.InlineSyntax {
  _MathInlineSyntax()
    : super(r'(?<!\\)\$(?!\$)([^\n$]+?)\$(?!\$)|\\\((.+?)\\\)');
  @override
  bool onMatch(md.InlineParser parser, Match match) {
    parser.addNode(md.Element.text('math-inline', match[1] ?? match[2] ?? ''));
    return true;
  }
}

class _MathBlockSyntax extends md.BlockSyntax {
  @override
  RegExp get pattern => RegExp(r'^\s*(\$\$|\\\[)');
  @override
  md.Node parse(md.BlockParser parser) {
    final line = parser.current.content.trimLeft();
    final marker = line.startsWith(r'$$') ? r'$$' : r'\]';
    var body = line.substring(2);
    parser.advance();
    final lines = <String>[];
    if (body.endsWith(marker)) {
      lines.add(body.substring(0, body.length - 2));
    } else {
      lines.add(body);
      while (!parser.isDone) {
        body = parser.current.content;
        parser.advance();
        if (body.trimRight().endsWith(marker)) {
          lines.add(body.trimRight().substring(0, body.trimRight().length - 2));
          break;
        }
        lines.add(body);
      }
    }
    return md.Element.text('math-block', lines.join('\n').trim());
  }
}

class _MathBuilder extends MarkdownElementBuilder {
  _MathBuilder(this.preferences, this.display);
  final AppPreferences preferences;
  final bool display;
  @override
  bool isBlockElement() => display;
  @override
  Widget? visitElementAfterWithContext(
    BuildContext context,
    md.Element element,
    TextStyle? preferredStyle,
    TextStyle? parentStyle,
  ) => RCodeMath(
    source: element.textContent,
    font: preferences.mathFont,
    fontSize:
        16 * preferences.mathSize * MediaQuery.textScalerOf(context).scale(1),
    display: display,
  );
}
