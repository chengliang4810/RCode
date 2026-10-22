# Portable native rendering

Flutter uses the existing Rust Luau, chart, Canvas, scene, shader and zatex
engines through a small owned C ABI. Dart paints their display lists and takes
premultiplied BGRA pixel buffers without base64 encoding. Controls and surfaces
forward their actual gestures; offscreen/background visuals pause.

Pass a conversation identity as `RCodeVisual(scope: scope, ...)`, then call
`disposeVisualScope(scope)` when leaving the conversation. Recycled rows retain
their worker and control state; the inactive pool evicts its oldest entry after
32 workers. Unscoped widgets release their worker when disposed.

Android's `native/rust.gradle` is applied by the app Gradle file and builds
`rcode-mobile` before packaging. Supported ABIs are `arm64-v8a` and `x86_64`.
Install Rust and Android NDK `27.0.12077973`; Gradle supplies the SDK path. The
scripts install missing Rust targets and use Cargo's `mobile` profile. The
generated libraries are under `apps/mobile/build/rust-jni`, outside source control.

The iOS Runner invokes `build_ios.sh` before linking, then force-loads the
resulting static library so Dart FFI can find its C symbols. A Rust toolchain
and Xcode are required. The same source supports devices and both simulator
architectures. Signing is managed by the Flutter iOS Runner.

No remote API keys are placed in build settings or native artifacts. A visual's
source configuration exists only in memory and is disposed with its worker.

The C ownership contract is in `rcode_visual.h`. The native integration tests
run on Windows after `cargo build -p rcode-mobile --lib`: add `target/debug` to
`PATH`, set `RCODE_NATIVE_VISUAL_TEST=1`, and run
`flutter test test/visual_native_test.dart` from `apps/mobile`. They use a local
authenticated WebSocket fixture, real Luau controls, actual widget recycling,
and all six bundled math fonts.
