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

移动端统一为 `apps/mobile` 下的 Flutter 应用，兼容 Android 和 iOS，通过 API 密钥连接桌面或 headless 引擎。Flutter 界面测试、真实引擎与 Dart 控制器集成、Rust visual/math 已在本机通过；Android APK 正在构建验收，Xcode 与 iOS 实机未验证。构建和真实引擎测试命令见 [移动端 README](../apps/mobile/README.md)。
