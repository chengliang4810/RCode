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

- **macOS** — `rcode-macos-arm64.dmg`
- **Windows** — `rcode-windows-x86_64-setup.exe`
- **Linux** — `rcode-linux-<arch>.tar.gz`, then run its `install.sh`

No RCode account is required. Model access and updates use the network; sessions are stored locally. The app updates itself.

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
