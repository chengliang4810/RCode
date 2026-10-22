# RCode

不绑定供应商的原生 AI 编程工作台。

*[English](README.md) | 简体中文 | [한국어](README.ko.md) | [日本語](README.ja.md) | [Español](README.es.md)*

![RCode 六项特点：模型协议兼容、原生桌面、子智能体协作、本地存储、用量统计和 MIT 开源](docs/media/readme/rcode-features.zh-CN.png)

![RCode 桌面应用](docs/media/readme/app-screenshot.jpg)

## 特点

### 不绑定供应商，兼容三大模型协议

支持 OpenAI Chat Completions、OpenAI Responses、Anthropic Messages，自由选择服务商与模型。

### Rust 原生桌面，为更快、更轻、更省内存而构建

以 Rust 与 GPUI 构建原生应用，结合增量更新、虚拟化渲染与缓存管理，优化响应速度和资源占用。支持 Windows、macOS、Linux。

### 目标驱动，子智能体并行协作

围绕明确目标持续推进，支持子智能体分工处理搜索、研究与多步骤任务。

### 会话本地存储，模型连接自主掌控

无需 RCode 账号，会话保存在本机，模型请求发往你配置的服务商。

### 用量透明，消耗清晰可见

会话与单轮 Token、缓存命中率、请求步数、输出速度，随时查看。

### MIT 开源，代码与部署自由掌控

支持源码查看、修改和自行构建，按自己的需求使用和定制。

## 桌面应用

从 [GitHub Releases](https://github.com/chengliang4810/RCode/releases/latest) 下载对应平台的最新版本：

- **macOS** — `rcode-macos-arm64.dmg`
- **Windows** — `rcode-windows-x86_64-setup.exe`
- **Linux** — `rcode-linux-<arch>.tar.gz`，解压后运行里面的 `install.sh`

无需 RCode 账号。模型访问和应用更新需要联网，会话保存在本机。应用支持自动更新。

## 无界面服务与远程连接

Linux 服务器可以运行无界面的引擎，桌面端通过服务器地址和 API Key 连接。首次安装前设置密钥：

```bash
read -r -s -p 'API Key: ' RCODE_API_KEY
printf '\n'
export RCODE_API_KEY
curl -fsSL https://github.com/chengliang4810/RCode/releases/latest/download/install.sh | sh
rcode status
```

密钥至少 8 位，只能包含 ASCII 字母和数字，且两类都必须包含。默认对外端口为 `27655`；安装前设置 `RCODE_REMOTE_PORT` 可以更换端口。安装脚本在 systemd 用户服务可用时配置后台服务，并尝试启用退出登录后继续运行；未能启动服务时按脚本提示处理。

已有二进制可以直接运行；必须提供 `--api-key` 或环境变量 `RCODE_API_KEY`，端口可用 `--port` 指定：

```bash
rcode headless --api-key '<你的密钥>'
rcode headless --api-key '<你的密钥>' --port 27656
```

也可以在设置好上述环境变量后安装和管理常驻服务：

```bash
rcode daemon install
rcode status      # 查看引擎状态
rcode update      # 更新到最新版本
rcode daemon start|stop|restart|status
```

在桌面端的「设置 → 设备」中添加主机，填写 `ws://服务器地址:27655` 和同一个 API Key。另一台桌面也可在此启用「外部访问」：首次启用会立即生成并启用 12 位随机字母数字密钥，也可修改为符合上述规则的自定义密钥并保存。

网络断开或远程客户端退出，不会停止远端引擎已经接收的运行；退出承载引擎的桌面应用会停止该机器上的引擎，服务器常驻应使用无界面服务。[Flutter 移动端](apps/mobile/README.md) 使用同一套直连 WebSocket 协议，兼容 Android 和 iOS，界面及流程参考原 iOS 应用。Flutter 界面与真实引擎集成测试已通过；Android 打包正在验收，Xcode 编译与 iOS 实机运行尚未验证。

---

想参与开发，或者好奇它怎么跑起来的？[Ask DeepWiki](https://deepwiki.com/chengliang4810/RCode)，也可以看 [ARCHITECTURE.md](ARCHITECTURE.md)。

采用 [MIT License](LICENSE)。
