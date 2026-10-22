# 当前开发验证

项目使用进程内 Native Agent 和用户配置的供应商。单元测试使用 MockHarness 和本地模拟服务。

## Rust

```sh
cargo check --locked --workspace --all-targets
cargo test --locked -p rcode-proto -p rcode-rpc -p rcode-harness -p rcode-engine -p rcode-client -p rcode-sync --lib
cargo test --locked -p rcode-engine --test message_queue --test restart_resume --test rich_composer_delivery
cargo test --locked -p rcode-ui --lib
```

Linux CI 的核心测试列表见 `scripts/ci/test-core.sh`。Windows 的 ConPTY 测试需要系统终端环境，macOS 的 CoreText/WebKit 和 iOS 测试需要对应平台。

## 本地网页预览

macOS 原生预览 fixture 使用独立的 Vite 开发服务器依赖：

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

GitHub Release 使用普通发布；版本名中的 `beta` 不决定 prerelease 标记。

## 移动端

iOS 包含离线演示、Rust 核心和 TestFlight 流程。在线配对需要另行提供实现注册表和会话传输协议的主机。本仓库包含客户端与测试基础设施，不提供可部署的远端服务。构建及可选在线测试见 [iOS README](../apps/ios/README.md)。
