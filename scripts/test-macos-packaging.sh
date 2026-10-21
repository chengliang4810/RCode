#!/usr/bin/env bash
# Exercise the real bundle/sign/dmg pipeline with a tiny native executable.
# All output stays in a disposable sandbox, separate from release artifacts.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SANDBOX="$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/rcode-package-smoke.XXXXXX")"
trap 'rm -rf "$SANDBOX"' EXIT
mkdir -p "$SANDBOX/scripts" "$SANDBOX/dist" "$SANDBOX/crates/ui/assets/fonts/licenses"
cp "$ROOT/Cargo.toml" "$ROOT/LICENSE" "$ROOT/LICENSE-APACHE" "$ROOT/THIRD_PARTY_NOTICES.md" "$SANDBOX/"
cp "$ROOT/scripts/package-macos.sh" "$SANDBOX/scripts/"
cp -R "$ROOT/dist/macos" "$SANDBOX/dist/"
cp "$ROOT/crates/ui/assets/fonts/licenses/"* "$SANDBOX/crates/ui/assets/fonts/licenses/"
VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')"
printf 'fn main() { println!("rcode %s"); }\n' "$VERSION" > "$SANDBOX/smoke.rs"
rustc "$SANDBOX/smoke.rs" -o "$SANDBOX/rcode-smoke"
CODESIGN_IDENTITY= NOTARY_KEY_PATH= NOTARY_KEY_ID= NOTARY_ISSUER_ID= \
  RCODE_PACKAGE_BIN="$SANDBOX/rcode-smoke" bash "$SANDBOX/scripts/package-macos.sh"
ARCH="$(uname -m)"
DMG="$SANDBOX/target/package/rcode-$VERSION-macos-$ARCH.dmg"
MOUNT="$SANDBOX/mounted"
mkdir "$MOUNT"
hdiutil attach -readonly -nobrowse -mountpoint "$MOUNT" "$DMG"
trap 'hdiutil detach "$MOUNT" >/dev/null 2>&1 || true; rm -rf "$SANDBOX"' EXIT
[[ "$("$MOUNT/RCode.app/Contents/MacOS/rcode" --version)" == "rcode $VERSION" ]]
[[ "$(lipo -archs "$MOUNT/RCode.app/Contents/MacOS/rcode")" == "$ARCH" ]]
[[ "$(readlink "$MOUNT/Applications")" == /Applications ]]
codesign --verify --deep --strict "$MOUNT/RCode.app"
hdiutil detach "$MOUNT"
trap 'rm -rf "$SANDBOX"' EXIT
echo "PASS: $ARCH bundle, entitlements, signature, dmg mount and executable version"
