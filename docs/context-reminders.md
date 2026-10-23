# 内部上下文增量

主对话使用结构化内部记录追加事实索引和 MCP 工具状态通知。记录只进入模型历史，不产生聊天中的普通用户消息。记忆使用规则、用户/反馈型索引条目和 MCP 指令正文仍位于 system；不使用 `in-history` 能力字段或历史 system 消息。

## 记忆分流

`MEMORY.md` 仍是普通 Markdown 索引，目录、主题文件、编辑界面和 Read/Write/Edit 工具保持原有格式。

- 仅识别独立列表项中的简单 Markdown 文件链接，并检查目标文件 frontmatter 的 `type`。
- `project` / `reference` 条目的索引行进入事实记录。这里传递的是指针和描述，不是主题正文。
- `user` / `feedback`、无 frontmatter、读取失败、复杂链接、重复指向同一文件等条目保留在 system。没有足够证据时不自动降低角色。
- 为分类，最多读取每个目标文件前 4096 字节中的已闭合 YAML frontmatter。正文不进入提示词，文件路径沿用已有信任所有行为；普通文件外的对象不会作为自动读取目标。
- 索引原有的 200 行、25,000 UTF-8 字节限制继续生效，超限说明仍保留。

system 中的记忆规则、目录及保留条目每个 run 更新。事实索引在每次模型调用前重新检查，因此外部修改及普通文件工具修改都能在下一个模型步骤被看见。行为分类发生变化时，system 中保留条目的刷新仍遵循每个 run 的边界。

## 记录与恢复

记录携带版本、域、范围、修订号、基线修订、状态，以及 `replace` / `patch`、`upsert` / `remove`。

1. 首次有事实数据时追加完整基线。
2. 后续比较当前状态与**实际保留历史**重放得到的状态；不变则不追加。
3. 改动只追加新值；删除明确列在 remove 中。
4. 禁用或读取失败时追加失效状态，防止把旧索引继续当作当前索引；恢复时重发完整基线。
5. 历史中没有基线，或压缩只保留了补丁时，重新追加完整当前状态。replace 开启新的基线，按对话顺序覆盖此前状态，不用全局最大修订号判断新旧。

记录的结构化 metadata 与已渲染文本一起保存到现有 `model-history`。写盘成功后才追加到待发送历史；保存失败会中止该次调用，不提前推进另一个状态游标。取消、重启和 HTTP 重试不会仅因“曾生成过更新”而漏发。旧记录没有 context 字段时按普通消息加载。

用户输入中的同名标签不会被解析成结构化 metadata。主题描述中的 XML 字符会转义。模型 API 仍收到普通 user 内容，但历史重放、去重和压缩不靠识别用户可伪造的文本标签。

内部记录不计作新的用户任务或压缩轮次；压缩保留真实用户任务，丢失上下文基线时重新建立。记录不会通过 `AgentEvent::UserMessage` 发布给界面。

## MCP 范围

每个 run 根据真实注册的 MCP 工具名、描述、schema 以及当前合并的 MCP 指令计算配置指纹。工具名使用完整 wire name，不通过拆分 `__` 猜测服务器身份。

工具加入、移除、同名 schema / 描述 / 指令内容变化会产生可用性或配置变化通知。指纹仅用于内部比较，不发送给模型；通知也不复制 MCP 指令正文。模型继续依据当前 system 指令和 tools 声明执行。

MCP 客户端目前提供合并指令，因此一个服务器的说明变化可能让多个 MCP 工具被标为配置变化。通知覆盖模型实际可用的工具，不是零工具服务器的完整连接监控。运行期间工具目录仍按现有 run 生命周期刷新。

MCP 指令或工具 schema 改动仍可能改变请求前缀。这次没有用通知替代真实能力声明，也没有把任意 MCP 指令迁移到 user。

## 源码与验证

- `crates/agent/src/context.rs`：状态重放、增量与内部记录。
- `crates/agent/src/memory/index.rs`：保守分类与有界元数据读取。
- `crates/agent/src/runner.rs`：调用前刷新、持久化、取消及压缩后重建。
- `crates/harness/src/context.rs`：主对话的文件和 MCP 状态来源。
- `crates/harness/src/native_history.rs`：沿用的完整模型历史存储。
- `crates/harness/tests/support/context_reminders.rs`：Chat Completions、Responses、Messages 的 HTTP 边界与恢复检查。

设计依据与实验结果分别见 [源码调查](context-reminders-memory-study-2026-10-23.md) 和 [第一轮实验](context-reminder-trial-2026-10-23.md)。实验命中率不是本次生产接入后的实测承诺。

2026-10-23 验证：

- Agent 库与集成测试全量回归 390 项通过，7 项既有辅助测试忽略；之后补充目录不可用测试，memory 专项 9/9 通过。
- Harness 单元测试共 201 项通过，1 项在线 bootstrap 测试忽略；主提示词/用户提问/steering 集成测试共 12 项通过。新增的生产请求导出探针另外显式执行通过。
- 三种协议的 HTTP 边界检查覆盖：system 角色保留、旧历史稳定、无变化去重、删除、关闭/开启、从磁盘恢复，以及压缩后建立完整基线；没有内部 `UserMessage` 事件泄漏到界面。
- 真实 Write 工具修改索引后，在同一 run 的下一次模型调用中观察到更新；保存失败与取消不提前推进交付状态。
- `cargo check --locked --offline -p rcode-engine` 通过。已有的 dead_code / unused_imports 警告未在本任务中扩展处理。
- 通过授权服务，用 `deepseek-v4.1-flash` 回放接入后生产代码构造的四个请求，分别返回 `FACT-V1`、`FACT-V2`、`NONE`、`FACT-V3`，4/4 正确且无工具调用。原有 mock 历史保留，非完整桌面交互验收。

生产请求回放记录见 [verification.json](../output/cache-study/2026-10-23/production-context/live/verification.json)。四次缓存计数分别为未知、0%、0%、90.63%，没有构造公平的预热对照，不用这四次结果宣称生产累计命中率。此前 JavaScript 原型的 90.77% 也不替代生产测量。
