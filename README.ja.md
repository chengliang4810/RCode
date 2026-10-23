# RCode

ローカル優先のコーディングエージェントです。エージェント本体はこのプロセス内で動作し、設定したモデルプロバイダーに接続します。セッションはこのデバイスに保存され、モデルへのリクエストは設定したプロバイダーに送信されます。RCode アカウントは不要です。

*[English](README.md) | [简体中文](README.zh-CN.md) | [한국어](README.ko.md) | 日本語 | [Español](README.es.md)*

![RCode デスクトップアプリ](docs/media/readme/app-screenshot.jpg)

## デスクトップアプリ

[GitHub Releases](https://github.com/chengliang4810/RCode/releases/latest) から、お使いのプラットフォーム向けの最新版をダウンロードしてください。

- **macOS** — `rcode-<version>-macos-arm64.dmg`
- **Windows** — `rcode-<version>-windows-x86_64-setup.exe`
- **Linux** — `rcode-<version>-linux-<arch>.tar.gz` を展開し、中の `install.sh` を実行

RCode アカウントは不要です。モデルへのアクセスと更新にはネットワークを使用し、セッションはデバイスに保存されます。アプリは自動でアップデートされます。

## ヘッドレス実行（CLI）

サーバーなど、ディスプレイのないマシン向けです。たとえば、ノートパソコンを閉じたあともエージェントを動かし続ける VPS に使えます。Linux のみ対応しています。

```bash
curl -fsSL https://github.com/chengliang4810/RCode/releases/latest/download/install.sh | sh
rcode status
```

インストーラーはエンジンをバックグラウンドサービスとして起動し、再起動後も自動的に立ち上がります。

```bash
rcode status      # エンジンの状態を確認
rcode update      # 最新版にアップデート
rcode daemon start|stop|restart|status
```

---

開発に参加したい方や仕組みが気になる方は、[Ask DeepWiki](https://deepwiki.com/chengliang4810/RCode) または [ARCHITECTURE.md](ARCHITECTURE.md) をご覧ください。

[MIT License](LICENSE) のもとで公開されています。
