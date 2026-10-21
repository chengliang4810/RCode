# 子智能体

内置子智能体按照本地 `D:/projects/ZCode` 的实现调整为两项：

| 角色 | 用途 | 工具 |
| --- | --- | --- |
| `general-purpose` | 复杂问题研究、代码搜索和多步任务 | 主对话可用的全部工具，但不允许再次调用 `Agent` |
| `Explore` | 跨文件、目录和命名方式的只读搜索 | `Bash`、`Glob`、`Grep`、`Read`、`WebFetch`、`WebSearch`、`TodoWrite` |

内置 `Plan`、`verification`、`statusline-setup` 和 `claude-code-guide` 不再注册。用户或项目自定义的同名角色仍按现有规则覆盖内置角色。

## 英文提示词

角色正文移植自 ZCode 的 `apps/zcode-cli/packages/core/src/subagent/general-purpose.ts` 和 `explore.ts`，使用独立 Glob/Grep 分支。只替换产品名称为 RCode，保留英文正文。运行时的完整调用描述来自 `profile.ts`，设置行使用 ZCode 服务层的简短英文描述。

原文保存在 `crates/agent/src/prompts/subagents/`，`source.json` 记录参考提交、文件、适配说明及正文哈希。快照测试校验角色正文与调用描述。

`general-purpose` 继承父会话已经固定的用户级和项目根目录 `AGENTS.md` 正文；内置 `Explore` 不注入这部分指令。不会将父会话的聊天历史和完整主系统提示词带入子会话。子会话另有工作目录、平台和实际 shell 信息。

Explore 不开放 Write/Edit 等文件修改工具。与 ZCode 一样，Bash 的只读行为由英文提示词约束，不在 Bash 执行层新增权限等级。

## 模型和思考强度

- 默认“继承默认”：继承父会话模型和思考强度。
- 主动选择具体模型时，默认使用其能力列表的最高思考档位；无思考能力时不显示档位控件。
- 单独修改档位只修改该子智能体配置。
- 模型和档位通过同一次 RPC、同一个 `subagentModels` 条目保存到现有 `agent-settings.json`，下一轮父对话读取，运行中的父轮次保持已有配置。
- 配置保留供应商和模型两个身份；带档位时编码为 `model:["providerId","modelId","high"]`，旧的两项编码继续可读。历史缺档位不自动补值，也不会沿用另一个显式模型的档位。
- 选择“继承默认”同时清除显式模型与档位。已删除的显式模型调用时返回不可用错误。
- 项目定义位于 `<workspace>/.rcode/agents`，用户定义位于 `~/.rcode/agents`，插件文件由插件管理。

## 设置界面与验证

内置组显示“2 项”，每行展示机器人图标和身份色点、角色名称、全部工具或工具数量、简短英文描述。右侧为裸模型名选择器和独立思考强度选择器；继承默认时不显示独立档位。内置角色不展示文件路径和重复来源徽标，不开放名称、正文、工具的编辑以及删除、启停。

本地模拟 HTTP/SSE 子会话测试检查实际请求的模型、思考强度、七项 Explore 工具、禁止递归以及 AGENTS.md 注入差异；另有持久化往返、提示词快照和选择器默认/保留档位测试。实际供应商调用不属于这些模拟测试的覆盖范围。
