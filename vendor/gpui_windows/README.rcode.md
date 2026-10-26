# RCode Windows text backend patch

Source: `zeronsh/zui`, commit `361ff20b919fdfb80199ee6d9aed725300cb7d54`,
`crates/gpui_windows` (Apache-2.0). The license is included beside this file.

The source is retained here so released builds include the fix without modifying
Cargo's checkout cache. Cargo.toml expands the upstream workspace dependencies;
path dependencies still use the same pinned Git revision.

Path rendering allocates its full-viewport resolve and 4× MSAA textures only
when a non-empty path batch is drawn. Resize and device recovery reset these
resources; the next path draw recreates them at the current viewport size.
Sampling and pixel formats are unchanged. This avoids unused texture commitment
on path-free windows, not the memory needed by windows that actually draw paths.

Text fixes in `src/direct_write.rs`:

- In-memory fonts use a COM owner to retain borrowed static bytes or owned font
  allocations without DirectWrite copying them. Native stream tests verify the
  original pointer remains valid after the local owner and font file are dropped.

- Font fallback mappings search the process's custom font collection before the
  system collection. Bundled Sarasa UI SC can supply Chinese inside Geist/Geist
  Mono without installing fonts in Windows.
- Explicit emoji keycaps (`#`, `*`, or a digit followed by VS16 and U+20E3) shape
  together using the existing Windows Segoe UI Emoji face. Normal digits and
  VS15 text presentation keep the requested font. This repairs the native
  DirectWrite fallback that otherwise separates the base digit and keycap mark.
- Glyph index zero is reported as missing, and final line metrics are measured
  after font-range assignment. No new font or UI font preference is introduced.

IME fixes in `src/events.rs`:

- Both cancellation notifications delete only an existing marked UTF-16 range.
  A late or repeated cancellation cannot erase an ordinary text selection.
- Failed/empty composition-attribute reads return before allocating or indexing.
  Positive short reads inspect only the returned prefix; unread zero padding is
  never mistaken for `ATTR_INPUT`.

Regression tests use native DirectWrite and, with `test-support`, a real GPUI
`PlatformInputHandler` adapter around an isolated UTF-16 editor. No account,
network, installed-font mutation or real message submission is involved:

```powershell
cargo test --manifest-path vendor/gpui_windows/Cargo.toml --offline native_directwrite -- --nocapture
cargo test --manifest-path vendor/gpui_windows/Cargo.toml --offline explicit_keycap_ranges
cargo test --manifest-path vendor/gpui_windows/Cargo.toml --offline font_data::tests
cargo test --manifest-path vendor/gpui_windows/Cargo.toml --offline --features test-support ime_
```

The DirectWrite test records ZWJ glyphs/clusters as well. The local `👩‍💻` sample
already forms one native cluster; this patch does not claim that its upstream
UI rendering, every Emoji sequence, or every IME was verified.

Remove this patch when the pinned upstream backend includes the same behavior.

Idle frame scheduling uses GPUI's `frame_requester` and `pause_frame_requests`
hooks. Each window has independent frame demand; the VSync loop invalidates
only requesting windows. Direct Manipulation running/inertia states and queued
gesture events keep updates active, and touchpad contact and restore wake the
window. Native paint and forced device recovery paths remain available. State
tests cover pause/resume and independent windows; native interaction and CPU
measurements are required before claiming a runtime improvement.

The VSync provider waits on frame-demand notifications while all windows are
idle, with a one-second device-health watchdog. New windows and new frame demand
wake it without reducing the active animation refresh rate. Synchronization tests
cover requests arriving at the idle transition and the watchdog timeout.

Minimized windows separately suspend frame delivery: background invalidations
remain pending without reactivating the VSync loop, and restore requests a frame.
Repeated minimize events preserve the saved callback. Installed-app measurements
on this machine reduced the VSync thread's context switches from about 148/s to
2/s while minimized; this is not the application's total CPU percentage.

After a window shrinks, a two-second timer waits until frame demand and Direct
Manipulation are idle, then runs DXGI `Trim` once. Pipeline bindings are cleared
and the rasterizer state restored; the presented image and four-sample path
antialiasing are preserved. This releases driver/runtime high-water caches without continuously
trimming during drawing. The installed-app maximize/restore check reduced settled
private commit from roughly 336 MiB to 122 MiB on the tested Windows/AMD system;
this is not a cross-device guarantee or a measured first-frame latency bound.

Path intermediate textures are also released by the idle trim after two seconds
without another path draw. The presented swap-chain image remains intact; the
next path draw recreates the resolve and 4x MSAA resources at the current size.
The timer still waits for pending frames and manipulation to settle. Pixel tests
cover idle release, the unchanged presented image, and drawing after recreation.
