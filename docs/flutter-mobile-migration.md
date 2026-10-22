# Flutter mobile migration

The requested end state replaces `apps/ios` with `apps/mobile`, targeting Android and iOS from the same Flutter UI. The original native app must be removed after its behavior, assets and build responsibilities have been transferred. Historical source remains available in Git.

## Acceptance scope

- Authenticated host/API-key pairing, masked secure storage, host-scoped caches, foreground recovery and sign-out cleanup.
- Sessions/Settings/Search phone tabs, new-session capsule and working/input summary; tablet sidebar and conversation split.
- Pins, named sections, ordering/assignment, collapse, project grouping, rename, archive/restore, search and refresh.
- New conversation: host/project/checkout, remote folder browser, branch/worktree, model/effort, retained drafts and images.
- Incremental transcripts, stable scroll position, jump to latest, Markdown/code/math, text selection/copy, remote images.
- Composer queue/steer/interrupt/stop, file mentions, model/effort context, attachment upload progress and failures.
- Multi-page questions, single/multiple choice, custom/pre-filled/multiline answers and answer validation.
- Shared queue preview/actions and host edit leases; renew/release, preserve hidden attachment/Appshot context and pre-edit drafts.
- Full tool output/diff, frozen and live subagents; no request may fall back to the wrong execution host.
- Complete Luau visuals: controls, vectors/charts, canvas gestures and GPU scenes/shaders with lifecycle suspension.
- Appearance, wallpaper effects, math settings, project grouping, archive browser and notification preferences/navigation.
- Accessibility labels, system text scaling, reduced motion, keyboard avoidance and tablet keyboard navigation.
- Android build and integration tests; iOS runner/build wiring and simulator verification when macOS/Xcode is available.
- Remove legacy native iOS sources, generated Swift bindings, obsolete platform build scripts and CI references. Retain only Flutter's required iOS runner/bootstrap and portable Rust components that the new app uses.

## Existing product boundaries

Direct headless mode did not implement server-originated background APNs delivery. Local notifications while connected and permission/navigation behavior must not be described as push delivery after the OS terminates the app. No terminal, local agent, file editor or full Git panel existed in the old mobile UI.

## Design reference

Light/dark backgrounds `#F3F3F5` / `#060606`, elevated `#FFFFFF` / `#111113`, text `#27272C` / `#E8E8EA`, accent `#5B43E8` / `#8B7CF6`. Sessions use 62-point rows and 16.5-point titles; the composer uses a 50-point capsule with a 25-point radius and expands to a focused card. Glass is recreated with cross-platform blur/translucency. Geist and Geist Mono remain bundled under their OFL license.

## Implementation

The shared application is in `apps/mobile`. Android and iOS use the same Dart
screens, authenticated WebSocket controller and session behavior. The portable
Rust library retains Luau, vector/chart/canvas rendering, GPU scenes/shaders and
zatex math behind a small C ABI. Flutter paints its display lists and pixel
surfaces and exposes the interactive controls to accessibility services.

The old `apps/ios` application, generated Swift bindings, UniFFI facade and old
`scripts/ios` build entry points have been removed. Flutter's minimal iOS Runner
remains necessary for packaging, permissions and plugin registration; its
bundle identifier is still `com.rcode.ios`.

Session snapshots now store complete history in the private application support
directory, with separate host/chat identities. Cached local attempts remain
available for explicit retry; loading a cache or reconnecting never resends a
mutation automatically. Keys remain in platform secure storage.

Closing a conversation detaches its view while active sessions continue
subscribing and updating their offline snapshots. Up to six quiet sessions
remain warm, with least-recently-used eviction. Forgetting a host revokes its
session-cache and draft writers before removing their stored data, so delayed
callbacks and old page disposal cannot restore forgotten content.

Phone/tablet width changes transfer the active conversation and draft between
the route and split view. Conversation rows grow with system text size, and
highlighted code supports partial text selection and system text scaling.

## Validation evidence

On 2026-10-23, the pinned Windows Flutter/Dart toolchain passed all 80 ordinary
unit/widget tests. Five environment-gated tests were skipped in that invocation:
one real-engine integration and four native FFI tests. The unchanged 45-message
session scroll fixture also passed three consecutive isolated runs, six tests
per run. `flutter analyze --no-pub` reported no issues.

The Windows tester has an intermittent native access violation. The exact
version-gated temporary runner and the observed limits are documented in
[windows-flutter-tests.md](windows-flutter-tests.md). Those passing results use
that runner; they do not establish that the unmodified Windows tester is stable.
The installed SDK, product logic and test assertions are unchanged.

The native Rust mobile suite passed 75 tests, with one pre-existing ignored
test. A separate five-test run enabled every native FFI gate and passed against
the rebuilt Windows library, checking actual canvas geometry/colors, controls,
widget recycling, locale changes and all six zatex math fonts.

The real Rust engine/Dart gate separately exercises authenticated RPC, typed
questions, attachment round trips, shared queue editing, command deduplication
and completed turns while clients are disconnected. It uses an explicit mock
harness and no external provider credentials. Its final run passed both the
Rust gate and its Dart child, with exactly two harness runs. On Windows this
test invokes the reviewed isolated runner directly; no PATH interception or
installed-SDK change is involved.

Both Android debug APKs built successfully through Gradle's automatic Rust
integration. All packaged shared libraries have LOAD alignment of at least
16 KiB, APK ZIP alignment passes `zipalign -P 16`, the ten C ABI entry points
are present, and obsolete UniFFI exports are absent.

The x86_64 APK also passed manual operation against the real local engine on an
Android 16/API 36 emulator:

- Chinese pairing, rejected incorrect key, successful connection and credential
  recovery after process termination.
- Actual native inline/display math, interactive Luau count increment, canvas,
  3D scene and GLSL shader output.
- A 15-second task continued after the app was force-stopped; reopening restored
  the completed response without another copy of the user message.
- Typed question/answer, system image selection, conversion/upload/readback,
  full image preview and the native share sheet. No external share was sent.
- New project/model selection, first-conversation notification permission,
  Chinese settings, dark theme and phone/tablet/phone draft preservation.
- Forgetting the host returned to an empty pairing form and removed its draft,
  workspace and session-cache entries from Android private storage.

The ignored `apps/mobile/build/qa` directory contains raw widget/native logs,
Android screenshots/hierarchies and machine-readable summaries. The
`mobile_smoke` engine example supplies the deterministic model events for
repeatable local runtime checks; it does not validate a real provider account.

## Remaining platform verification

No macOS machine or macOS CI execution environment is available in this session.
The iOS project, plugin declarations and Rust build/link steps have been checked
from source, but an Xcode simulator build, device run, signing and TestFlight
upload have not been verified. CI configuration is present; its existence is
not a successful CI run. The migration is not marked fully accepted while these
platform checks remain outstanding.

Android arm64 has been compiled and package-validated, but no physical Android
device was available for this run. The manual Android pass covers the emulator
flows above; tool/subagent and shared-queue behavior rely on their dedicated
widget and real-engine tests. Local notification permission has been exercised;
the product boundary about OS-terminated push delivery still applies.
