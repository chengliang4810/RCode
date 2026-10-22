# RCode for iOS

A UIKit viewport onto the RCode mesh, built on the Rust mobile core. The phone
is a **peer device**: it mirrors the workspace registry, joins per-chat session
rooms, and drives remote engines through the durable command ledger. No agent
runs on the phone.

**Rust decides what to paint and where; Swift paints, scrolls and handles
gestures.** Everything in Rust is platform-neutral — the future Android app
links the same library. See [`docs/mobile-rewrite.md`](../../docs/mobile-rewrite.md).

## Build & run

Requires Xcode 26+ and a Rust toolchain with the iOS targets:

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
cd apps/ios
xcodebuild -project RCode.xcodeproj -scheme RCode \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' build
```

The RCode target's **Rust core** build phase runs `scripts/ios/build-core.sh`,
which builds `crates/mobile` for the active platform (always optimized — the
`mobile` cargo profile) and regenerates the ignored UniFFI bindings in
`RCode/Core/Generated/`. `RCODE_SKIP_CORE=1` reuses the last built library
when iterating on Swift only.

## Layout

```
RCode/
  App/         AppDelegate, SceneDelegate, AppModel (owns CoreClient; maps
               workspace snapshots to view models; Keychain credentials)
  Core/        Generated UniFFI bindings; Fonts (the exact bytes Rust measures,
               registered with CoreText) + CoreText fallback measurer
  Design/      Palette (RCode color roles → light/dark), Glass helpers,
               cell-grid status glyph, harness brand marks
  Transcript/  TranscriptListView (virtualized scroll host over LayoutFrame),
               RowView/RowModel (CoreText painter for Rust display lists,
               streaming veil), FrameRelay
  Session/     SessionViewController, SessionSource (Core/Fixture), new-session
               canvas
  Composer/    ComposerBar (glass capsule ⇄ card with inline context chips;
               send/queue/steer/stop), question and queue panels, attachments
  Threads/     Sessions (foldable sidebar sections)/folder/search lists,
               cells, new-project folder browser
  Shell/       Tab bar (Sessions, Settings, search; "New session" accessory
               with live summary), pairing
  Debug/       Transcript lab + hitch meter
```

### Transcript pipeline

1. `rcode-client` applies session-doc updates incrementally (O(changed
   entries)) and publishes immutable snapshots.
2. `TranscriptView.attach(client, chatId)` subscribes **in Rust**; the layout
   thread turns entries into rows (one per markdown block / user message / tool
   group), reparses streaming markdown incrementally, and measures every row
   with `rcode-text` at the viewport width.
3. Each pass publishes a `LayoutFrame`: exact heights + prefix-sum offsets.
   `rowsIn(y0, y1)` is a binary search; `display(i)` builds a display list
   (text runs at exact positions, boxes, links, scrollers, widgets).
4. `TranscriptListView` positions reusable `RowView`s at those offsets and
   paints runs with CoreText at the Rust coordinates — measurement and
   rendering can't disagree. Display models for rows beyond the viewport are
   prefetched off the main thread.
5. Agent visuals (`show_visual` Luau scripts) get a worker thread each
   (`crates/mobile/src/visual`): it runs the script, lays the node tree out,
   draws canvases and charts as vector paint, renders scenes and shaders with
   wgpu, and reports its height back to the row. `VisualView` paints what
   arrives (rasterized on the worker thread) and forwards touches; the demo's
   "Agent visuals" chat (`-demo -route chat:chat-visuals`) has one of each.

## Launch arguments

| Arg | Effect |
| --- | --- |
| `-demo` | Offline demo workspace (Rust `DemoHost`: registry, docs, streaming replies) |
| `-fast` / `-longreply` | Demo stream speed / reply length |
| `-big` / `-huge` | Demo transcripts with 120 / 600 turns |
| `-route chat:<id>` / `new` / `more` / `search` | Open a screen at launch |
| `-signedout` | Clear stored credentials (Debug builds only) |
| `-pair <host> <userId> <orgId>` | Pair with a supplied host and its `userId@orgId` bearer (Debug builds only) |
| `-harness native` | Use RCode's in-process agent for new sessions |
| `-bench` (with `-lab`) | Display-link fling benchmark; writes `Documents/bench.json` |
| `-measureopen` | Session open latency; writes `Documents/open.json` |
| `-lab [-turns N] [-autostream] [-autoscroll] [-top] [-meter]` | Transcript lab over fixture markdown, with an on-screen hitch meter |

## Tests

```sh
# Rust
cargo test -p rcode-text -p rcode-markdown -p rcode-client -p rcode-mobile
cargo test --release -p rcode-mobile --lib bench_layout -- --ignored --nocapture

# Line-break accuracy vs CoreText (same font bytes), flows, hitch benchmarks
xcodebuild … -only-testing:RCodeTests/LineBreakAccuracyTests test
xcodebuild … -only-testing:RCodeUITests/SessionFlowTests test
xcodebuild … -only-testing:RCodeUITests/ScrollPerformanceTests test
```

Online integration requires a separately provisioned, reachable host that
implements the registry/session transport and a configured Native provider.
This repository does not ship the former Cloudflare Worker service.
`rcode headless` alone is not a replacement for that service. Pair in the app
using the host address and token; the Debug hook is `-pair host userId orgId`.

The opt-in test in `RCodeUITests/LiveStackTests.swift` uses those same inputs:

```sh
TEST_RUNNER_RCODE_LIVE_HOST=https://your-host.example \
TEST_RUNNER_RCODE_LIVE_PAIR_TOKEN="$PAIR_TOKEN" \
  xcodebuild … -only-testing:RCodeUITests/LiveStackTests test
```

This test sends a real prompt to the host's configured Native provider. It
skips when pairing inputs are absent. The offline `-demo` fixtures remain
available without a host or provider credentials.

## Session notifications

The client retains notification permissions, APNs token registration and
notification navigation. Remote delivery requires the paired host to implement
`POST /registry/:org/push-target` and APNs delivery; that backend is not included
here. The former Worker deployment and `wrangler` instructions no longer
apply. The TestFlight workflow still configures the app's Push capability.

## TestFlight release

Run the **TestFlight** workflow from GitHub Actions on `main`. It installs the
Rust iOS targets, selects the next build number from App Store Connect,
archives (device, Release — the Rust core builds in the archive) with
automatic signing, and uploads a TestFlight build (internal, or external with
beta review). Secrets: `AC_API_KEY_P8`, `AC_API_KEY_ID`,
`AC_API_ISSUER_ID`.
