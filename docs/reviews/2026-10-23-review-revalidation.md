# RCode 历史审查复验与修复

日期：2026-10-23。修改前基线：`ebb2e6d5455df5fc29800bd226b2c7565d52edf3`，`main`，工作区干净。

输入为本地 `RCode-analysis.md`，报告声明基于 2026-10-22 下载的 main，没有准确提交 SHA。此次直接对照当前源码，修复确认成立的问题并执行回归；不把原报告的 Verified 当作本次动态验证。完全信任直接执行的既定产品决策保留，没有增加项目 MCP 审批或 WebFetch 内网限制。

## 分项记录

- [更新、下载、插件与发布流程](2026-10-23-updates-plugins.md)：H1/H2/M7/M8/M11 及相关低危。
- [MCP、Agent 与 Client](2026-10-23-mcp-agent.md)：H3/H4/H5/M2/M3/M4/M9/M13/M19 及相关低危。
- [UI、GPU、移动端和文本](2026-10-23-ui-gpu.md)：M5/M6/M12/M15/M16/M18、额外四项 Medium 及相关低危。

主任务补充核查：

| 报告项 | 结论、处理和验证依据 |
| --- | --- |
| H6 broadcast 积压导致监听永久退出 | 当前成立。LinkCache 的 wake/online 只在 Closed 时退出，Lagged 同样执行缓存失效逻辑并继续监听。`online_event_lag_does_not_disable_sign_out` 用超过通道容量的事件后再注销，验证 revoked 仍生效。 |
| H4 运行准备阶段主动停止 | 除 MCP 自身的连接期限外，harness 的 `mcp_for_run` 竞争当前运行取消令牌，避免停止后仍等初始化。`stop_interrupts_mcp_initialization_before_the_connection_deadline` 启动停在握手的 stdio 服务后停止，要求一秒内返回。 |
| M1 SSE EOF 丢末行 | 当前成立。EOF 刷新残余行和事件，运行时共用解码路径。完整终止事件缺分隔符可成功；普通 delta 仍保留文本并报告中断，损坏 UTF-8 仍报错。没有把任意 EOF 当完成。新增 parser 和真实 Responses runtime 回归，未声称实际账单已减少。 |
| M14 过期游标为所有历史 part 分配 slot | 当前成立。先用标量 ID 确认边界；过期游标只创建尾页 slot。保留对有效深层游标的完整搜索，避免原报告建议的提前退出破坏分页。`stale_cursor_is_bounded_without_discarding_a_deep_valid_cursor` 覆盖 2048-part 历史、深层有效游标和已删 part。查找失效游标仍需扫描 ID，不是 O(1) 索引。 |
| M17 流式更新克隆完整文本镜像 | 当前成立。LoroText 和本地 written 镜像均追加 suffix，后者保留既有容量；非追加更新保持字段刷新。`segment_writer_retains_text_capacity_and_supports_rewrites` 验证有预留容量时追加前后指针相同，且文本重写仍可持久化。全文相等/前缀比较仍存在，没有宣称整个流式路径变为线性。 |
| M10 WebSocket URL 中 token 未编码且可能被日志记录 | 未编码部分修复：device、role、connId、token 均按组件编码，覆盖 `+ & = # /` 的单测。**token 仍在 query**，日志暴露风险未消除；迁移认证载体需与远端 relay 协议配套，本仓库未包含生产 relay 服务实现。 |
| Low SQLite 初次创建后才 chmod | 当前成立。Unix 先以 0600 原子创建文件并设置权限，再打开 SQLite；权限操作失败时拒绝继续。Windows 不改变 ACL。Unix 专有路径未在本机动态运行。 |
| Low RPC unary 取消后 pending 留存 | 当前成立。发送前创建 RAII guard，future 丢弃立即移除 pending 并尽力发取消；也覆盖旧 subscribe API 在发送背压期间取消。两个回归验证 pending 立即清空及发出的取消帧。旧 API 成功返回裸 receiver 后，静默期间 drop 的取消仍依赖后续事件，生产宜逐步迁移 scoped API。 |
| Low 同一 RPC ID 覆盖旧任务 abort handle | 当前成立。替换时中止旧 handle。`reusing_request_id_cancels_the_previous_server_task` 验证旧 stream 的资源确实释放。 |
| Low 流式 RPC payload 多余 clone | 当前成立。`try_send` 直接接收 item 所有权，保留背压断流和取消行为。 |
| Low upload 阻塞与目录重扫 | 部分修复。RPC 的 append/commit 改为 spawn_blocking，不在 Tokio worker 同步解码与写文件；追加时统计目录的实现仍存在，没有声称整个上传复杂度降低。既有上传、队列附件集成回归覆盖结果与唤醒顺序。 |
| Low RPC WebSocket 默认消息大小与队列组合 | 仍使用库的有界默认消息限制和 256 深度队列；没有新增更小限制。RPC 包含完整文档/历史载荷，需先建立兼容的消息大小合同，避免任意缩小导致合法同步失败。 |
| Low agent_task_stores 无移除 | 当前 map 确实按会话保留任务存储，尚未增加会话删除/关闭通知以回收。运行间复用用于子任务交互，不能在一轮结束时直接清空。本轮清理了任务淘汰/存储释放时所属日志，但不能宣称此 map 已有容量上限。 |
| Low model HTTP client 初始化 expect | 当前仍存在，UI asset client 对应位置已改为错误返回。model 三种 provider 构造及请求 builder 共享 client，需要统一错误初始化契约，本次未扩大公共构造 API 变更。 |

## 正向改善与保留限制

- WebFetch 的隐藏块转换局部优化进行了修改前后独立编译对照，输出相同。10,000 个隐藏块、各转换 10 次，两轮旧版约 1.12/1.06 秒、新版约 84/50 毫秒；仅代表合成负载，详见 MCP/Agent 分项记录。
- 文本追加用指针保持断言验证复用分配；完成消息 tree 用 Arc 身份保持验证无关更新不触发重建；GPU 稳态 dummy 纹理创建从每帧 4 次降为 0，blur bind 循环内复用。这些是局部行为/资源改善，未宣称系统帧率或全会话内存下降比例。
- MCP 取消/超时后不重放副作用、WebFetch 超限在 EOF 前退出、事件积压后注销继续生效、更新缺摘要前置拒绝、缓存篡改拒绝均有确定性回归。
- GPU 原有 `light_fades_out_toward_the_edges_unless_opaque` 在本机失败；临时恢复原始 HEAD 的 GPU 文件并使用同一多 crate Cargo 参数构建后同样失败。已恢复本次源码，其余 15 项 GPU 回归通过（包含新增真实 bloom 光晕与跨帧复用对照）。不能将过滤后的通过描述为 GPU 全部通过。
- Linux/macOS 原生更新安装、WebKitGTK 关闭、真实 device loss、真实远端生产 MCP/relay 未联调。H2 仍未实现发布签名；HTTPS 元数据来源与制品摘要校验不能替代签名。
- 保留项另包括 Client checkpoint/rows 大小合同与 escort 崩溃恢复、Linux browser 关闭等待、shader 编译缓存淘汰、mobile visual 长会话工作线程回收和字体库超大 arena。它们在分项记录中明确列出。

## 验证结果

所有 Cargo 命令使用 `--locked --offline --target-dir C:\Users\chengliang\.codex\tmp\zeron-process-target`，避免在 D 盘重复构建缓存。Windows shell 相关测试临时将 Git Bash 目录加入 PATH。未修改生产配置、未发布版本、未推送远端。

| 验证 | 结果 |
| --- | --- |
| `cargo check --workspace --all-targets` | 通过；包含测试目标编译，没有宣称运行全部工作区测试。编译器仍有原有 unused/dead_code 及依赖 future-incompatibility 警告。 |
| `cargo test -p rcode-github -p rcode-update -p rcode-plugins --lib` | 5 / 32 / 22 项通过；Plugins 1 项 ignored 未运行。 |
| `cargo test -p rcode-agent -p rcode-mcp -p rcode-client` | Agent 单元 372、集成 25；MCP 单元 13、集成 11；Client 单元 22、集成 25 全部通过。Agent 7 项、MCP 3 项原有 ignored 测试未运行。 |
| `cargo test -p rcode-doc -p rcode-rpc -p rcode-model -p rcode-preview -p rcode-harness --lib` | Doc 165、RPC 26、Model 44、Preview 9、Harness 202 项通过；Model 2 项、Harness 1 项 ignored 未运行。 |
| `cargo test -p rcode-visual-gpu --lib`（与前述核心 crate 同组构建） | 最终 15 项通过；另 1 项 light fade 故障在修改前后均失败，已明确过滤。真实 scene、纹理、反馈 buffer、bloom 对照均执行。 |
| `cargo test -p rcode-ui --lib --no-run`，随后运行该新编译测试二进制，过滤 `asset_http::tests:: attachments::tests:: sound::tests:: background_preparation_ --test-threads=1` | 43 项通过；其余 1785 项未运行。新增缓存复用测试首轮失败后修正了位置 upsert 的失效逻辑，再跑全部 43 项通过。 |
| `cargo test -p rcode-engine --lib uploads::` 和 `--test queued_attachments` | 上传单元 13 项、经真实上传 RPC 的队列附件集成 3 项通过，覆盖文件落地前延迟、落地后恢复以及非图片附件。 |
| `python -m unittest discover -s scripts -p test_release_packaging.py` | 7 项通过。 |
| `git -c core.whitespace=cr-at-eol diff --check` | 通过，保留各源文件原有 LF/CRLF，没有将整文件换行变化混入修复。 |
