import 'dart:typed_data';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as image;
import 'package:rcode_mobile/ui/attachments.dart';
import 'package:rcode_mobile/ui/message_text.dart';
import 'package:rcode_mobile/ui/question_panel.dart';
import 'package:rcode_mobile/theme.dart';

void main() {
  test('queue edits preserve invisible image and application context', () {
    const raw = 'Compare.\n\n$attachmentMarker\n- /images/photo.jpg$appshotMarker\n<appshot>private context</appshot>';
    expect(visiblePrompt(raw), 'Compare.');
    expect(promptImages(raw), ['/images/photo.jpg']);
    expect(replaceVisiblePrompt(raw, 'Compare.', 'Explain.'), raw.replaceFirst('Compare.', 'Explain.'));
    expect(replaceVisiblePrompt(raw, 'Compare.', ''), '');
    expect(visiblePrompt(replaceVisiblePrompt(raw, 'Compare.', 'Explain.')), 'Explain.');
  });
  test('images normalize to JPEG with a bounded long edge', () {
    final bytes = Uint8List.fromList(image.encodePng(image.Image(width: 3000, height: 30)));
    final normalized = image.decodeJpg(normalizeImage(bytes))!;
    expect(normalized.width, 2560);
    expect(normalized.height, 26);
    expect(() => normalizeImage(Uint8List.fromList([1, 2, 3])), throwsFormatException);
  });
  testWidgets('questions preserve option IDs, prior pages and suggested custom text', (tester) async {
    List<Map<String, dynamic>>? submitted;
    await tester.pumpWidget(MaterialApp(theme: rcodeTheme(Brightness.light),
      home: Scaffold(body: QuestionPanel(input: {
        'requestId': 'request-1', 'request': {'kind': 'questions', 'questions': [
          {'id': 'choice', 'header': 'Choice', 'question': 'Choose one', 'options': [
            {'id': 'one', 'label': 'Same label'}, {'id': 'two', 'label': 'Same label'}]},
          {'id': 'detail', 'header': 'Details', 'question': 'Add details', 'options': [], 'prefill': 'Suggested text', 'multiline': true},
        ]}}, onSubmit: (answers) async { submitted = answers; }, onDismiss: () async {}))));
    await tester.tap(find.text('Same label').at(1));
    await tester.pump(const Duration(milliseconds: 240));
    expect(find.text('Add details'), findsOneWidget);
    expect(find.text('Suggested text'), findsOneWidget);
    await tester.tap(find.text('Back'));
    await tester.pump();
    expect(find.byIcon(Icons.check_circle), findsOneWidget);
    await tester.tap(find.byKey(const Key('question-next')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('question-next')));
    await tester.pump();
    expect(submitted, [
      {'questionId': 'choice', 'selectedOptionIds': ['two']},
      {'questionId': 'detail', 'selectedOptionIds': <String>[], 'text': 'Suggested text'},
    ]);
    await tester.pumpWidget(const SizedBox());
  });
}
