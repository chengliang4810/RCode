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

移动端统一为 `apps/mobile` 下的 Flutter 应用，以 Android 和 iOS 为目标，通过 API 密钥连接桌面或 headless 引擎。最终 Flutter 套件 80 项通过，5 项环境门控跳过；真实引擎与 Dart 门控另行通过，Rust 移动核心 75 项通过、1 项既有忽略，原生 FFI 专项 5 项通过。Windows Flutter 测试使用版本受限的临时工具副本关闭采样分析器，安装的 SDK 和测试断言未改动。

Android arm64/x86_64 APK 均构建并通过原生库和 ZIP 的 16 KiB 对齐检查。x86_64 包已在 Android 16/API 36 模拟器中验证鉴权连接、强制关闭后服务端继续执行与重连恢复、问答、图片上传和系统分享、真实原生绘图/数学、首次通知权限及手机/平板草稿往返。Android arm64 实机、Xcode 编译和 iOS 运行尚未验证，当前没有可用 Mac 或 macOS CI 执行环境。构建命令见 [移动端 README](../apps/mobile/README.md)，证据与边界见 [迁移记录](flutter-mobile-migration.md)。
