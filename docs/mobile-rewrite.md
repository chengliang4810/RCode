# Flutter mobile application

`apps/mobile` contains one Flutter application for Android and iOS. It replaces
the former native app. The small `apps/mobile/ios` Runner remains responsible
for Flutter startup, plugin registration and platform clipboard access; all
application pages and navigation are Dart widgets. The original implementation
and its tests remain available in Git history.

## Application layers

| Layer | Responsibility |
| --- | --- |
| `lib/data` | Authenticated WebSocket RPC, reconnect, secure credentials, workspace subscriptions, incremental transcripts, command recovery, uploads and queue edit leases |
| `lib/ui` | Pairing, sessions/search/sections, project and model selection, conversations, composer, tools, questions, queue and settings |
| `lib/visual` | Flutter display-list painting and gestures for the portable Luau renderer; mathematical SVG rendering |
| `lib/theme.dart`, `assets/fonts` | Shared light/dark palette, Geist and Geist Mono |
| `lib/preferences.dart`, `lib/wallpaper.dart` | Appearance, math, notification and grouping preferences, retained drafts and wallpaper effects |
| `lib/notifications.dart` | Permission handling and local notifications from connected workspace updates |
| `native`, `crates/mobile` | Owned C ABI for the existing Rust visual and math engines, Android/iOS build integration |
| `android`, `ios` | Required Flutter platform runners, permissions and packaging |

Phone navigation uses Sessions, Settings and Search tabs. Tablets use a sidebar
and conversation split. The page layout, 62-point session rows, typography,
colors, composer shape, selection flows and operations follow the original app.
Platform blur, scrolling and keyboard behavior use Flutter's platform adapters.

The Dart client uses the same direct RPC endpoint as a remote desktop client.
Accepted commands belong to the engine: closing the mobile view, losing the
socket or suspending the application does not cancel the remote run. Reopening
restores authoritative transcript and command state. An unconfirmed send retries
with the same command and message identities.

Credentials are kept in the platform secure store. Transcripts and drafts use
host-scoped local caches. Signing out removes the saved pairing and its caches.
The application does not execute the coding agent locally.

## Build and validation

See [the mobile README](../apps/mobile/README.md) for toolchains, Android/iOS
commands and the authenticated Dart-to-engine test. `macos.yml` now validates
Flutter on Linux, builds an Android APK, and builds the Flutter iOS simulator
application. The manually dispatched TestFlight workflow archives the Flutter
Runner with the existing `com.rcode.ios` identity.

On the current Windows development host, Flutter unit/widget regressions, the
real authenticated engine/Dart integration, and Rust visual/math tests have
passed. Android APK compilation is being checked separately. The iOS Runner,
Xcode simulator build, device signing and TestFlight upload have **not** been
run on this host. CI wiring is not evidence of a successful remote build.

Notifications are local while the application receives workspace events. Direct
headless mode has no background APNs/FCM delivery service; notification settings
do not promise delivery after the operating system suspends or kills the app.
The remote engine continues its accepted work independently.
