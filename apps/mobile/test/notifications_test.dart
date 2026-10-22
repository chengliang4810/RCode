import 'package:flutter_test/flutter_test.dart';
import 'package:rcode_mobile/notifications.dart';
import 'package:rcode_mobile/preferences.dart';
import 'package:shared_preferences/shared_preferences.dart';

void main() {
  test(
    'first-session permission is requested once and remembers denial',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await AppPreferences.load();
      var requests = 0;
      Future<bool?> request() async {
        requests++;
        return true;
      }

      await requestNotificationsAfterFirstSession(
        prefs,
        requestPermission: request,
      );
      await requestNotificationsAfterFirstSession(
        await AppPreferences.load(),
        requestPermission: request,
      );
      expect(requests, 1);
      expect(prefs.notifications, isTrue);
      SharedPreferences.setMockInitialValues({});
      final denied = await AppPreferences.load();
      await requestNotificationsAfterFirstSession(
        denied,
        requestPermission: () async => false,
      );
      expect(denied.notifications, isFalse);
      expect(denied.getJson('notificationPermission')?['asked'], isTrue);
    },
  );
  test('disabled notifications do not prompt and permission errors cannot fail a sent message', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await AppPreferences.load();
    await prefs.setValue('notifications', false);
    var requests = 0;
    await requestNotificationsAfterFirstSession(
      prefs,
      requestPermission: () async {
        requests++;
        return true;
      },
    );
    expect(requests, 0);
    expect(prefs.getJson('notificationPermission'), isNull);
    await prefs.setValue('notifications', true);
    await requestNotificationsAfterFirstSession(
      prefs,
      requestPermission: () async =>
          throw StateError('Native permission unavailable'),
    );
    expect(prefs.getJson('notificationPermission'), isNull);
  });
  test(
    'interrupt and liveness expiry do not announce successful completion',
    () {
      final tracker = NotificationTracker();
      tracker.update('a', [
        {'chatId': 'one', 'status': 'working', 'lastCompletedTurn': null},
      ]);
      expect(
        tracker.update('a', [
          {'chatId': 'one', 'status': 'idle', 'lastCompletedTurn': null},
        ]),
        isEmpty,
      );
      tracker.update('a', [
        {'chatId': 'one', 'status': 'working', 'lastCompletedTurn': 'turn-1'},
      ]);
      expect(
        tracker.update('a', [
          {'chatId': 'one', 'status': 'idle', 'lastCompletedTurn': 'turn-1'},
        ]),
        isEmpty,
      );
    },
  );
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
