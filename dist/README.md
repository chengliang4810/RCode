# Packaging

## Linux (implemented)

```sh
scripts/package-linux.sh            # release build (fat LTO, stripped)
PROFILE=debug scripts/package-linux.sh   # fast smoke package
```

Produces `target/package/rcode-<version>-linux-<arch>.tar.gz` containing:

- `rcode` — the binary (headed by default; `rcode headless --api-key <key>`
  runs the authenticated engine without a UI)
- `rcode.desktop` — XDG desktop entry template (`Exec=rcode` for packagers;
  the installers rewrite `Exec`, `TryExec`, and `Icon` to absolute paths under
  `~/.rcode/app/current`, since `~/.local/bin` is often not on a desktop
  session's `PATH`)
- `rcode.png` — 1024×1024 RCode app icon
- `install.sh` — installs into `~/.rcode/app/<version>` behind a `current`
  symlink (the curl installer's layout, which the in-app updater manages),
  links `~/.local/bin/rcode` to it, and writes the desktop entry and icon under
  `$XDG_DATA_HOME` (default `~/.local/share`). The curl installer does the same
  from the extracted tarball; `scripts/test-linux-desktop-entry.sh` checks both

The release profile in the root `Cargo.toml` sets `lto = "fat"` and
`strip = "symbols"` for distribution builds.

### Headless service configuration

`rcode headless` requires `--api-key` or `RCODE_API_KEY`. Keys have at least
8 characters, use only ASCII letters and digits, and include both. The external
WebSocket listener defaults to port `27655`; change it with `--port` or
`RCODE_REMOTE_PORT`:

```sh
rcode headless --api-key '<your-api-key>' --port 27656
```

For `dist/install.sh`, export `RCODE_API_KEY` first; `RCODE_REMOTE_PORT` is
optional. To install a service from an existing binary, set the same environment
variables and run `rcode daemon install`. By default, the Linux service reads
its saved credentials from the private `~/.rcode/env` file. A systemd user service and
linger are needed to keep it running without an active login session.

Desktop clients add `ws://server-address:27655` and the key in **Settings →
Devices**. That page also enables a desktop's external listener and generates a
12-character key on first enable. Custom keys follow the rules above. Remote
client disconnects do not stop accepted runs; quitting the desktop that hosts
the engine does stop its engine. Use the headless service for a persistent host.
The [Flutter mobile app](../apps/mobile/README.md) uses the same protocol on
Android and iOS. Widget and real-engine integration checks, Android packaging
and device checks, and unsigned Xcode/iOS simulator checks are recorded in
[development validation](../docs/development-validation.md). Physical iPhone/iPad,
signing, TestFlight and real-provider acceptance remain unverified.

## macOS

```sh
scripts/package-macos.sh    # → target/package/rcode-<version>-macos-<arch>.dmg
```

Builds the release binary, assembles `RCode.app` (Info.plist + icns), ad-hoc
signs it (set `CODESIGN_IDENTITY` for a real Developer ID), and wraps it in a
dmg. The auto-update tarball retains an internal `RCode.app` path so older
installed builds can update into RCode. The release workflow builds Intel
(`macos-15-intel`, `x86_64`) and Apple silicon (`macos-26`, `arm64`) separately,
checks each binary's architecture and version, verifies its signature and dmg,
and publishes both dmg files and app tarballs. The manual steps it automates, for reference
(run on a macOS host — gpui needs Metal; no cross-build from Linux):

1. Build the universal (or per-arch) binary:
   ```sh
   cargo build --release -p rcode --target aarch64-apple-darwin
   cargo build --release -p rcode --target x86_64-apple-darwin
   lipo -create -output rcode \
     target/aarch64-apple-darwin/release/rcode \
     target/x86_64-apple-darwin/release/rcode
   ```
2. Assemble the bundle:
   ```sh
   mkdir -p RCode.app/Contents/{MacOS,Resources}
   cp rcode RCode.app/Contents/MacOS/rcode
   version="$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')"
   sed -e "s/__VERSION__/$version/" -e "s/__NUMERIC_VERSION__/${version%%-*}/" \
     dist/macos/Info.plist > RCode.app/Contents/Info.plist
   ```
3. Icon: generate `rcode.icns` from `dist/macos/icon-1024.png` (the macOS-shaped
   variant of the artwork — squircle mask, margins, and shadow pre-baked, since
   `sips` can't apply an alpha mask) and place it at
   `RCode.app/Contents/Resources/rcode.icns`:
   ```sh
   mkdir rcode.iconset && sips -z 256 256 dist/macos/icon-1024.png --out rcode.iconset/icon_256x256.png
   iconutil -c icns rcode.iconset -o RCode.app/Contents/Resources/rcode.icns
   ```
4. Sign + notarize (required for distribution):
   ```sh
   codesign --deep --force --options runtime --sign "Developer ID Application: …" RCode.app
   xcrun notarytool submit RCode.zip --keychain-profile … --wait
   xcrun stapler staple RCode.app
   ```
5. Ship as a `.dmg` (`hdiutil create -volname RCode -srcfolder RCode.app -ov -format UDZO RCode.dmg`).

## Windows

```powershell
./scripts/package-windows.ps1 -ReleasesUrl https://github.com/chengliang4810/RCode/releases/latest/download
```

Produces, under `target/package/`:

- `rcode-<version>-windows-<arch>-setup.exe` — the per-user installer built
  from `dist/windows/rcode.iss` with Inno Setup 6
- `rcode-<version>-windows-<arch>.zip` — the portable package
- `rcode-<version>-windows-<arch>.exe` — the bare executable the in-app
  updater downloads

The installer and the zip both carry `rcode-update.json`, the marker that lets
the app update itself in place. CI runs `scripts/test-windows-installer.ps1`
against the setup on every Windows build.

## Release workflow

Use versions such as `0.0.1-beta`, then `0.0.2-beta`, with matching Git tags
`v0.0.1-beta`, `v0.0.2-beta`. The tag must match the workspace version.
Apple bundle fields and Windows executable version resources use the numeric
part (`0.0.1`); artifact names, RCode's version and update metadata retain
`0.0.1-beta`. The macOS bundle stores that full value in `RCodeVersion`.

Manually running `.github/workflows/release.yml` builds and verifies all
platform artifacts without publishing a release. Tag runs publish only after
every platform succeeds and the complete artifact set, architecture, bundle
versions and checksums have been verified. The workflow publishes ordinary releases; a `beta` version suffix does not
set GitHub's prerelease flag.
