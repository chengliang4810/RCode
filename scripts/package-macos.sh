#!/usr/bin/env bash
# macOS packaging: build the release binary for the host arch and produce
#   target/package/rcode-<version>-macos-<arch>.dmg          (user download)
#   target/package/rcode-<version>-macos-<arch>-app.tar.gz   (auto-updater)
# containing RCode.app (unsigned unless CODESIGN_IDENTITY is set).
#
# Usage: scripts/package-macos.sh
# Env:   CODESIGN_IDENTITY="Developer ID Application: …" to sign the bundle.
#        NOTARY_KEY_PATH + NOTARY_KEY_ID + NOTARY_ISSUER_ID — App Store Connect
#        API key (.p8) for notarization; all three set → notarize + staple the
#        app and the dmg, which removes the Gatekeeper warning entirely.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
command -v cargo >/dev/null 2>&1 || PATH="$HOME/.cargo/bin:$PATH"
VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')"
ARCH="$(uname -m)"
case "$ARCH" in
  arm64) TARGET=aarch64-apple-darwin ;;
  x86_64) TARGET=x86_64-apple-darwin ;;
  *) echo "Unsupported macOS architecture: $ARCH" >&2; exit 1 ;;
esac
if [[ -n "${RCODE_PACKAGE_ARCH:-}" && "$ARCH" != "$RCODE_PACKAGE_ARCH" ]]; then
  echo "Expected $RCODE_PACKAGE_ARCH runner, got $ARCH" >&2
  exit 1
fi
OUT_DIR="$ROOT/target/package"
APP="$OUT_DIR/RCode.app"
DMG="$OUT_DIR/rcode-$VERSION-macos-$ARCH.dmg"
APP_TARBALL="$OUT_DIR/rcode-$VERSION-macos-$ARCH-app.tar.gz"

cd "$ROOT"
mkdir -p "$OUT_DIR"
# codesign's AMFI parser is stricter than a general XML parser. Always give
# it Apple's canonical XML rather than a source file with CRLF or comments.
ENTITLEMENTS="$OUT_DIR/RCode.entitlements"
plutil -convert xml1 -o "$ENTITLEMENTS" "$ROOT/dist/macos/RCode.entitlements"
plutil -lint "$ENTITLEMENTS"
if [[ -n "${RCODE_PACKAGE_BIN:-}" ]]; then
  BIN="$RCODE_PACKAGE_BIN"
else
  BIN="$(python3 "$ROOT/scripts/build-package-executable.py" --target "$TARGET")"
fi
[[ "$(lipo -archs "$BIN")" == "$ARCH" ]] || { echo "Incorrect Mach-O architecture" >&2; exit 1; }
[[ "$("$BIN" --version)" == "rcode $VERSION" ]] || { echo "Incorrect executable version" >&2; exit 1; }

rm -rf "$APP" "$DMG" "$APP_TARBALL"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
install -m 755 "$BIN" "$APP/Contents/MacOS/rcode"
# Apple bundle versions are numeric; RCodeVersion retains the full beta
# version for the updater, executable and artifact names.
NUMERIC_VERSION="${VERSION%%-*}"
sed -e "s/__VERSION__/$VERSION/" -e "s/__NUMERIC_VERSION__/$NUMERIC_VERSION/" \
  "$ROOT/dist/macos/Info.plist" >"$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
mkdir -p "$APP/Contents/Resources/licenses/fonts"
cp "$ROOT/crates/ui/assets/fonts/licenses/"* "$APP/Contents/Resources/licenses/fonts/"
cp "$ROOT/crates/zatex/fonts/"*.txt "$APP/Contents/Resources/licenses/fonts/"
mkdir -p "$APP/Contents/Resources/licenses/icons"
cp "$ROOT/crates/ui/assets/file-icons/LICENSE.symbols" "$APP/Contents/Resources/licenses/icons/"
cp "$ROOT/THIRD_PARTY_NOTICES.md" "$ROOT/LICENSE" "$ROOT/LICENSE-APACHE" "$APP/Contents/Resources/licenses/"

# Icon: iconset from the pre-masked macOS icon (squircle + margins + shadow
# baked into dist/macos/icon-1024.png — sips can't alpha-mask, so the mask is
# applied ahead of time; dist/rcode.png stays the full-bleed shared artwork).
ICONSET="$OUT_DIR/rcode.iconset"
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$ROOT/dist/macos/icon-1024.png" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  retina=$((size * 2))
  sips -z "$retina" "$retina" "$ROOT/dist/macos/icon-1024.png" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/rcode.icns"
rm -rf "$ICONSET"

if [[ -n "${CODESIGN_IDENTITY:-}" ]]; then
  # Hardened runtime + secure timestamp are both notarization requirements.
  # (No --deep: Apple deprecated it; the bundle is a single Mach-O anyway.)
  codesign --entitlements "$ENTITLEMENTS" --force --options runtime --timestamp --sign "$CODESIGN_IDENTITY" "$APP"
else
  # Ad-hoc signature so the app launches on Apple silicon (Gatekeeper still
  # requires right-click → Open on first launch without notarization).
  codesign --deep --force --entitlements "$ENTITLEMENTS" --sign - "$APP"
fi

codesign --verify --deep --strict "$APP"

# notarize <path>: submit to Apple and wait for the verdict. A rejection may
# still exit 0 depending on the notarytool version — the `stapler staple` that
# follows each call has no ticket to attach then, and fails the build for us.
notarize() {
  xcrun notarytool submit "$1" \
    --key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" \
    --issuer "$NOTARY_ISSUER_ID" --wait
}
NOTARIZE=false
[[ -n "${NOTARY_KEY_PATH:-}" && -n "${NOTARY_KEY_ID:-}" && -n "${NOTARY_ISSUER_ID:-}" ]] && NOTARIZE=true

if $NOTARIZE; then
  # Staple the bundle BEFORE tarring it: the auto-updater swaps the .app with
  # no dmg involved, so the tarball copy must carry its own ticket to pass
  # Gatekeeper offline.
  ZIP="$OUT_DIR/rcode-notarize.zip"
  ditto -c -k --keepParent "$APP" "$ZIP"
  notarize "$ZIP"
  rm -f "$ZIP"
  xcrun stapler staple "$APP"
fi

# The auto-updater artifact.
tar -czf "$APP_TARBALL" -C "$OUT_DIR" RCode.app
echo "packaged: $APP_TARBALL"

# The dmg presents the classic drag-into-Applications layout over the
# ascii-hands artwork (committed renders from scripts/dmg-background.py).
# dmgbuild writes the .DS_Store (background, icon view, icon positions)
# directly — no Finder scripting, so it also works on headless CI runners.
python3 -c 'import dmgbuild' 2>/dev/null ||
  python3 -m pip install --quiet --user dmgbuild 2>/dev/null ||
  python3 -m pip install --quiet --user --break-system-packages dmgbuild

# Pair the 1x/2x background renders into a hidpi tiff so the artwork stays
# crisp on retina displays.
BG_TIFF="$OUT_DIR/dmg-background.tiff"
tiffutil -cathidpicheck "$ROOT/dist/macos/dmg-background.png" \
  "$ROOT/dist/macos/dmg-background@2x.png" -out "$BG_TIFF" >/dev/null 2>&1

APP="$APP" DMG="$DMG" BG_TIFF="$BG_TIFF" python3 - <<'PY'
import os
import dmgbuild

app = os.environ["APP"]
dmgbuild.build_dmg(
    filename=os.environ["DMG"],
    volume_name="RCode",
    settings={
        "format": "UDZO",
        "files": [app],
        "symlinks": {"Applications": "/Applications"},
        "icon": os.path.join(app, "Contents/Resources/rcode.icns"),
        "background": os.environ["BG_TIFF"],
        "show_status_bar": False,
        "show_tab_view": False,
        "show_toolbar": False,
        "show_pathbar": False,
        "show_sidebar": False,
        "default_view": "icon-view",
        # Window and icon geometry must match scripts/dmg-background.py.
        "window_rect": ((200, 120), (660, 400)),
        "icon_size": 104,
        "text_size": 12,
        "icon_locations": {"RCode.app": (165, 195), "Applications": (495, 195)},
    },
)
PY
rm -f "$BG_TIFF"
if $NOTARIZE; then
  notarize "$DMG"
  xcrun stapler staple "$DMG"
fi
hdiutil verify "$DMG"
echo "packaged: $DMG"
