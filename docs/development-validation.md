# 当前开发验证

项目使用 Native 执行器和用户配置的供应商。单元测试中的 MockHarness 不要求安装外部 CLI。

## Rust

```sh
cargo check --locked --workspace --all-targets
cargo test --locked -p rcode-proto -p rcode-rpc -p rcode-harness -p rcode-engine -p rcode-client -p rcode-sync --lib
cargo test --locked -p rcode-engine --test message_queue --test restart_resume --test rich_composer_delivery
cargo test --locked -p rcode-ui --lib
```

Linux CI 的核心测试列表见 `scripts/ci/test-core.sh`。Windows 的 ConPTY 测试需要系统终端环境，macOS 的 CoreText/WebKit 和 iOS 测试需要对应平台。

## 本地网页预览

macOS 原生预览 fixture 使用独立的 Vite 开发服务器依赖，不依赖已移除的 `edge` 项目：

```sh
npm ci --prefix scripts/fixtures/preview-server
cargo build --locked -p rcode-ui --example preview-fixture --features browser-fixture
VITE_BINARY="$PWD/scripts/fixtures/preview-server/node_modules/vite/bin/vite.js" \
  scripts/ci/run-macos-fixture.sh target/debug/examples/preview-fixture /tmp/rcode-preview-captures
```

## 打包与安装脚本

```sh
python scripts/test_release_packaging.py
bash scripts/test-linux-desktop-entry.sh
```

GitHub Release 使用普通发布；版本名可包含 `beta`，发布工作流不再据此切换为 prerelease。

## 移动端

iOS 的离线演示、Rust 核心和 TestFlight 流程仍保留。在线配对需要另行提供实现注册表和会话传输协议的主机；本仓库不提供原 Cloudflare Worker 后端。构建及可选在线测试见 [iOS README](../apps/ios/README.md)。

性能文档中的旧 CLI、云端链路及脚本命令属于历史记录；不得据此启动已退役的服务或推断当前性能。
