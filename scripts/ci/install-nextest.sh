#!/usr/bin/env bash
# Install a pinned cargo-nextest release (Linux and Windows, x86_64) after
# verifying its SHA-256, instead of trusting a third-party install action.
# To bump: change VERSION and the per-platform SHA256 values (read them off the
# release's assets, not from an install log).
set -euo pipefail
VERSION=0.9.146
SHA256=682c21b777c333e96fd532e114d3a5a894e0729ab88d94c0a9f20f8419695428
# Windows pin. Computed locally when the platform was added: the release API
# was rate-limited, so it could not be read off the release page. Confirm it
# there before treating it as trusted.
SHA256_WINDOWS=0fa689815c8157e4633225b6b173184b3d546eb6ffb754c3d0e6ea5973284a20

case "$(uname -s -m)" in
  "Linux x86_64")
    ASSET="cargo-nextest-$VERSION-x86_64-unknown-linux-gnu.tar.gz"
    EXPECTED="$SHA256"
    ;;
  "MINGW64_NT x86_64" | "MSYS_NT x86_64" | "CYGWIN_NT x86_64")
    ASSET="cargo-nextest-$VERSION-x86_64-pc-windows-msvc.zip"
    EXPECTED="$SHA256_WINDOWS"
    ;;
  *) echo "install-nextest.sh: unsupported $(uname -s -m)" >&2; exit 1 ;;
esac
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl --proto '=https' --tlsv1.2 -fsSL --retry 5 -o "$tmp/nextest.archive" \
  "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-$VERSION/$ASSET"
echo "$EXPECTED  $tmp/nextest.archive" | sha256sum -c -
mkdir -p "$HOME/.cargo/bin"
case "$ASSET" in
  *.tar.gz)
    tar -xzf "$tmp/nextest.archive" -C "$HOME/.cargo/bin" cargo-nextest
    ;;
  *.zip)
    unzip -o -q "$tmp/nextest.archive" -d "$tmp/unpacked"
    cp "$tmp/unpacked/cargo-nextest.exe" "$HOME/.cargo/bin/"
    ;;
esac
# Verify through the plugin name, not the binary: a Windows `cargo` looks for
# `cargo-nextest` on the PATH it inherited from the environment rather than this
# shell's, so running the binary directly reports success while `cargo nextest`
# would still fail with "unrecognized subcommand".
cargo nextest --version
