# 插件市场

设置中的“插件市场”内置以下来源，均安装到个人插件目录：

- [Claude Code 官方市场](https://github.com/anthropics/claude-plugins-official)
- [Ponytail](https://github.com/DietrichGebert/ponytail)
- [Superpowers](https://github.com/obra/superpowers)

每页显示 20 项。搜索先筛选整个所选市场，再分页；搜索和切换来源会回到第一页。插件管理页负责已安装插件的启停与卸载，不再提供本地目录安装入口或 `InstallPlugin` RPC。

## GitHub 访问

`rcode-github` 是插件市场和软件更新共用的公开资源访问模块。每次请求依次尝试 `https://gh-proxy.com/`、`https://ghfast.top/`、原始 GitHub 地址，成功后立即停止，没有轮询分配或用户代理设置。

公开 GitHub 仓库、Raw、归档及公开仓库 API 地址可走代理；含凭据、查询参数或其他私有 API 路径的地址不进入代理列表。调用者使用无鉴权的公开资源客户端，其他业务的模型或 MCP 请求不经过这些节点。

HTTP 错误、超时、截断响应、无效 JSON/ZIP 和更新包校验失败都会触发下一节点。更新文件保留 `.partial` 和 SHA-256 校验，失败不替换已有目标。

市场目录按来源分别缓存，有效期为 1 小时。进入页面或切换来源时先展示有效的本地目录；缓存过期时仍立即展示，再后台更新。没有缓存或缓存损坏时才等待网络加载。手动刷新强制请求网络，保留当前搜索和分页；刷新失败保留缓存，并显示缓存状态。搜索和分页不请求网络，安装状态始终根据当前个人插件目录计算。

## 安装与运行

安装请求只接受内置来源 ID 和插件名称。引擎解析官方目录中的相对目录、GitHub 仓库、固定 SHA/分支与仓库子目录，下载 ZIP 后验证路径与解压大小，暂存并验证贡献内容，最后发布目录。失败不产生半安装状态，也不会覆盖已有插件。

支持标准技能、命令、子智能体、命令型 Hooks 和 MCP 贡献，包括清单中的显式文件路径、内联 Hook/MCP 配置与 `strict: false` 条目。插件技能与子智能体使用插件命名空间。仅有 LSP 的条目暂不支持。

运行时补齐了 SessionStart 来源匹配、嵌套 `hookSpecificOutput.additionalContext`、上下文实际注入、压缩后重新引导、插件根目录环境、后台 Hooks、命令参数、StopFailure、Git Bash 发现及 MCP 工具命名兼容。正常退出的后台服务启动器保留其后台进程；超时和取消仍终止相应进程树。

## 验证

`scripts/audit-plugin-marketplaces.py` 只读取清单和 Hooks 声明，不执行扫描到的插件代码。输出位于 `output/marketplace-compatibility/`。结构覆盖率与真实运行验收分别记录，不能把扫描比例当作所有第三方服务已经通过业务测试。

关键回归：

```powershell
cargo test --locked -p rcode-github -p rcode-plugins -p rcode-update -p rcode-presentation -p rcode-mcp --lib
cargo test --locked -p rcode-agent --lib
cargo test --locked -p rcode-engine --test plugin_projection --test personal_catalogs
cargo test --locked -p rcode-ui --lib settings::marketplace::
cargo test --locked -p rcode-harness --lib hooks::tests::
```

真实网络验收（使用临时目录，不安装到用户目录）：

```powershell
cargo test --locked -p rcode-plugins --lib builtin_markets -- --ignored --nocapture
cargo test --locked -p rcode-harness --lib superpowers_live -- --ignored --nocapture
```
