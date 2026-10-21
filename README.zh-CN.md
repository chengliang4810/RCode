# RCode

一个本地优先的编码 agent：agent 本体就跑在这个进程里，模型接你配置的服务商。会话只留在本机 —— 没有账号，什么都不会上传。

*[English](README.md) | 简体中文 | [한국어](README.ko.md) | [日本語](README.ja.md) | [Español](README.es.md)*

![RCode 桌面应用](docs/media/readme/app-screenshot.jpg)

## 桌面应用

从 [GitHub Releases](https://github.com/chengliang4810/RCode/releases/latest) 下载对应平台的最新版本：

- **macOS** — `rcode-macos-arm64.dmg`
- **Windows** — `rcode-windows-x86_64-setup.exe`
- **Linux** — `rcode-linux-<arch>.tar.gz`，解压后运行里面的 `install.sh`

不用账号，也不用联网，会话就存在这台设备上。应用会自动更新。

## 无界面运行（CLI）

适用于服务器等没有显示器的机器，比如在你合上笔记本之后继续跑 agent 的 VPS。仅支持 Linux：

```bash
curl -fsSL https://github.com/chengliang4810/RCode/releases/latest/download/install.sh | sh
rcode status
```

安装脚本会把引擎作为后台服务拉起来，重启之后也会自己回来。

```bash
rcode status      # 查看引擎状态
rcode update      # 更新到最新版本
rcode daemon start|stop|restart|status
```

---

想参与开发，或者好奇它怎么跑起来的？[Ask DeepWiki](https://deepwiki.com/chengliang4810/RCode)，也可以看 [ARCHITECTURE.md](ARCHITECTURE.md)。

采用 [MIT License](LICENSE)。
