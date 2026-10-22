import 'package:flutter_test/flutter_test.dart';
import 'package:rcode_mobile/notifications.dart';

void main() {
  test(
    'notices follow live transitions while snapshots and reconnects stay quiet',
    () {
      final tracker = NotificationTracker();
      expect(
        tracker.update('a', [
          {'chatId': 'one', 'status': 'working'},
        ]),
        isEmpty,
      );
      expect(
        tracker.update('a', [
          {'chatId': 'one', 'status': 'awaitingInput'},
        ]),
        [(chatId: 'one', kind: 'input')],
      );
      expect(
        tracker.update('a', [
          {'chatId': 'one', 'status': 'awaitingInput'},
        ]),
        isEmpty,
      );
      expect(
        tracker.update('a', [
          {'chatId': 'one', 'status': 'working'},
        ]),
        isEmpty,
      );
      expect(
        tracker.update('a', [
          {'chatId': 'one', 'status': 'idle', 'lastCompletedTurn': 'turn-1'},
        ]),
        [(chatId: 'one', kind: 'done')],
      );
      expect(
        tracker.update('a', [
          {'chatId': 'one', 'status': 'idle', 'lastCompletedTurn': 'turn-2'},
        ]),
        [(chatId: 'one', kind: 'done')],
      );
      expect(
        tracker.update('a', [
          {'chatId': 'one', 'status': 'errored'},
        ]),
        [(chatId: 'one', kind: 'failed')],
      );
      tracker.reset();
      expect(
        tracker.update('a', [
          {'chatId': 'one', 'status': 'errored'},
        ]),
        isEmpty,
      );
      expect(
        tracker.update('b', [
          {'chatId': 'one', 'status': 'awaitingInput'},
        ]),
        isEmpty,
      );
      tracker.update('b', []);
      expect(
        tracker.update('b', [
          {'chatId': 'one', 'status': 'idle', 'lastCompletedTurn': 'turn-3'},
        ]),
        isEmpty,
      );
    },
  );
}
