# 国际化与原生 agent 改造：验收记录（T14）

对应计划：`docs/design/ui-agent-i18n-implementation-plan.md`（T00–T14）。
执行日期：2026-10-14 / 15。分支 `rewrite/native-agent`，**全部提交在本地**。

2026-10-22 补充：后续界面新增了国际化接入遗漏，已进行专项补齐。当前展示边界和验证命令见 [国际化覆盖说明](i18n-coverage.md)。以下内容保留为当时的阶段验收记录。

---

## 一、交付了什么

### 1.1 十一个栏目

设置页从「配置外部 agent CLI」改成「配置模型服务商」，其余栏目按计划落地。

| 栏目 | 状态 | 位置 |
|---|---|---|
| Providers（模型设置） | 重写 | `crates/ui/src/settings/providers.rs` |
| memory | 完成 | `crates/agent/src/memory.rs` + `tools/memory.rs` |
| commands | 完成（只保留 `/compact`） | `crates/agent/src/command.rs` |
| skills | 发现 + 开关 | `crates/agent/src/skills.rs` |
| hooks | 事件枚举 + 匹配器 + Command 执行器 | `crates/harness/src/hooks.rs` |
| subagents / mcp / automations / plugin | **未做** | 见 §四 |

### 1.2 国际化

`crates/presentation` 提供 Fluent 资源、构建期生成 `TextKey`、中英双语键与变量一致性校验。**UI 层用户可见硬编码文案已归零**（测试断言与有意保留的 wire 文本除外）。

### 1.3 删除的死代码

| 内容 | 行数 |
|---|---|
| Accounts 页 + 用量环（7 个 RPC 无 handler） | 3,190 |
| `shell/harness_updates.rs` 孤岛 + 全部接线 | 1,049 |
| `settings/harnesses.rs` + `settings/completion.rs` | 1,565 |
| 新建项目的选设备步骤、「Agent updates」开关、`"voice"` 别名 | — |

---

## 二、已验证（附实际命令与数字）

### 2.1 测试

| 套件 | 命令 | 结果 |
|---|---|---|
| 全 workspace lib + bins | `cargo nextest run --workspace --lib --bins` | **3060 passed / 5 failed / 13 skipped** |
| UI（CI 的 `ui-tests` 目标） | `cargo nextest run -p rcode-ui --lib` | **1640 passed / 0 failed / 9 skipped** |
| 核心（CI 的 `test-core` 目标） | `scripts/ci/test-core.sh` | **828 passed / 8 failed / 9 skipped** |

**5 + 8 项失败全部是本机环境，不是代码缺陷**，逐条列在 §三。

CI 的两个目标在本轮工作**开始前就是红的**（基线 `2d965c83`：UI 套件 6 项失败，其中 `settings::harnesses::install_phase_*` 随文件删除消失，其余 5 项即上表）。现在 UI 目标全绿，核心目标只剩环境项。

### 2.2 编译

- `cargo check --workspace --all-targets`：**通过，无未使用导入警告**
- `cargo build -p rcode-presentation`：通过（这是 FTL 中英键与变量一致性的构建期闸门）

### 2.3 本轮修复的真实缺陷

| 编号 | 问题 | 影响 |
|---|---|---|
| F18 | Tombstone GC 后旧设备复活已删会话 | **数据正确性**：删除操作在离线设备重连后被撤销 |
| — | `ProviderDraft.web_search` 非 `Option` | 设置页保存**静默清掉**手配的搜索端点与 token |
| — | `bump_harness_catalog` 失去唯一调用者 | 保存服务商后模型选择器一直用旧缓存到重启 |
| — | `catalog::fallback_harnesses` 仍列已删的 CLI | 离线时提供**必然失败**的模型选项 |

F18 的修复依据：`gc_floor` 的语义在本侧可读——**wipe 把 horizon 归零，tombstone GC 把它推前**。据此区分「服务器从没见过的新行」（重新播种）与「已删除且墓碑被回收的旧行」（丢弃）。原测试把 bug 写成了断言。

---

## 三、未验证与已知缺口（重要）

### 3.1 本机环境的 8 项失败

| 测试 | 原因 |
|---|---|
| `terminals::windows_tests::*`（5 项） | 等 `PS ` 提示符，被本机 oh-my-posh profile 改写。实测抓到 powerline 字形与颜色码 |
| `rcode-preview::discovery::*`（3 项） | 测试要 spawn `python`，Windows PATH 上只有 Microsoft Store 别名 |

两者在 Linux CI 上都不存在。**但这只是推断，没有在 Linux 上实测过。**

### 3.2 跨平台检查只做了一半

`cargo check --target x86_64-unknown-linux-gnu` **无法完成**：`ring` 与 `tree-sitter-*` 的构建脚本需要 Linux C 交叉编译器。因此：

- **纯 Rust 且无 C 依赖的 crate 已交叉检查通过**：`rcode-doc`、`rcode-presentation`、`rcode-proto`、`rcode-text`、`rcode-theme`、`rcode-markdown`、`rcode-veil`
- **其余 crate 的 Linux 路径未经编译验证**

替代手段：Windows 上的「未使用导入」警告正是 cfg 门控名字的探测器。本轮用它抓到了一处真实风险——`crates/preview/src/discovery.rs` 的 `IpAddr`/`Ipv4Addr`/`Ipv6Addr` 只在 `#[cfg(target_os = "linux")]` 下使用，在 Windows 上删掉这个 import 会**打断 Linux 构建**。已改为按 `target_os = "linux"` 门控。

**没有验证的**：
- macOS / iOS 构建（本机无 Xcode）
- 三平台原生浏览器（webkit2gtk / WebKit / WebView2）的实际渲染
- Edge（TypeScript / Cloudflare Worker）——`node_modules` 未安装，未跑 Vitest/typecheck/workerd

### 3.3 界面实机证据

**未提供。** 所有验证都是命令行层面的。计划要求「Windows 提供实际界面和流程证据」——需要人工看着画面确认，本轮未做。建议的最小清单（2026-10-15 更新，含第二轮新增的字段）：

1. 设置 → 服务商：从空配置加一个 provider + model，保存后出现在模型选择器
2. **添加模型弹窗**：输入类型选「文本 + 图片」，模型行出现**「视觉」徽章**；改成「文本」徽章消失
3. **推理等级**：在弹窗里用「+」加几档，chip 按由弱到强排列；点 chip 可移除；保存后**对话里的推理档位 chip 出现这些档**（此前一直是空的）
4. 用该模型完成一次真实对话（含一次工具调用与一次权限确认）
5. 语言切到简体中文，**确认框点「立即重启」**，确认界面与原生菜单都是中文
6. 浏览器面板、终端面板各开一次
7. 新建项目：确认不再有选设备步骤


### 3.4 测试覆盖的边界

- 只跑了 **lib + bins**。engine 的 24 个集成 target 里，CI 只跑 4 个（`restart_resume`、`local_profiles`、`message_queue`、`attachments_roundtrip`），其余（`e2e`、`side_chats`、`projectless`、`workspace_sync`、`worktree_on_run` 等）**从未在 CI 跑过**
- Edge 侧完全没有测试

### 3.5 那些从未在 CI 跑过的集成 target，在本机是红的（2026-10-15 实测）

第二轮补跑了一次**全量**（不带 `--lib --bins` 的 `cargo nextest run --config-file scripts/ci/nextest.toml`，含全部集成 target）：

```
3392 tests run: 3369 passed, 23 failed, 27 skipped
```

23 项里 8 项是 §3.1 已记录的环境项（5 项 `terminals::windows_tests`、3 项 `rcode-preview::discovery`）。**剩下 15 项是本次新发现的**，分布在 `diff_sync_churn`、`m5_repos_diffs_terminals`、`device_routing`、`previews`、`runtime_survival`、`side_chats`、`workspace_files`、`worktree_on_run`——正是 §3.4 说的「从未在 CI 跑过」的那批。

**这些不是本轮改动引入的**：把源码回退到 `f37fff2a` 的父提交，用同一组 target 重跑，81 项里 **16 项失败**，覆盖了上述 15 项（多出的一项 `deleted_checkout_is_evicted_after_grace` 是时序 flaky，全量那轮是过的）。改动前后失败集合一致。

其中终端相关的一批有明确的直接原因，值得替换掉 §3.1 里那条推断：

```
open sh: Other("could not open Windows terminal: 系统找不到指定的路径。 (os error 3)")
```

即**本机打不开 Windows 终端**（ConPTY 宿主路径问题），不是 `PS ` 提示符被 profile 改写——那条解释只适用于 `terminals::windows_tests` 那 5 项。其余失败（worktree 物化、checkout 变更拒绝、discard、relay 路由）的根因尚未定位，只确认了**改动前后都失败**。


---

## 四、记录在案、明确不做的

| 项 | 说明 | 量级 |
|---|---|---|
| `crates/harness` 孤儿模块 | `executable`/`redact`/`native_bridge`/`catalog`/`jsonrpc`/`code_signature`/`process`/`model_context` 等，外部 CLI 删掉后只剩 `mod` 声明。编译时一片 `never used` | ~4,500 行 |
| 登录门 / 组织门 / 同步向导 / SignIn RPC | 仅 `RCODE_FORCE_GATE` 可达。引擎对所有构建都报 `Development` | ~1 天 |
| subagents / mcp / automations / plugin 四个栏目 | 计划 §三 的 5–8 项 | ~22 天 |
| 改名 RCode | 方案见 `.rcode/plans/zcode-settings-port/03-rename-to-rcode.md` | 8 批 |

### 4.1 一个必须交接的技术约束

**把 `WorkspaceScope::Development` 改成 `Local` 不是一行改动。**

`EngineProfile::local` 存 `data_dir/profiles/local`，`development` 存 `data_dir/orgs/<org>/<user>`。改 scope 会把**每个既有安装的数据目录搬走**，表现为「所有会话消失」。需要迁移，与改名方案的 B3 同类。

四个测试（3 个在 `crates/ui/src/state.rs`、1 个在 `crates/engine/tests/local_profiles.rs`）断言 `Local`/`Synced`，当前以 `#[ignore = "asserts a workspace scope this build does not reach"]` 标记——它们描述的是**目标终态**，不是当前行为。用 `Development` 让它们通过等于给这个令人困惑的状态盖章；删掉又丢了意图声明。

> **2026-10-16 已解决**：`WorkspaceScope` 连同 `EngineProfile::local`、`local-profile.json` 与那三个 `#[ignore]` 测试一起删除了，存储根改由 `DataRoots`（config 根 + 可移动的 data 根）表达。下面是当时的记录，保留原文；迁移细节见 §七。

---

## 五、结论

计划的 T00–T13 全部完成。**T14 的验收是不完整的**：命令行层面的验证做完了且结果良好（全 workspace 3060/3065），但**界面实机证据、Linux/macOS 实测、Edge 侧验证三项没有做**。

交付前必须补的是 §3.3 那五条人工确认，以及 §3.2 列出的三个平台构建。

---

## 六、第二轮（2026-10-15）：模型元数据与推理等级

用户看过界面后提的第二批问题，共四项：服务商页要参考稿的主从布局、移除设备残留、导航栏进 i18n、切语言弹重启确认框。前四项之外的第五项是**模型要有输入类型与推理等级**，它牵出了本轮唯一一处运行时改动。

### 6.1 推理等级此前是断的

用户要求「添加模型时选输入类型（决定『视觉』徽章）」和「每个模型不同的推理等级列表」。落地时发现：`NativeHarness::models()` 报 `reasoning_levels: Vec::new()`，而移植来的 `with_reasoning_effort` / `with_extended_thinking` **全仓没有调用者**。

所以只加字段会得到一个「显示得出、发不出去」的档位选择器——正是这个仓库一直在清理的那类东西。本轮把整条链接通：`ModelConfig` 声明梯子 → `models()` 报出来 → 合成器 chip 显示 → `RunRequest.reasoning` → `NativeHost` → 按协议映射（OpenAI 的 `reasoning_effort`、Responses 的 `reasoning.effort`、Anthropic 的 `thinking` + `output_config.effort`）。

三处需要说明的取舍（详见 `.rcode/plans/zcode-settings-port/02-port-plan.md` §十一）：

- **梯子顶端钳到 `high`**：9 档词汇表比三家协议都长，`xhigh` 及以上是同一个请求。代价是 `xhigh` 与 `high` 在线上不可区分。
- **Anthropic 预算钳在 `max_tokens` 之下**，留不出空间就不启用思考，而不是发非法请求。
- **声明为 text-only 的模型带图直接失败**（拒绝并说明），默认值 `text-and-images` 以保住旧文件里的视觉能力。

### 6.2 本轮的测试数字

| 套件 | 命令 | 结果 |
|---|---|---|
| `rcode-harness` | `cargo test -p rcode-harness --lib` | **178 passed / 0 failed** |
| UI（CI 的 `ui-tests` 目标） | `cargo nextest run --config-file scripts/ci/nextest.toml -p rcode-ui --lib` | **1641 passed / 0 failed / 9 skipped** |
| 全量（含全部集成 target） | `cargo nextest run --config-file scripts/ci/nextest.toml --locked --no-fail-fast` | **3369 passed / 23 failed / 27 skipped** —— 23 项全部为既存失败，见 §3.5 |
| `rcode-presentation` | `cargo build -p rcode-presentation` | 通过（中英 key 对齐闸门） |

新增 9 项测试：钳位映射、思考预算边界、每模型梯子、text-only 拒绝/放行/无图放行、旧文件默认值、声明往返、档位表完整性。

### 6.3 本轮明确的缺口

- **text-only 只在 harness 层拦**：合成器仍允许拖图进去，失败发生在发送时而非拖入时。要做到拖入即拒，需要把 `input_format` 透到 `rcode_proto::Model` → registry → pickers → composer dropzone。
- **档位名仍是英文**（`reasoning_label`，与合成器 chip 同源）。
- **界面不提示 `xhigh` 与 `high` 等价**。
- §3.3 的人工清单仍然**没有做**——第二轮同样只做到命令行层面。

---

## 七、第三轮（2026-10-16）：删掉 scope，把存储根拆干净

上一轮结束后用户提了两件事，顺序是先定存储、再谈收尾。

### 7.1 数据目录：一个名字，两个根

原本 Windows 走 `%LOCALAPPDATA%\RCode`、其余平台走 XDG/`~/Library`，同一个应用在不同平台落点不同，用户看不出「我的数据在哪」。现在所有平台都是 `~/.rcode`（Windows 从 `USERPROFILE` 解析，Git Bash 与资源管理器启动得到同一个目录）。

在此之上把原来的单一目录拆成两个：

- **config 根**（`~/.rcode`）：device-id、引擎锁、Provider 凭据、界面设置、日志，以及 `location` 指针本身。
- **data 根**：profile store（文档、journal、附件）、克隆仓库、scratch、cache、worktrees。默认与 config 根同目录；设置页可以把 data 根指到别处（`location` 指针），一个「改了要重启」的确认框负责收尾。

`RCODE_DATA_DIR` 仍然覆盖 config 根（开发版与安装版并存靠它，也就是「用 RCode 开发 RCode」需要的那份隔离）；指针跟着当前使用的根走。规格见 `docs/reference/windows-development.md`，架构叙述见 `ARCHITECTURE.md` §1「Storage: two roots, one profile」。

清理掉的孤儿（都是早先 scope 时代留下的，没有任何代码再读写）：`profiles/`、`agent-accounts/`、`model-catalogs/`、`local-profile.json`、旧的 `rcode-headed.log*`。

### 7.2 `WorkspaceScope` 删除

§4.1 记的那个「改 scope 会搬走每个安装的数据目录」的坑，在只剩一种 profile 形状之后不成立了，所以直接删：

- `crates/proto`：`WorkspaceScope` 枚举与 `EngineInfo.workspace_scope`。
- `crates/engine`：`EngineProfile` 的 `scope`、`EngineProfile::local`、`LOCAL_PROFILE_FILE`、`load_or_create_local_profile_id`；`resolve_profile` 变成全函数（身份 → store 根）；`WorkspaceHostConfig::local_only` 由 `edge.is_none()` 直接给。
- `crates/ui`：`AppState.workspace_scope` 与 onboarding 分支；侧栏 pin/section 只看 registry（`state.sidebar_preferences`）与乐观写入投影；`workspace_locator` 只对身份做哈希。
- `apps/rcode`：`live_engine_scope` 删除，`status` 打 `Config:` / `Data:` / `Mode: local only` / `Store:`。

侧栏 pin 落库的读写路径其实早就只认 registry 了，两个 scope 走的是同一条路，所以这次是行为不变的收口——九处分支点逐一核对过。

**测试上的坑**（也是第一次尝试失败、回退重做的原因）：夹具里的 `settings.sidebar_pins_mut`、`sidebar_sections_by_profile` 是**死路径**，线上从不读；写入需要真实的引擎句柄（否则被拒）、需要身份（否则算不出 profile key）、且写入在途时播种快照会被投影盖掉。为此新增了 `crates/ui/src/sidebar_test_support.rs` 作为共享夹具（静默引擎句柄 + `seed_preferences`），再由它移植 `sidebar_sections.rs` 的 14 个分区/拖拽夹具。

### 7.3 本轮的测试数字

| 套件 | 命令 | 结果 |
|---|---|---|
| 全量 | `cargo nextest run --config-file scripts/ci/nextest.toml --locked --no-fail-fast` | **3329 tests run: 3307 passed / 22 failed / 21 skipped** —— 22 项与改动前逐条一致（set-for-set 比对通过），全部既存失败 |
| feature-gated 示例 | `cargo check -p rcode-ui --examples --features browser-fixture,appshots-fixture,project-palette-fixture,resource-profile,pull-request-fixture,windows-render-fixture` | 通过（`--all-targets` 不编译这些目标，其中两个的编译错误是既存的） |
| 应用 | `cargo build -p rcode` + 启动 | 通过 |

已知 flaky：`rcode-engine::e2e interrupt_stamps_streaming_entry_aborted` 时序敏感，单独跑约三次失败一次（改动前后同样）。本轮它通过。

> 本机 `D:` 盘在做这轮验证时被 `target/` 撑满（101 GB 用尽），链接器报 `LNK1318`。改用 `CARGO_TARGET_DIR` 指向 `C:` 后跑通；这条与产品无关，但下次在本机跑全量前值得先看一眼磁盘。

### 7.4 本轮明确的缺口

- **iOS 与 macOS 只在代码层面改到**：本机没有 Xcode，打包/公证路径、iOS 绑定生成都没实机验证。
- **CI 没跑**：本机全量测试代替，平台矩阵（docs/design 里的三个平台构建）依旧没补。
- **`docs/PARITY.md` 只加了状态说明**，没逐条重审——它描述的功能清单跨越了被删除的云端，逐条核对是独立的一轮。

