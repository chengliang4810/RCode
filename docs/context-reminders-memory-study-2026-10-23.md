# Claude Code / Peri 隐藏上下文与记忆机制调查

日期：2026-10-23（Asia/Shanghai）。本报告记录本地源码事实，未运行两个客户端、未调用付费模型，也未读取用户的真实记忆文件或凭据。

后续已完成 [第一轮真实模型实验](context-reminder-trial-2026-10-23.md)，并按用户确认接入 [事实索引增量与 MCP 状态通知](context-reminders.md)。本文件保留源码调查结论；MCP 指令正文没有迁移到 user。

| 对象 | 本地位置 | 核对版本 |
|---|---|---|
| Claude Code Best | `D:\projects\claude-code` | `77a7934e15`，package.json 为 2.8.4；社区衍生实现，不等同于官方当前客户端 |
| Peri | `D:\projects\peri` | `cd18db7aff` |
| RCode | `D:\projects\zeron` | 当前工作区；缓存排序与 usage 兼容改动仍未提交 |

## 结论与前述判断的校正

1. 两个参考项目都有 `<system-reminder>` 相关机制。标签是模型输入文本，不是 API 新角色，也不需要 DSH 的 `in-history` 能力声明。模型能否同等遵循其中的说明，仍要在目标服务上验证。
2. Claude Code Best 有完整的文件自动记忆路径、通用的已跟踪文件变化提醒，以及按相关性追加记忆正文的可选路径。交互模式会把已存在的 MEMORY.md 加入文件变化跟踪，外部修改可作为变化片段追加到历史；未发现“索引每变化一次就统一追加完整新索引”的专用策略。首轮调查遗漏了通用文件变化提醒，下面已补正。
3. Peri 确实有 `/memory` 面板，扫描并编辑 `~/.claude/memory` 的 Markdown 文件。此前仅检查核心模型链路，遗漏了这个界面模块。当前追踪到的扫描结果只进入 TUI 文件列表，未找到该目录到自动记忆召回、模型注入和自动写回的完整内置链路。
4. Peri 的 `memory_probe`、Markdown memory 计量和 Langfuse memory 模块主要是内存占用管理，不能据名称认作模型长期记忆。
5. 用户已明确排除 `in-history` 专用能力判断。本报告不重新引入该方案。后续试验和事实索引/MCP 状态通知接入已另行完成，MCP 指令正文仍保留 system。

## 一、Claude Code Best 的内部提醒

固定 system 明确解释：user 消息和工具结果可能带有系统提供的 `<system-reminder>` 信息。程序内部先建立 attachment 或 `isMeta: true` 的 user 消息，发送前包装文本并按需要合并相邻消息。API 请求仍为 `role: user`；`isMeta` 是客户端元数据，不发送为模型角色。普通 meta user 被显示过滤器排除；附件也可按类型不渲染。部分记忆附件会展示简短加载提示，而不显示完整原文。

主要证据：

- [固定提示词约定](D:/projects/claude-code/src/constants/prompts.ts:192)
- [提醒包装](D:/projects/claude-code/src/utils/messages.ts:3488)
- [API user 序列化](D:/projects/claude-code/src/services/api/claude.ts:615)
- [meta 消息显示过滤](D:/projects/claude-code/src/utils/messages.ts:5098)
- [不渲染的附件类型](D:/projects/claude-code/src/components/messages/nullRenderingAttachments.ts:14)

### MCP 指令变化

`mcpInstructionsDelta.ts` 将已宣布的服务器名称与当前连接集合比较；新增时附加服务器说明，断开时告知之前说明不再适用。启用该机制后，原动态 system 中的 MCP 说明返回空值，改由 `mcp_instructions_delta` 附件承载，最终编码为隐藏的 user 提醒。

这条路径受 `CLAUDE_CODE_MCP_INSTR_DELTA`、用户类型与功能配置控制，不是按模型是否支持历史 system 判断。未检查本机实际运行配置，不能声称已启用。它主要按服务器名称比较，假定同一连接内 instructions 不变，并非通用的同名说明内容变化检测。

还有两处不能直接照搬：完整压缩后已有补发 MCP 说明的代码，但普通用户的会话落盘路径会过滤大多数 attachment。源码中的“persisted delta”设计意图不等于所有运行模式已具有相同恢复保证。

- [开关与差异计算](D:/projects/claude-code/src/utils/mcpInstructionsDelta.ts:37)
- [动态 system 移除 MCP 说明](D:/projects/claude-code/src/constants/prompts.ts:487)
- [MCP 提醒内容](D:/projects/claude-code/src/utils/messages.ts:4656)
- [压缩后补发](D:/projects/claude-code/src/services/compact/compact.ts:605)
- [外部用户附件落盘过滤](D:/projects/claude-code/src/utils/sessionStorage.ts:4488)

## 二、Claude Code Best 的记忆层次

### A. 文件自动记忆：规则、索引和正文分开

默认目录由 `getAutoMemPath()` 解析，通常为 `~/.claude/projects/<project-key>/memory/`，允许配置覆盖。`MEMORY.md` 是入口索引；主题文件保存 user、feedback、project、reference 等内容。主模型使用普通文件工具维护主题文件和索引。

主会话的实际输入链路是：

```text
system：记忆使用/保存规则 + 目录
user 前缀：项目指令及 MEMORY.md 索引（经典路径）
对话历史：问题、回答和按需读取文件的工具结果
```

`loadMemoryPrompt()` 生成规则和目录，不直接拼接主会话的索引正文。索引由 `getMemoryFiles()` 读取，通过 `getUserContext()` 汇总，最终由 `prependUserContext()` 放进带 `isMeta` 的 `<project-instructions>` user 前缀。部分旧注释称索引进入 system，以上结论以当前调用链和最终消息角色为准。

`getMemoryFiles()` 和 `getUserContext()` 使用 memoize；记忆规则也通过 `systemPromptSection('memory', ...)` 缓存。clear/compact 等显式生命周期会清理这些缓存。普通每轮不会重新读取并改写这个前缀。因此，同一会话内的文件更新并不等于前缀立即更新；工具读取、文件写入结果和下面的外部修改提醒仍可让主模型知道变化。

- [自动记忆启用规则](D:/projects/claude-code/src/memdir/paths.ts:22)
- [目录解析](D:/projects/claude-code/src/memdir/paths.ts:223)
- [保存规则与索引约定](D:/projects/claude-code/src/memdir/memdir.ts:199)
- [system 记忆规则生成](D:/projects/claude-code/src/memdir/memdir.ts:419)
- [读取 AutoMem 索引](D:/projects/claude-code/src/utils/claudemd.ts:978)
- [缓存的 user 上下文](D:/projects/claude-code/src/context.ts:155)
- [前缀 user 消息生成](D:/projects/claude-code/src/utils/api.ts:443)
- [压缩时清理缓存](D:/projects/claude-code/src/services/compact/postCompactCleanup.ts:63)

### A.1 补充核对：索引外部修改可以通过通用文件变化提醒追加

交互模式的 `REPL.onInit()` 将 `getMemoryFiles()` 返回的文件填入 `readFileState`。返回列表包括已存在且自动记忆启用时的 MEMORY.md（AutoMem），没有在此处排除它。因此这条路径不只跟踪 CLAUDE.md，也可以跟踪自动记忆索引。

后续 `getChangedFiles()` 遍历仍在文件状态缓存里的路径，检查 mtime；发现变化后调用 Read，再通过 `getSnippetForTwoFileDiff()` 提取变化附近的**新内容片段**，生成 `edited_text_file` 附件。附件转为带 `isMeta` 的 user 消息，并包装为 `<system-reminder>`。工具循环会将附件追加到本轮结果序列。

```text
会话前缀：原索引快照（继续复用）
已有对话：保持原内容
后续历史：MEMORY.md 外部修改提醒 + 变化片段
```

需要区分：

- 这不是索引专用的完整快照追加。片段算法保留变更附近 8 行上下文，过滤删除行与 diff 元数据，限制输出长度；不能把它当作无损补丁或完整新索引。小索引可能因为全部落在上下文范围内，看起来像全文追加。
- 模型通过自身 Edit/Write 修改文件时，工具同时更新 `readFileState` 的内容和时间戳；在没有后续外部修改的情况下，不会再由此检测器重复通知同一次写入。模型已经通过工具调用及结果知道自己的修改。
- 检测要求文件已被跟踪、尚未从缓存淘汰且满足读取条件；带 offset/limit 的读取状态被跳过。不存在于启动扫描中的新文件，不会仅因在目录中出现就自动被这个遍历发现。附件关闭/SIMPLE 模式也会影响这条路径。
- 这里确认的是交互初始化加通用检测的源码链路，未将其推广为所有 SDK/headless 入口都具有相同初始化与持久化行为。

证据：[AutoMem 加入发现结果](D:/projects/claude-code/src/utils/claudemd.ts:978)、[交互初始化填入跟踪缓存](D:/projects/claude-code/src/screens/REPL.tsx:4725)、[文件变化检测](D:/projects/claude-code/src/utils/attachments.ts:2115)、[变化片段算法](D:/projects/claude-code/packages/builtin-tools/src/tools/FileEditTool/utils.ts:223)、[转换为内部提醒](D:/projects/claude-code/src/utils/messages.ts:3970)、[追加到本轮工具结果](D:/projects/claude-code/src/query.ts:1894)、[Edit 更新跟踪状态](D:/projects/claude-code/packages/builtin-tools/src/tools/FileEditTool/FileEditTool.ts:509)、[Write 更新跟踪状态](D:/projects/claude-code/packages/builtin-tools/src/tools/FileWriteTool/FileWriteTool.ts:333)。

### B. 按相关性追加记忆正文：可选路径

`tengu_moth_copse` 开启时，`filterInjectedMemoryFiles()` 从固定前缀过滤 AutoMem/TeamMem 索引，记忆规则也切换为不要求维护索引的描述。随后在用户轮次异步启动召回：扫描记忆元数据，通过额外的模型查询挑选相关文件，读取正文，再生成 `relevant_memories` 附件。

```text
本轮用户问题 + 记忆文件元数据
              ↓ 额外的相关性选择请求
选中的主题文件正文
              ↓
relevant_memories 附件
              ↓
user 中的 <system-reminder>
```

源码包含以下约束：

- 每轮最多选择 5 个文件；单文件读取受 200 行和 4096 字节上限约束，超出时附截断说明。
- 按已读取文件与当前历史中已有的记忆附件去重。
- 按当前保留历史累计内容长度，达到约 60 KiB 的配置阈值后停止预取；实际累计使用 JavaScript 字符串长度，不应把该计数当作精确 UTF-8 字节账单。
- 压缩移除旧附件后，可重新召回。
- 附件创建时保存已渲染的 header；回放时优先复用，避免“保存于几天前”等相对时间重新渲染造成历史字节变化。
- 预取有取消机制，不阻塞主轮次；未完成的结果等后续工具循环消费。选择失败返回空候选。

这不是无成本缓存优化：它增加相关性选择请求，且召回质量影响模型看到的内容。该功能开关默认回退值为 false，另受自动记忆开关、poor mode 等条件约束；不能声称该本地实例已经启用。

- [过滤固定索引](D:/projects/claude-code/src/utils/claudemd.ts:1135)
- [召回启动条件](D:/projects/claude-code/src/utils/attachments.ts:2419)
- [额外的相关性模型查询](D:/projects/claude-code/src/memdir/findRelevantMemories.ts:80)
- [选择、去重和读取](D:/projects/claude-code/src/utils/attachments.ts:2248)
- [读取预算与累计限制](D:/projects/claude-code/src/utils/attachments.ts:272)
- [复用已渲染正文头并转为提醒](D:/projects/claude-code/src/utils/messages.ts:4140)

### C. 其他名称相近但职责不同的记忆

| 模块 | 实际职责 | 与缓存优化的关系 |
|---|---|---|
| extractMemories | 在开关允许时，从已完成轮次提取长期信息并写入自动记忆目录；主模型已写入时跳过相应范围 | 使用父会话的 cache-safe fork 参数，仍会增加模型调用 |
| SessionMemory | 在当前 session 的 `session-memory/summary.md` 维护会话摘要；后台更新受功能开关约束 | 服务于长会话与压缩，不等同于项目长期事实文件 |
| LocalMemoryRecall / local-memory | 本地衍生实现额外提供的跨会话笔记仓库，位于 `~/.claude/local-memory/<store>/<key>.md` | 模型按需调用只读工具，内容作为工具结果进入历史；不是自动索引注入 |
| Agent memory / team memory / daily log | 特定 Agent、共享或日志模式的扩展 | 各有独立条件，不代表基础路径全部默认启用，也不是本次 RCode 的实施范围 |

证据：[提取触发](D:/projects/claude-code/src/query/stopHooks.ts:143)、[避免重复写入](D:/projects/claude-code/src/services/extractMemories/extractMemories.ts:342)、[cache-safe 提取调用](D:/projects/claude-code/src/services/extractMemories/extractMemories.ts:412)、[SessionMemory](D:/projects/claude-code/src/services/SessionMemory/sessionMemory.ts:1)、[会话摘要路径](D:/projects/claude-code/src/utils/permissions/filesystem.ts:257)、[本地笔记仓库](D:/projects/claude-code/src/services/SessionMemory/multiStore.ts:1)、[LocalMemoryRecall](D:/projects/claude-code/packages/builtin-tools/src/tools/LocalMemoryRecallTool/prompt.ts)。

## 三、Peri 的提醒、记忆管理和会话连续性

### 结构化提醒

Peri 的 `TranscriptEntry` 将普通消息和 Reminder 分开。Reminder 带 source、kind、body、summary、audiences、delivery 等字段。模型投影将其编码为 `<system-reminder ...>`，映射到 Human，再由模型桥接层转换为 API user。模型可见性与 TUI 可见性独立；不是所有提醒都隐藏，面向 TUI 的提醒通常默认折叠。

MCP 已确认的生产者是首轮连接概览、连接状态变化和工具搜索提示。在检查的核心路径中未找到与 Claude Code Best 相同的服务器 `instructions` 增删广播；不能把“状态提醒存在”扩展成“完整 MCP 指令迁移已实现”。

- [提醒结构](D:/projects/peri/peri-acp-types/src/system_reminder.rs:79)
- [提醒编码](D:/projects/peri/peri-acp-types/src/system_reminder.rs:548)
- [提醒与普通消息的独立存储及投影](D:/projects/peri/peri-agent/src/session/transcript/entry.rs)
- [追加与持久化](D:/projects/peri/peri-agent/src/session/transcript.rs:324)
- [TUI 展示](D:/projects/peri/peri-tui/src/kit/tui_render_unit/reminder.rs)
- [MCP 状态提醒](D:/projects/peri/peri-middlewares/src/mcp/middleware.rs:537)

### /memory 面板真实存在，但已确认链路止于界面管理

`PanelKind::Memory` 注册了 `/memory` 命令。`MemoryScan` 每 30 秒扫描 `~/.claude/memory` 下最多 200 个直接子级 Markdown 文件，收集相对路径、大小和修改时间。扫描器不读取正文。结果经 `MEMORY_LIST` 投影给面板，Enter 调用外部编辑器打开所选文件。

```text
~/.claude/memory/*.md
        ↓ 元数据扫描
MEMORY_LIST
        ↓
/memory 面板 → 外部编辑器
```

该目录不同于 Claude 自动记忆的 `~/.claude/projects/<project-key>/memory/`，也不同于其 `local-memory` 笔记仓库。不能因路径含 `.claude` 就认为它们是同一模块。

本轮检查了 tracked 文件名、核心 Rust/TS/提示词、记忆面板、扫描器、`MEMORY_LIST` 的所有生产引用、Workspace 指令读取和 Agent 提醒链路，未找到上述全局 memory 目录到自动索引注入、相关性召回或自动写回的接线。此结论限于当前本地内置实现，不排除用户通过外部 MCP/插件或手动文件操作补充记忆能力。`peri-cool` 文档子模块与 `e2e/tui-tester` 当前未初始化；本轮未拉取或改变参考仓库。

面板头部旧注释写 2s，实际扫描调度代码是 30s；以执行代码为准。

- [/memory 注册](D:/projects/peri/peri-tui/src/kit/panel_registry.rs:298)
- [实际扫描与间隔](D:/projects/peri/peri-tui/src/kit/service_snapshot/memory_scan.rs:24)
- [仅收集文件元数据](D:/projects/peri/peri-tui/src/kit/service_snapshot/memory_scan.rs:41)
- [发布给界面](D:/projects/peri/peri-tui/src/kit/service_snapshot/commit.rs:288)
- [打开外部编辑器](D:/projects/peri/peri-tui/src/kit/panels/memory.rs:237)

### 已接入模型的持久上下文

Peri 会从 Workspace 资源面读取 AGENTS.md / CLAUDE.md 等项目指令并冻结；`AgentsMdMiddleware` 只持有冻结正文，通过 `prompt_contribution()` 注入，不在每轮重新读盘。会话历史和摘要提供连续对话能力，`incoming_recalls` 通过上下文消息补充运行信息。这些机制不能与“自动长期记忆学习”混称。

- [Workspace 指令候选](D:/projects/peri/mcp-packages/workspace/src/resources/instructions.rs:31)
- [冻结项目指令贡献](D:/projects/peri/peri-middlewares/src/agents_md/mod.rs:40)
- [FrozenContext](D:/projects/peri/peri-agent/src/session/store.rs:51)
- [版本化冻结快照](D:/projects/peri/peri-acp/src/session/frozen_snapshot.rs:29)
- [输入回顾提醒](D:/projects/peri/peri-agent/src/session/exec/executor/input_recalls.rs:12)

### 其他缓存措施及限制

Peri 使用确定性工具目录、冻结前缀和静态/动态 system 边界；非 direct 工具通过 SearchExtraTools / ExecuteExtraTool 按需发现与调用。不过延迟工具目录仍进入动态 system contribution，目录变化仍可能改变历史之前的前缀。Anthropic 显式缓存断点与 OpenAI-compatible 的内部标记剥离也不能混为同一服务端缓存能力。

证据：[工具目录贡献](D:/projects/peri/peri-middlewares/src/tool_search/middleware.rs:119)、[动态 system 组装](D:/projects/peri/peri-agent/src/agent/model_bridge.rs:264)、[静态/动态缓存分块](D:/projects/peri/peri-model/src/anthropic/cache.rs:15)。

## 四、对 RCode 决策的意义

| 候选方向 | 来源与已确认事实 | 待验证边界 |
|---|---|---|
| 规则稳定、内容分开 | Claude 主会话已将记忆规则与索引/正文分开 | 不能仅改 role 就宣称语义等价 |
| 稳定前缀、追加外部文件变化片段 | Claude 的通用 getChangedFiles 路径，交互模式可覆盖已跟踪索引 | 片段并非无损补丁；删除、截断、跟踪淘汰、恢复与压缩需验证 |
| 索引变化后追加完整快照 | 当前讨论的 RCode 候选 | 不是本轮在 Claude 找到的经典或相关召回路径，仍需独立实现和测试 |
| 按相关性追加正文 | Claude 的可选 relevant_memories 路径 | 增加选择请求、预算和召回质量问题；不是无功能变化的缓存修复 |
| 结构化内部提醒 | Peri 的独立 Reminder、受众和存储投影 | 可按 RCode 需要保留来源、正文、显示策略和持久化，不必复制整套 ACP 类型 |
| MCP 指令增量提醒 | Claude 的可选 mcp_instructions_delta | 同名内容更新、失效、恢复和压缩要完整验证；工具 schema 变化另行处理 |

已有的 RCode 工具排序与 usage 兼容保留；不新增 `in-history` 能力字段或路由分支。本报告不批准任何记忆/MCP 功能变更，也不改变现有文件工具、权限模型、记忆 UI 或团队记忆范围。

### RCode 初期参考关系与当前差别

RCode 的项目目录、MEMORY.md 索引、独立主题文件、user/feedback/project/reference 分类、普通 Read/Write/Edit 工具维护，确实沿用了 Claude 风格的文件记忆设计。但存储结构与请求更新策略是两层。当前 `memory::system_prompt_section()` 每次读取索引并拼接规则、目录和索引；`PromptSessions` 将它作为 dynamic system section，每个 run 刷新。测试明确要求新 run 的 system 出现新记忆并移除旧记忆。

当前 Git 可见初始提交 `41f1c4c` 已包含上述读取、拼接、每 run 刷新注释及测试；它不是本轮缓存优化新引入的策略。这只证明 RCode 当时采用的行为，不反推当时参考的 Claude 版本与本次本地快照完全一致。

证据：[文件结构与保存约定](D:/projects/zeron/crates/agent/src/memory_prompt.md:27)、[规则和索引拼接](D:/projects/zeron/crates/agent/src/memory.rs:94)、[每 run 动态刷新](D:/projects/zeron/crates/harness/src/prompts.rs:168)、[替换旧 system 内容的测试](D:/projects/zeron/crates/harness/src/prompts/tests.rs:123)。

## 五、证据范围

本轮完成的是本地源码、调用方和数据流核对，以及文档整理。没有运行参考项目的构建、交互界面或真实模型调用，没有验证其部署开关或个人配置。不能将源码存在、面板存在、注释中的目标，等同于当前运行实例已启用或真实行为已经验收。此前 72 次缓存实验仍属于原报告中的限定场景，不作为本报告中新机制的实测结果。
