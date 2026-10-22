#!/usr/bin/env bash
set -euo pipefail
native_dir="$(cd "$(dirname "$0")" && pwd)"
task_root="$(cd "$native_dir/../../.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH" CARGO_INCREMENTAL=0
case "${PLATFORM_NAME:-iphoneos}" in
  iphonesimulator) sdk=iphonesimulator; suffix=-sim;;
  iphoneos) sdk=iphoneos; suffix=;;
  *) echo 'RCode native visuals require an iOS device or simulator target.' >&2; exit 1;;
esac
export SDKROOT="$(xcrun --sdk "$sdk" --show-sdk-path)"
export IPHONEOS_DEPLOYMENT_TARGET="${IPHONEOS_DEPLOYMENT_TARGET:-15.0}"
output="${BUILT_PRODUCTS_DIR:-$task_root/apps/mobile/build/ios-native}"
mkdir -p "$output"
libraries=()
for arch in ${ARCHS:-arm64}; do
  case "$arch:$suffix" in
    arm64:) triple=aarch64-apple-ios;;
    arm64:-sim) triple=aarch64-apple-ios-sim;;
    x86_64:-sim) triple=x86_64-apple-ios;;
    *) echo "Unsupported iOS architecture: $arch" >&2; exit 1;;
  esac
  rustup target add "$triple"
  (cd "$task_root" && cargo rustc --locked -p rcode-mobile --lib --crate-type staticlib --profile mobile --target "$triple")
  libraries+=("$task_root/target/$triple/mobile/librcode_mobile.a")
done
xcrun lipo -create "${libraries[@]}" -output "$output/librcode_mobile.a"
