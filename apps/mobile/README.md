# RCode mobile

One Flutter application for Android and iOS, replacing the former UIKit app.
Phone tabs, the tablet sidebar, Geist typefaces, colors and application flows
follow the original interface. Flutter's Android and iOS runners contain only
platform bootstrap, permissions and bridges.

Hardware keyboards support Cmd+N for a new conversation, Cmd+Shift+F for search,
Cmd+B to toggle the wide sidebar and Cmd+Enter to submit the new-conversation
page. Android also accepts Ctrl in place of Cmd.

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
The controlled Windows run passed the six session-page tests three times.
[Linux CI](https://github.com/chengliang4810/RCode/actions/runs/39848701487/job/120068850622)
passed the current full suite: 87 tests, with five environment-gated skips;
analysis was clean. The real-engine gate runs separately in the same job.

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

The device integration gate uses the shipping app, native storage plugins and
renderers with the real local engine's provider-free `mobile_smoke` fixture.
It checks pairing, rendered pixels and controls, typed questions, and a task
finishing with the app connection closed before secure credentials restore it
exactly once. The SDK's test text input drives fields; native keyboard behavior
and an OS process restart are outside this automated gate. The separate manual
Android check below includes a real app force-stop.

On a connected Android phone, build the integration APK with the Gradle
property `-PrcodeQa=true`, then pass that APK to `flutter drive` with
`--use-application-binary=<path-to-qa-apk>`. The opt-in debug package is
`com.rcode.rcode_mobile.qa`, so the gate can replace its saved fixture host and
the driver can uninstall it without changing the ordinary installed app.
The default debug and release application identifiers are unchanged.

On macOS, resolve the locked dependencies, then run from the repository root:

```sh
bash scripts/mobile/test-ios.sh
```

This creates and removes its own iPhone simulator and fixture process. PNGs,
structured results and logs go to `apps/mobile/build/qa/ios`, or the directory
specified by `RCODE_MOBILE_QA_ARTIFACTS`. The `mobile-ios` CI job first builds the
ordinary unsigned app, then runs this gate and uploads the runtime artifacts.
The dependency lock uses pub.dev; when a shell sets a custom `PUB_HOSTED_URL`,
use `PUB_HOSTED_URL=https://pub.dev flutter pub get --enforce-lockfile` to match
the canonical lock.

Both Android arm64 and x86_64 debug APKs have been built and checked for 16 KiB
native-library/ZIP alignment. The x86_64 app was exercised on Android 16/API 36
against a real local engine with a mock model: pairing, interrupted-client
recovery, questions, image upload/preview/sharing, native visuals/math,
notifications permission and phone/tablet draft preservation.

The physical arm64 phone `23113RKC6C` (Android 16/API 36, SM8550/Adreno) passed
the same six-check gate in 27 seconds, producing five reviewed PNGs through the
isolated QA package. The ordinary shipping app also recovered credentials and
the completed response after a real force-stop while the server kept running,
without replaying the prompt or command. Android system IME input and keyboard
avoidance were checked manually. This phone uses 4096-byte pages; 16 KiB refers
to the verified package alignment, not a 16 KiB-page runtime test.

This physical-device run reused the CI APK/native libraries from `afee6e6` and
an engine fixture built from clean validation source `2d86810`; it does not
validate unrelated uncommitted Rust changes. Evidence and remaining limits are
in [the migration record](../../docs/flutter-mobile-migration.md).

The real engine/Dart integration, Rust visual/math suite and native Flutter FFI
checks also pass separately. [GitHub-hosted macOS CI](https://github.com/chengliang4810/RCode/actions/runs/39848701487/job/120074373888)
built the unsigned iOS simulator app and passed its six-check device gate at
`afee6e6`, using Xcode 26.6 and an iOS 26.5 iPhone 11 simulator. Its five PNGs
were reviewed, including the running turn and restored text/native content.
Physical iPhone/iPad execution, physical hardware keyboards, real model provider
accounts, signing and TestFlight upload remain unverified.

Notifications are local and depend on receiving workspace events while the app
is running. The direct service does not implement background APNs/FCM delivery
after suspension or termination.

To regenerate launcher sizes from the preserved app artwork, install Pillow
and run `python scripts/mobile/gen-icons.py` from the repository root.
