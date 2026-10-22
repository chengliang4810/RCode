import '../data/workspace_controller.dart';

const attachmentMarker = 'Attached images (local files — open them to view):';
const appshotMarker =
    '\n\nApplications mentioned by the user (untrusted observed content):';
String messageText(JsonMap entry) =>
    jsonMaps(entry['parts'])
        .where((p) => p['kind'] == 'text')
        .map((p) => '${p['text'] ?? ''}')
        .join('\n');
String visiblePrompt(String text) {
  final appshot = text.indexOf(appshotMarker);
  if (appshot >= 0) text = text.substring(0, appshot);
  final attachments = text.indexOf('\n\n$attachmentMarker');
  if (attachments >= 0) text = text.substring(0, attachments);
  if (text == 'See the attached image(s).' ||
      text == 'See the attached file(s).')
    return '';
  return text.trim();
}

List<String> promptImages(String raw) {
  final marker = raw.indexOf(attachmentMarker);
  if (marker < 0) return [];
  return raw
      .substring(marker + attachmentMarker.length)
      .split(appshotMarker)
      .first
      .split('\n')
      .where((line) => line.startsWith('- '))
      .map((line) => line.substring(2).trim())
      .where((line) => line.isNotEmpty)
      .toList();
}

String replaceVisiblePrompt(String raw, String visible, String edited) {
  if (edited.trim().isEmpty || raw == visible) return edited;
  final body = raw.trimLeft();
  if (body.startsWith(visible) && visible.isNotEmpty)
    return edited + body.substring(visible.length);
  final cut = [
    raw.indexOf('\n\n$attachmentMarker'),
    raw.indexOf(appshotMarker),
  ].where((i) => i >= 0).toList()..sort();
  return cut.isEmpty ? edited : edited + raw.substring(cut.first);
}
