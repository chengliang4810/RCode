#!/usr/bin/env bash
set -euo pipefail
native_dir="$(cd "$(dirname "$0")" && pwd)"
task_root="$(cd "$native_dir/../../.." && pwd)"
abis="${1:-arm64-v8a,x86_64}"
ndk="${2:-${ANDROID_NDK_HOME:-}}"
output="${3:-$task_root/apps/mobile/build/rust-jni}"
if [ -z "$ndk" ]; then
  sdk="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
  [ -n "$sdk" ] || { echo 'Set ANDROID_NDK_HOME or ANDROID_SDK_ROOT.' >&2; exit 1; }
  ndk="$sdk/ndk/27.0.12077973"
fi
case "$(uname -s)" in Darwin) host=darwin-x86_64;; Linux) host=linux-x86_64;; *) echo 'Unsupported NDK host' >&2; exit 1;; esac
toolchain="$ndk/toolchains/llvm/prebuilt/$host"
export ANDROID_NDK_HOME="$ndk" ANDROID_NDK_ROOT="$ndk" CARGO_INCREMENTAL=0
IFS=',' read -r -a selected <<< "$abis"
for abi in "${selected[@]}"; do
  case "$abi" in
    arm64-v8a) triple=aarch64-linux-android; compiler=aarch64-linux-android24;;
    x86_64) triple=x86_64-linux-android; compiler=x86_64-linux-android24;;
    *) echo "Unsupported native ABI: $abi" >&2; exit 1;;
  esac
  lower="${triple//-/_}"
  upper="$(printf '%s' "$lower" | tr '[:lower:]' '[:upper:]')"
  export "CARGO_TARGET_${upper}_LINKER=$toolchain/bin/$compiler-clang"
  export "CARGO_TARGET_${upper}_RUSTFLAGS=-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-lc++_shared"
  export "CC_$lower=$toolchain/bin/$compiler-clang" "CXX_$lower=$toolchain/bin/$compiler-clang++"
  export "AR_$lower=$toolchain/bin/llvm-ar" "CXXSTDLIB_$lower=c++_shared"
  rustup target add "$triple"
  (cd "$task_root" && cargo rustc --locked -p rcode-mobile --lib --crate-type cdylib --profile mobile --target "$triple")
  mkdir -p "$output/$abi"
  cp "$task_root/target/$triple/mobile/librcode_mobile.so" "$output/$abi/librcode_mobile.so"
  "$toolchain/bin/llvm-strip" --strip-unneeded "$output/$abi/librcode_mobile.so"
  cp "$toolchain/sysroot/usr/lib/$triple/libc++_shared.so" "$output/$abi/libc++_shared.so"
done
