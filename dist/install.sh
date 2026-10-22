#!/bin/sh
# RCode (native) headless installer.
#
#   curl -fsSL https://github.com/chengliang4810/RCode/releases/latest/download/install.sh | sh
#
# Installs the native binary (requires the system ALSA runtime) to
# ~/.rcode/app, puts `rcode` on PATH, adds a launcher entry and icon under
# $XDG_DATA_HOME (default ~/.local/share), and runs it as a local-only
# systemd user service that survives reboots. Re-running upgrades in place;
# ~/.rcode state is preserved. Model providers are configured in RCode.
set -eu

BASE="${RCODE_BASE_URL:-https://github.com/chengliang4810/RCode/releases/latest/download}"

# --- platform ---------------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux) plat=linux ;;
  Darwin)
    echo "rcode install: on macOS, download the desktop app instead:" >&2
    echo "  $BASE/latest.txt → $BASE/rcode-<version>-macos-arm64.dmg" >&2
    exit 1
    ;;
  *)
    echo "rcode install: unsupported OS '$os' — only Linux for now." >&2
    exit 1
    ;;
esac
case "$arch" in
  x86_64 | amd64) arch=x86_64 ;;
  aarch64 | arm64) arch=aarch64 ;;
  *)
    echo "rcode install: unsupported architecture '$arch'." >&2
    exit 1
    ;;
esac

# --- download ----------------------------------------------------------------
ver="$(curl -fsSL "$BASE/latest.txt" | tr -d '[:space:]')"
[ -n "$ver" ] || { echo "rcode install: could not resolve latest version" >&2; exit 1; }
file="rcode-$ver-$plat-$arch.tar.gz"
data_root="$HOME/.rcode"
app_root="$data_root/app"
dest="$app_root/$ver"

if [ -x "$dest/rcode" ]; then
  echo "rcode $ver already downloaded — relinking."
else
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  echo "downloading rcode $ver ($plat-$arch)…"
  curl -fSL --progress-bar "$BASE/$file" -o "$tmp/$file"
  mkdir -p "$dest"
  tar -xzf "$tmp/$file" -C "$dest" --strip-components=1
fi

# Probe before changing a working installation or starting its service. The
# desktop and headless modes share one binary, including CPAL's ALSA linkage.
if ! "$dest/rcode" --version >/dev/null; then
  echo "rcode install: the downloaded executable could not start; see the loader error above." >&2
  echo "Install the missing runtime libraries (including ALSA, libasound.so.2), then rerun this installer." >&2
  exit 1
fi

ln -sfn "$dest" "$app_root/current"
mkdir -p "$HOME/.local/bin"
ln -sf "$app_root/current/rcode" "$HOME/.local/bin/rcode"

# --- desktop entry -------------------------------------------------------------
# Launchers list RCode through a per-user .desktop entry. The one in the tarball
# says `Exec=rcode` and `TryExec=rcode`, which only resolve when ~/.local/bin is
# on the PATH of the desktop session (often not, e.g. a bare Wayland + fuzzel
# setup) and TryExec then hides the entry outright. So write it with absolute
# paths through the `current` symlink, which keeps working across updates. The
# icon is referenced by path too: the only artwork is 1024x1024, a size the
# hicolor theme doesn't index, so a name lookup alone can come up empty.
# (Duplicated in scripts/package-linux.sh's install.sh; keep the two in sync.)
install_desktop_entry() {
  src="$1"
  app="$2"
  [ -f "$src/rcode.desktop" ] && [ -f "$src/rcode.png" ] || return 1
  case "${XDG_DATA_HOME:-}" in
    /*) data_home="$XDG_DATA_HOME" ;;
    *) data_home="$HOME/.local/share" ;;
  esac
  apps_dir="$data_home/applications"
  icon_dir="$data_home/icons/hicolor/1024x1024/apps"
  bin="$app/current/rcode"
  icon="$app/current/rcode.png"
  # Desktop Entry `Exec` quoting: double-quote an argument with reserved
  # characters, backslash-escape ", `, $ and \ inside, then double every
  # backslash again for the file's own string escaping. `%` must be `%%`.
  case "$bin" in
    *[!A-Za-z0-9_./-]*)
      exec_bin="\"$(printf '%s' "$bin" | sed -e 's/\\/\\\\\\\\/g' -e 's/["`$]/\\\\&/g' -e 's/%/%%/g')\""
      ;;
    *) exec_bin="$bin" ;;
  esac
  try_bin="$(printf '%s' "$bin" | sed 's/\\/\\\\/g')"
  icon_val="$(printf '%s' "$icon" | sed 's/\\/\\\\/g')"

  mkdir -p "$apps_dir" "$icon_dir" || return 1
  # Write beside the final name, then rename, so a launcher watching the
  # directory never reads a half-written entry (a leading dot is ignored).
  # Not `tmp`: sh has no `local`, and the curl installer's EXIT trap removes
  # its download dir through `$tmp`.
  entry_tmp="$apps_dir/.rcode.desktop.$$"
  while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
      Exec=*) printf 'Exec=%s %%u\n' "$exec_bin" ;;
      TryExec=*) printf 'TryExec=%s\n' "$try_bin" ;;
      Icon=*) printf 'Icon=%s\n' "$icon_val" ;;
      *) printf '%s\n' "$line" ;;
    esac
  done <"$src/rcode.desktop" >"$entry_tmp" || { rm -f "$entry_tmp"; return 1; }
  mv -f "$entry_tmp" "$apps_dir/rcode.desktop" || { rm -f "$entry_tmp"; return 1; }
  cp "$src/rcode.png" "$icon_dir/.rcode.png.$$" \
    && mv -f "$icon_dir/.rcode.png.$$" "$icon_dir/rcode.png" || return 1

  # Best-effort cache refresh; both tools are optional. The icon cache is only
  # refreshed, never created: a user-level hicolor cache nobody else maintains
  # would hide icons other apps later install there, and the entry above
  # references the icon by path anyway.
  command -v update-desktop-database >/dev/null 2>&1 \
    && update-desktop-database "$apps_dir" >/dev/null 2>&1 || true
  [ -f "$data_home/icons/hicolor/icon-theme.cache" ] \
    && command -v gtk-update-icon-cache >/dev/null 2>&1 \
    && gtk-update-icon-cache -q -t -f "$data_home/icons/hicolor" >/dev/null 2>&1 || true
  return 0
}
# A missing desktop entry must never fail an otherwise good install.
install_desktop_entry "$dest" "$app_root" \
  || echo "warn: could not install the desktop entry — RCode won't appear in application launchers"

# --- service -----------------------------------------------------------------
# The daemon serves the local profile. Provider credentials are configured
# inside RCode.

service=manual
if command -v systemctl >/dev/null 2>&1 && [ -n "${XDG_RUNTIME_DIR:-}" ]; then
  mkdir -p "$HOME/.config/systemd/user"
  cat >"$HOME/.config/systemd/user/rcode.service" <<'UNIT'
[Unit]
Description=RCode native headless engine
After=network-online.target
StartLimitIntervalSec=60
StartLimitBurst=5

[Service]
ExecStart=%h/.rcode/app/current/rcode headless
Restart=on-failure
RestartSec=5
EnvironmentFile=-%h/.rcode/env

[Install]
WantedBy=default.target
UNIT
  systemctl --user daemon-reload
  systemctl --user enable rcode
  systemctl --user restart rcode
  service=running
  # Keep the user manager (and the engine) running without an active login.
  loginctl enable-linger "$USER" 2>/dev/null \
    || sudo -n loginctl enable-linger "$USER" 2>/dev/null \
    || echo "warn: could not enable linger — the engine stops when you log out (run: sudo loginctl enable-linger $USER)"
else
  echo "warn: systemd user session not available — run the engine manually with: rcode headless"
fi

case ":$PATH:" in
  *":$HOME/.local/bin:"*) path_hint="" ;;
  *) path_hint=' (add ~/.local/bin to your PATH)' ;;
esac

echo ""
echo "✓ rcode $ver installed$path_hint"
echo ""
case "$service" in
  running)
    echo "the local engine is running with the new version."
    echo "  systemctl --user status rcode    check the service"
    ;;
  manual)
    echo "next: run the local-only engine with \`rcode headless\`."
    ;;
esac
