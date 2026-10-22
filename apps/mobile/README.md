# RCode mobile

Flutter application for Android and iOS. The interface follows the former UIKit app: phone tabs, a two-column tablet layout, the Geist typefaces and the RCode light/dark palette.

The Dart client connects to an RCode engine over authenticated WebSocket RPC. Luau visuals retain the portable Rust renderer through a C ABI; application pages are Flutter widgets.

```sh
flutter pub get
flutter analyze
flutter test
flutter build apk --debug
# On macOS with Xcode:
flutter build ios --simulator --no-codesign
```

The mobile application is a remote viewer/controller. Accepted tasks run in the connected desktop or headless engine and continue when the mobile client disconnects.
