# 模型请求缓存诊断

用于定位缓存下降时，客户端实际请求是否改变了可复用的前缀。诊断不改变模型请求、提示词、工具声明、重试预算或会话统计。

## 开关与位置

默认关闭。在 RCode 数据目录创建独立的 `request-diagnostics.json`：

```json
{"enabled": true}
```

开关在下一次主对话 run 开始时读取，无需修改 `agent-settings.json`。设为 `false` 或移除开关文件后，后续 run 停止记录；正在进行的 run 按启动时的设置继续。无效配置按关闭处理。

日志位于同一数据目录的 `request-diagnostics/requests.jsonl`。单文件最多约 16 MiB，保留两个轮转备份，总量约 48 MiB。最多保留 4096 条消息指纹；超过时标记 `messagesTruncated`，完整请求摘要仍覆盖全部内容。每次实际 HTTP 尝试最多记录 32 份不同的响应元数据快照。

日志通过容量为 64 的队列在后台线程写入。目录不可写、队列满或记录超过 1 MiB 时会告警并跳过诊断，模型调用继续；进程突然退出可能丢失队列中的最后几条记录。关闭诊断不会删除已有日志。

## 记录范围

- 实际发送前的完整 JSON 请求 SHA-256 与 UTF-8 字节数；在观测投影截断或脱敏之前计算，认证 headers 不参与。
- system / instructions、按 wire 顺序排列的工具声明、其余请求参数的独立指纹。
- 按 wire 顺序排列的消息角色、完整消息 SHA-256 和字节数。Chat 的 system 消息也保留在该序列中，便于发现位置变化。
- provider 协议与 URL origin，以及去除认证信息和查询参数后的 endpoint 摘要；不记录 URL 用户信息、路径正文、查询参数或 headers。
- 每次实际 HTTP 尝试的独立 ID、状态码、耗时、完成/失败/取消/丢弃状态，以及失败类别。应用层续跑和 transport 重试均产生新的 attempt；同一逻辑模型调用保留相同 callId 和递增 attempt。
- 返回的实际 model、响应 ID 与 HTTP request ID；仅保留有界、符合标识符形态且不含凭据形态的值。可用于识别服务返回的标识变化，无法证明网关内部路由。
- 原始 usage 中允许列表内的 token 数值和 null，包括缓存标准字段、网关别名和 details。保留冲突零值，不用归一化值覆盖原字段；任意字符串和未知字段不进入日志。
- 完成后的归一化用量，以及 `model_call_link` 中的 ModelCall ID。该 ID 与界面持久化 usage part 的 ID 相同，可与既有统计关联；诊断不改变计费统计。

不保存提示词、文件/工具结果、推理正文、schema 正文、图片内容或凭据。摘要可用于比较内容是否相同，并不等于能还原原文。当前接入主对话及目标验证；标题、摘要和子智能体的独立调用尚未接入此日志，不能当作供应商总账。

## 离线分析

```powershell
node scripts/analyze-request-diagnostics.mjs C:\Users\chengliang\.rcode\request-diagnostics
node scripts/analyze-request-diagnostics.mjs C:\Users\chengliang\.rcode\request-diagnostics <native-session-id>
```

工具按旧到新的轮转文件读取，忽略尚未写完的最后一行；损坏的完整行会报错。按 native session 与调用目的比较相邻请求，输出 system、工具、参数变化、首个不匹配消息的位置（通过 commonMessages）、只追加历史、历史减少或无法完整比较等结果，同时保留原始 usage 快照、实际返回模型、重试序号与 ModelCall ID。

`append_only` 或 `identical` 配合低缓存率，说明已采集的客户端请求没有出现相应前缀改动，值得继续检查上游缓存；不能据此确定是落盘延迟、路由变化还是服务端统计问题。`history_changed` 也不是缓存失效的直接证明：某些协议会合并相邻消息，仍需结合具体字段和上游序列化判断。指纹被截断、日志缺行、轮转丢失基线或没有返回缓存计数时，不作确定归因。

此日志从启用后的新请求开始采集，不能补回历史对话当时未保存的 wire 指纹或原始 usage。
