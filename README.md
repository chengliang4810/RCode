# RCode

A native AI coding workbench, without provider lock-in.

*English | [简体中文](README.zh-CN.md) | [한국어](README.ko.md) | [日本語](README.ja.md) | [Español](README.es.md)*

![RCode feature overview (Chinese)](docs/media/readme/rcode-features.zh-CN.png)

![RCode desktop app](docs/media/readme/app-screenshot.jpg)

## Features

### No provider lock-in. Three model protocols supported.

Connect through OpenAI Chat Completions, OpenAI Responses, or Anthropic Messages. Choose the provider and model that fit your work.

### Native Rust desktop, built for faster, lighter, more memory-efficient operation

Built with Rust and GPUI, with incremental updates, virtualized rendering, and cache management to optimize responsiveness and resource use. Available on Windows, macOS, and Linux.

### Goal-driven work with parallel subagents

Keep work moving toward an explicit goal, with subagents sharing search, research, and multi-step tasks.

### Local session storage. Model connections under your control.

No RCode account required. Sessions are stored on your device, and model requests go to the provider you configure.

### Transparent usage, visible at every turn

Inspect session and per-turn tokens, cache hit rates, model-call counts, and output speed.

### MIT licensed. Inspect, modify, and build it yourself.

Access the source, customize the application, and build RCode for your own needs.

## Desktop app

Download the latest release for your platform from [GitHub Releases](https://github.com/chengliang4810/RCode/releases/latest):

- **macOS** — `rcode-<version>-macos-arm64.dmg`
- **Windows** — `rcode-<version>-windows-x86_64-setup.exe`
- **Linux** — `rcode-<version>-linux-<arch>.tar.gz`, then run its `install.sh`

No RCode account is required. Model access and updates use the network; sessions are stored locally. The app updates itself.

## Headless service and remote connections

A Linux server can run the engine without a display. Desktop clients connect with its address and API key. Set a key before the first installation:

```bash
read -r -s -p 'API Key: ' RCODE_API_KEY
printf '\n'
export RCODE_API_KEY
curl -fsSL https://github.com/chengliang4810/RCode/releases/latest/download/install.sh | sh
rcode status
```

Keys must have at least 8 characters, use only ASCII letters and digits, and include both a letter and a digit. The external port defaults to `27655`; set `RCODE_REMOTE_PORT` before installation to change it. When a systemd user service is available, the installer configures the background service and attempts to keep it running after logout. Follow its output if the service cannot start.

With an existing binary, run the engine directly. `--api-key` or `RCODE_API_KEY` is required; `--port` selects a different port:

```bash
rcode headless --api-key '<your-api-key>'
rcode headless --api-key '<your-api-key>' --port 27656
```

With those environment variables set, install and manage a persistent service:

```bash
rcode daemon install
rcode status      # engine status
rcode update      # update to the latest release
rcode daemon start|stop|restart|status
```

In the desktop app, open **Settings → Devices**, add a host, and enter `ws://server-address:27655` and the same API key. A desktop can also enable **External access** there: the first enable generates and activates a random 12-character alphanumeric key. You can replace it with a custom key that meets the rules above and save it.

A network disconnect or a remote client closing does not stop runs already accepted by the remote engine. Quitting the desktop that hosts the engine stops that machine's engine; use the headless service for a persistent server. The [Flutter mobile app](apps/mobile/README.md) supports Android and iOS using the same direct WebSocket protocol. Its screens follow the former iOS app. Flutter widget and authenticated engine integration tests have passed. Android arm64/x86_64 packaging, Android simulator and arm64 device checks, and unsigned Xcode/iOS simulator checks are recorded in [development validation](docs/development-validation.md). Physical iPhone/iPad, signing, TestFlight and real-provider acceptance remain unverified.

---

Developing or curious how it works? [Ask DeepWiki](https://deepwiki.com/chengliang4810/RCode) or check out [ARCHITECTURE.md](ARCHITECTURE.md).

Licensed under the [MIT License](LICENSE).
