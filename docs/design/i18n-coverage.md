# Internationalization coverage

The desktop UI uses `rcode-presentation` resources for English (`en-US`) and
Simplified Chinese (`zh-CN`). The Flutter Android/iOS application uses
`apps/mobile/lib/l10n.dart`, the app language preference and Flutter localization
delegates. Model names, user content and remote tool output remain source data.

The October 2026 coverage pass includes command search, updates, settings,
conversation actions and notifications, PR assessments, file operations,
terminal and browser failures, Markdown controls, and iOS navigation, actions,
accessibility labels and permission descriptions. This is a coverage inventory,
not a claim that every operating-system or third-party diagnostic is translated.

## Display boundaries

- Keep protocol values, persisted enums, element IDs and search-result identity
  independent of translated labels. Command search accepts both the displayed
  command and its stable English name.
- Localize complete sentences and pass paths, names and numbers as parameters.
  Quantity messages use numeric Fluent arguments or native plural resources.
- PR assessment labels belong in `presentation::assessment`; model handoff
  prompts continue to use stable protocol facts.
- Application-owned workspace and terminal failures carry stable typed codes
  through the existing RPC error string. `presentation::workspace` formats them
  for display. Preserve native diagnostics and recovery paths; retain errors
  from older hosts when no known code can be decoded.
- The mobile FFI recognizes the same codes, including relay method wrappers,
  before exposing an error to native UI. The error category remains unchanged.
- Browser host failures use `BrowserError`, and Linux startup exchanges stable
  profile failure codes. Website content and WebView/native diagnostics remain
  source data.
- Flutter resolves application labels and typed errors through `BuildContext`
  localization helpers; changing the language rebuilds the current locale.
  Platform permission descriptions belong to the Android/iOS runners.

Older UIs do not understand the new typed error payloads; coordinated UI/host
updates are needed to obtain localized errors in that direction. New UIs retain
the original diagnostic when talking to older hosts.

## Checks

```powershell
cargo check --workspace --all-targets --locked --offline
cargo test -p rcode-presentation -p rcode-proto -p rcode-mobile --lib --locked --offline
cargo test -p rcode-ui --lib --locked --offline
cargo test -p rcode-engine --lib workspace_files --locked --offline
cargo test -p rcode-engine --test workspace_files --test projectless --locked --offline
cd apps/mobile
flutter test --no-pub --concurrency=1
cd ../..
git -c core.whitespace=cr-at-eol diff --check
```

The presentation build rejects invalid Fluent syntax, duplicate keys and
cross-language key/argument drift. Presentation tests check lookup and display
formatting; UI tests cover the affected interactions and stable identities.
Flutter widget tests exercise Chinese/English labels and language switching.
The native iOS resource checks listed below describe the historical app; they
do not validate the replacement Flutter runner.

Windows verification on 2026-10-22:

| Suite | Passed | Ignored |
|---|---:|---:|
| Desktop UI library | 1,797 | 5 |
| Presentation library | 44 | 0 |
| Protocol library | 86 | 0 |
| Mobile Rust library | 71 | 1 |
| Engine library | 381 | 2 |
| Workspace files and projectless integration | 11 | 0 |

The all-target workspace check passed. Native iOS resource validation passed
with 228 text keys and three plural messages. Windows test fixtures use an
executed readiness marker instead of a fixed PowerShell prompt, and a native
home-directory fallback; temporary Git fixtures fix their line-ending policy.

Resource tests cannot prove that every display path calls a localizer. Continue
checking visible labels, placeholders, tooltips, accessibility names, native
dialogs and notification text, including loading, empty and error states.
Runtime acceptance on iOS/macOS/Linux, native permission prompts and translated
layout still requires builds on those platforms.
