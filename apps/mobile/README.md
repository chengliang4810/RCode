# RCode mobile

One Flutter application for Android and iOS, replacing the former UIKit app.
Phone tabs, the tablet sidebar, Geist typefaces, colors and application flows
follow the original interface. Flutter's Android and iOS runners contain only
platform bootstrap, permissions and bridges.

The application connects to a desktop or headless RCode engine over
authenticated WebSocket RPC. Enter its `ws://host:27655` or `wss://host` address
and API key. Keys contain at least eight letters and digits, with both types
present. Credentials use the platform secure store. Accepted tasks continue on
the engine when the mobile application disconnects; reconnecting restores the
conversation. The mobile device does not run the agent.

## Toolchains

- Flutter **3.47.7** / Dart **3.13.5**, the version pinned by CI.
- Rust from the repository's `rust-toolchain.toml`, available on `PATH`.
- Android SDK, JDK 17 or newer, and Android NDK **27.0.12077973** for Android.
- macOS with Xcode and its iOS SDK for iOS. Install CocoaPods if Flutter reports
  it is needed for the resolved plugins. The iOS deployment target is 15.0.

```sh
cd apps/mobile
flutter pub get --enforce-lockfile
flutter analyze --no-pub
flutter test --no-pub --concurrency=1
flutter build apk --debug --target-platform android-arm64,android-x64
```

On Windows with this pinned SDK, run `scripts/mobile/test-windows.ps1` from the
repository root for the [isolated tester workaround](../../docs/windows-flutter-tests.md).
It disables host CPU profiling without editing the installed SDK, application
or test assertions. Linux CI uses the normal Flutter command above.
The controlled Windows run passed the six session-page tests three times and
the complete suite (72 passed, five documented gate skips); analysis was clean.

The Android build compiles the portable Rust visual/math library for arm64 and
x86_64 through Gradle. See [native build details](native/README.md).

On macOS, the Flutter Runner builds and links the same portable library:

```sh
cd apps/mobile
flutter pub get --enforce-lockfile
flutter build ios --debug --simulator --no-codesign
# An unsigned archive; distribution signing remains a separate step:
flutter build ipa --release --no-codesign --build-number <number>
```

The iOS bundle identifier remains `com.rcode.ios`. The manual TestFlight
workflow uses the existing App Store Connect secrets and signs the Flutter
archive during export. It is not triggered by local development commands.

## Integration checks

After resolving Flutter dependencies, run this from the repository root:

```sh
cargo test --locked -p rcode-engine --test flutter_direct -- --ignored --nocapture
```

It starts a local authenticated real engine, runs the shipping Dart controllers
against it, and checks questions, queue edits, attachment round trips, command
deduplication and completion while the client is disconnected. It requires no
external host, provider credential or App Store account.

Native visual tests use the existing Luau, chart, Canvas, scene/shader and zatex
engines. Their setup is in [native/README.md](native/README.md). The six math
fonts are passed to the native renderer; they are not just display preferences.

The current Windows validation covers Flutter unit/widgets, the real
engine/Dart integration and Rust visual/math tests. Android APK compilation is
being checked separately. Xcode simulator builds, physical iOS execution,
signing and TestFlight upload remain unverified on this host.

Notifications are local and depend on receiving workspace events while the app
is running. The direct service does not implement background APNs/FCM delivery
after suspension or termination.

To regenerate launcher sizes from the preserved app artwork, install Pillow
and run `python scripts/mobile/gen-icons.py` from the repository root.
