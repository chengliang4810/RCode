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

移动端统一为 `apps/mobile` 下的 Flutter 应用，以 Android 和 iOS 为目标，通过 API 密钥连接桌面或 headless 引擎。[Linux CI 的移动核心任务](https://github.com/chengliang4810/RCode/actions/runs/39848701487/job/120068850622)在 `afee6e6` 上完成 Flutter 静态检查，87 项单元／界面测试通过、5 项环境门控跳过；真实引擎与 Dart 门控在同一任务另行通过。Rust 移动、客户端、Markdown 和文本回归共 303 项通过，移动核心占其中 75 项，另有 1 项既有忽略。Windows 原生专项 5 项通过，包含 4 项 FFI 检查和 1 项字体样式检查。Windows Flutter 测试使用版本受限的临时工具副本关闭采样分析器，安装的 SDK 和测试断言未改动。

Android arm64/x86_64 APK 均构建并通过原生库和 ZIP 的 16 KiB 对齐检查，最终 CI 包内 11 个共享库的全部 ELF LOAD 对齐均至少为 16 KiB。x86_64 包已在 Android 16/API 36 模拟器中验证鉴权连接、强制关闭后服务端继续执行与重连恢复、问答、图片上传和系统分享、真实原生绘图／数学、首次通知权限及手机／平板草稿往返。[最终 iOS CI](https://github.com/chengliang4810/RCode/actions/runs/39848701487/job/120074373888)在 `afee6e6` 上通过无签名编译及实际设备集成检查，环境为 Xcode 26.6、iOS 26.5、iPhone 11 模拟器。iPhone／iPad 实机、物理键盘、签名和 TestFlight 上传仍未执行。构建命令见 [移动端 README](../apps/mobile/README.md)，证据与边界见 [迁移记录](flutter-mobile-migration.md)。

同一设备集成门控已在 Android 和 iOS 模拟器各通过 6 项检查、各导出 5 张截图，测试体分别耗时约 32 秒和 39 秒。它覆盖真实鉴权连接和安全存储、原生数学与图形像素、交互控件、结构化问答，以及任务仍运行时断开应用连接、重建后恢复一次完成结果。发送中和恢复后的实际文字、原生画面均已逐张核对。`scripts/mobile/test-ios.sh` 的结果确认测试 fixture 已退出、专用模拟器已删除。该门控重建应用模型，Android 操作系统强制结束进程的场景由前述人工验收覆盖；模型供应商使用明确的测试实现。

Android arm64 真机 `23113RKC6C`（Android 16/API 36、SM8550/Adreno）通过同一门控：独立 `.qa` 包在约 27 秒内完成 6 项检查并导出 5 张已复核截图。手机实际内存页大小为 4096 字节；16 KiB 仅指前述包对齐检查，不能视为 16 KiB 页设备运行验证。普通 shipping 应用还完成了真实强制关闭验收：服务端于 `2026-10-23T07:10:06.199553Z` 确认任务 streaming/working 后立即 `am force-stop`，确认应用进程不存在；服务端于 `07:10:21.078918Z` 自行完成。约 `07:10:51Z` 冷启动后凭据和会话恢复，完成正文及原生图形正常；用户消息、完成回复和命令均从基线 3 增至 4，恢复后保持 4，没有重发。Android 系统 IME 输入和键盘避让也已人工验证。

本次真机验收复用 `afee6e6` 的 CI APK 与原生库，fixture 来自干净验证源码 `2d86810`，不代表验证当前工作区其他未提交 Rust 修改。原始证据位于 ignored `build/qa/physical-device/run-20261023-arm64`，包括 `device-gate` 的结果／日志／5 张 PNG，以及 `force-stop-action.json`、`force-stop-observer.jsonl`、`force-stop-restored.jsonl` 和 `09-main-restored-chat.png`。真实模型供应商、iPhone／iPad 实机、物理键盘、签名和 TestFlight 的未验证边界保持不变。
