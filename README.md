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

## High concurrency. Low memory.

### Latest local streaming stress test

**Experimental candidate: 500 concurrent streaming tasks, about 836 MiB peak client memory.** Two consecutive 500-task rounds in the same process delivered **1,000/1,000 complete responses with zero stream disconnects**.

| Round | Concurrent tasks | Peak client memory | Complete responses |
|---|---:|---:|---:|
| First | 500 | 835.71 MiB | 500/500 |
| Second, same process | 500 | 807.53 MiB | 500/500 |

Measured on Windows 11 on October 25, 2026, with a local production-profile candidate and fixed loopback SSE streams. These are client private-working-set measurements, excluding fixture, measurement and child processes—not remote-model inference results. The candidate is experimental and has not been adopted for release. [Test details and scope](docs/benchmarks/2026-10-25-windows-streaming.md).

### Real-model concurrency

**500 AI tasks running at once. Just 1.76 GiB peak client memory.** At 200 concurrent tasks, peak memory was only **763 MiB**. RCode's native Rust desktop keeps multi-task AI collaboration lightweight.

| Concurrent tasks | Peak client memory |
|---:|---:|
| 10 | 83 MiB |
| 20 | 138 MiB |
| 30 | 140 MiB |
| 50 | 326 MiB |
| 100 | 499 MiB |
| 200 | 763 MiB |
| 500 | 1.76 GiB |

Real-model testing on October 24, 2026, using a local optimized Windows release build on Windows 11 with `deepseek-v4.1-flash`. Concurrency counts active agent tasks; memory is the client's private working set, excluding remote inference. [Test environment and full results](docs/benchmarks/2026-10-24-windows-concurrency.md).

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
