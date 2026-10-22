import 'dart:typed_data';

/// Owned native visual instance. Calls are bounded and do no I/O on the UI isolate.
abstract interface class VisualBackend {
  Map<String, dynamic>? poll();
  Uint8List takeImage(int token);
  void command(Map<String, dynamic> command);
  void dispose();
}
