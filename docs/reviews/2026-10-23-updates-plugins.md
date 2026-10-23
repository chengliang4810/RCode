# 更新、GitHub 下载及插件复验

复验日期：2026-10-23。修改前基线：`ebb2e6d5455df5fc29800bd226b2c7565d52edf3`。
输入报告：`RCode-analysis.md`（声明下载于 2026-10-22，没有准确提交 SHA，原报告未构建、未复现）。本文件只记录分配到更新、GitHub、插件及发布流程的项目。

| 报告项 | 当前核实与处理 | 验证及边界 |
| --- | --- | --- |
| H1 Unix 更新缺少摘要仍继续 | 仍成立，已修复。`download_release_file` 及 Unix/macOS staging 必须具有 64 位十六进制 SHA-256，校验缺失时在请求、解压、执行前失败。`latest.txt` 仍能用于发现版本，不能授权安装。 | `every_update_requires_a_valid_checksum_before_download` 覆盖缺失、空值、短值、非十六进制；原有 `corrupted_downloads_retry_without_replacing_an_existing_file` 验证错误制品不覆盖文件。Windows 已有强制摘要，本次保持。 |
| H2 镜像可同时替换清单、制品和摘要 | 仍成立，镜像替换入口已关闭。发布清单及版本元数据只从配置的发布源通过 HTTPS 获取；制品仍可走镜像，但必须匹配该源提供的摘要。 | `origin_fetch_does_not_fall_back_to_mirrors_on_failure` 用本地 HTTP 代理记录 CONNECT，验证源失败不会访问镜像。**仍未实现发布签名**：没有受控签名密钥及发布端签名配套，不能把 HTTPS 源信任描述为离线签名验证。默认源为官方 GitHub；显式配置的其他源属于使用者选择的信任源。 |
| M7 插件镜像 ZIP 仅校验格式 | 仍成立，已关闭镜像替换路径。目录、提交解析及 ZIP 均从官方 GitHub 获取。分支、标签、HEAD、短 SHA 经官方 API 解析为完整提交 SHA 后下载；完整 SHA 可直接使用。旧镜像目录缓存切换命名，避免继续复用。 | `branch_and_short_revisions_resolve_to_immutable_commits`、`immutable_revisions_are_preserved_and_commit_responses_are_validated`，以及原有 ZIP 解压/发布测试。Git 提交 SHA 不是 ZIP 文件的 SHA-256，不能直接拿两者比较。当前信任 GitHub HTTPS 提供该提交的归档，未增加独立的制品签名。首次解析浮动引用会增加一次 API 请求，也受到 GitHub API 可用性和频率限制。 |
| M8 安装即默认启用插件 | 行为仍存在，保留。`plugins/src/lib.rs` 由 disabled 列表决定启用状态，已有安装即启用/显式停用测试。 | 这是既定完全信任、直接执行策略的一部分；本次不恢复操作审批。用户安装插件仍代表接受其中命令、hooks、MCP 的执行能力。 |
| M11 Windows 未应用的 staging 目录持续保留 | 仍成立，已增加回收。完成 staging 后记录所属进程；启动和下一次 staging 时清理所属进程已退出的目录，保留存活实例使用的目录。旧版无 owner 目录保留 7 天后回收。 | `abandoned_stages_are_reclaimed_without_removing_live_or_unrelated_directories` 覆盖死亡/存活进程、旧版目录宽限期和无关目录；同时检查当前进程确实被识别为存活。只处理专属前缀的真实目录，跳过目录符号链接。当前活跃进程的 staging 仍保留，避免删除待应用更新。 |
| Low 自动应用等待闲置阻塞检查循环 | 仍成立，已修复。保留一个可继续轮询的自动更新 future，等待闲置期间调度循环仍处理手动检查、唤醒、定期检查和已安装新版本的重启探测。 | `deferred_auto_apply_does_not_block_checks_or_get_cancelled_by_wakes` 验证手动唤醒和定期轮询不被长期任务阻塞，且唤醒不会丢弃该任务。HTTP 检查本身仍受既有超时约束。 |
| Low 缓存 staging 仅检查文件存在 | Unix headless 仍成立；macOS 原先会运行缓存二进制的 `--version`，这也不能证明缓存完整。已修复为校验当前制品摘要标记及可执行文件摘要，未验证的旧缓存不再被直接执行。 | `cached_stages_require_the_current_artifact_and_unchanged_executable` 覆盖旧缓存无标记、制品摘要变化、文件被修改。Windows 原本在 apply 前复验摘要，本次保持。缓存标记由本机在验证制品并完成版本检查后生成，不构成抵抗本机同权限攻击者的签名；本次没有为 macOS bundle 内每项资源建立独立清单。 |
| Low 每次下载可写到 4 GiB | 当前确实存在 4 GiB 上限；保留，未宣称优化。下载已流式限长且失败时清理 partial。 | 没有可信的跨平台最大包大小/发布大小字段作为更小阈值的依据，任意下调可能阻断正式更新。后续可将可信制品大小纳入发布元数据，并在下载前评估可用空间。当前不能保证磁盘不足时仍可完成更新。 |
| Low release 工作流全局 contents:write | 仍成立，已修复。全局改为 `contents: read`，只有 publish job 授予 `contents: write`。 | 本地 `python -m unittest discover -s scripts -p test_release_packaging.py`：7 项通过；没有在 GitHub 触发真实发布。 |
| Low 仓库内 dist、pycache、base_fmt 杂物 | `dist/` 是正式安装器、桌面入口、图标、许可/打包资源，不应按构建产物删除。根部 `__pycache__/*.pyc` 与 `base_fmt.txt` 的确已跟踪；后者是格式检查位置列表。 | 本次仅记录，不混入与运行缺陷无关的文件删除；`.gitignore` 已忽略后续 pycache。 |

## 验证结果记录

- 已执行发布打包 Python 测试：7 项全部通过。
- Rust 回归已由主任务执行：`cargo test -p rcode-github -p rcode-update -p rcode-plugins --lib`；GitHub 5 项、Update 32 项、Plugins 22 项通过，Plugins 另有 1 项原有 ignored 测试未运行。
- 本地为 Windows 环境；Linux/macOS 真实安装、bundle 交换、服务重启及签名链路未执行。Unix 摘要及缓存校验的公共函数在 Windows 单元测试中覆盖；既有 Unix tarball 集成测试仍由相应平台执行。
- 正向改善以确定性回归行为为证据：无可信摘要时不下载/执行，源失败不自动信任镜像，修改的缓存被拒绝，遗留目录被回收，延期更新不阻塞调度。没有以静态代码变化推算用户端速度或帧率收益。
