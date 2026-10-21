# RCode

A local-first coding agent that runs in this process and talks to whichever model provider you configure. Sessions stay on this device — there is no account and nothing is uploaded.

*English | [简体中文](README.zh-CN.md) | [한국어](README.ko.md) | [日本語](README.ja.md) | [Español](README.es.md)*

![RCode desktop app](docs/media/readme/app-screenshot.jpg)

## Desktop app

Download the latest release for your platform from [GitHub Releases](https://github.com/chengliang4810/RCode/releases/latest):

- **macOS** — `rcode-macos-arm64.dmg`
- **Windows** — `rcode-windows-x86_64-setup.exe`
- **Linux** — `rcode-linux-<arch>.tar.gz`, then run its `install.sh`

No account or network connection is needed; sessions stay on your device. The app updates itself.

## Headless (CLI)

For servers and other machines without a display, such as a VPS that keeps agents running after you close your laptop. Linux only:

```bash
curl -fsSL https://github.com/chengliang4810/RCode/releases/latest/download/install.sh | sh
rcode status
```

The installer starts the engine as a background service that survives reboots.

```bash
rcode status      # engine status
rcode update      # update to the latest release
rcode daemon start|stop|restart|status
```

---

Developing or curious how it works? [Ask DeepWiki](https://deepwiki.com/chengliang4810/RCode) or check out [ARCHITECTURE.md](ARCHITECTURE.md).

Licensed under the [MIT License](LICENSE).
