conversation-empty-title = 问点什么
conversation-empty-body = 描述你想完成的事。
conversation-stop = 停止
conversation-send = 发送
conversation-queued = 已排队
conversation-thinking = 思考中
conversation-working = 处理中
conversation-interrupted = 已中断
conversation-compacted = 较早的对话已压缩，以腾出上下文。
conversation-attached-images = 已附加 { $count } 张图片

# 工具卡片：工具显示名与工具组摘要行。工具的参数字面内容（路径、命令、网址、
# 搜索词）不翻译，也不在这里出现。未识别的工具依然有卡片（tool-unknown），
# 不会被丢弃。
tool-run = 运行
tool-read = 读取
tool-write = 写入
tool-edit = 编辑
tool-patch = 补丁
tool-search = 搜索
tool-glob = 匹配
tool-fetch = 抓取
tool-web = 网页
tool-todo = 待办
tool-mcp = MCP
tool-agent = 智能体
tool-unknown = 工具
tool-patch-workspace = 工作区
tool-search-in = 在 { $path } 中查找 { $pattern }
tool-todo-progress = 已完成 { $done }/{ $total }

tool-summary-ran = 运行了 { $count } 个命令
tool-summary-edited = 编辑了 { $count } 个文件
tool-summary-read = 读取了 { $count } 个文件
tool-summary-searched = 搜索了 { $count } 次
tool-summary-fetched = 抓取了 { $count } 个页面
tool-summary-todos = 更新了待办
tool-summary-called = 调用了 { $count } 个工具
tool-summary-none = { $count } 个工具
tool-summary-failed = { $count } 个失败
# 思考项与笔记项由界面自身合成，不会进入共享摘要，因此它们的措辞也放在这里。
tool-summary-thoughts = 思考了 { $count } 次
tool-summary-notes = 写了 { $count } 条笔记
conversation-thought-process = 思考过程
conversation-wrote-a-note = 写下
conversation-payload-diff = 差异
conversation-payload-output = 输出
conversation-show-full = 查看完整{ $what }
conversation-show-full-size = 查看完整{ $what }（{ $size }）
conversation-loading-full = 正在加载完整{ $what }…
conversation-load-full-failed = 无法加载完整{ $what } — 点击重试


# 运行中的状态行，以及一次发送会经过的几种状态。
conversation-sending = 发送中
conversation-queued-will-send = 已排队 — 将自动发送
conversation-send-undelivered = 未送达 — 点击重试
conversation-worked-for = 用时 { $duration }
conversation-elapsed-seconds = { $seconds }秒
conversation-elapsed-minutes = { $minutes }分{ $seconds }秒
conversation-elapsed-hours = { $hours }小时{ $minutes }分
conversation-elapsed-days = { $days }天{ $hours }小时

# 工作指示器旁轮换的状态词。各语言保持相同词条，仅描述整体工作状态。
conversation-flavour-processing = 处理中
conversation-flavour-thinking = 思考中
conversation-flavour-analyzing = 分析中
conversation-flavour-pondering = 琢磨中
conversation-flavour-deliberating = 推敲中
conversation-flavour-organizing = 梳理中
conversation-flavour-formulating = 构思中
conversation-flavour-reasoning = 推演中
conversation-flavour-distilling = 提炼中
conversation-flavour-untangling = 理顺中

# 对话界面自身的装饰文字：媒体占位、元信息条，以及交互结束后留下的胶囊。
# 其中的名称、标题、设备 id 都是数据，保持原样。
conversation-loading-image = 正在加载图片…
conversation-image-unavailable = 图片不可用
conversation-image = 图片
conversation-copy-path = 复制路径
conversation-copy-image = 复制图片
conversation-show-more = 展开
conversation-show-less = 收起
conversation-expand-message = 展开消息
conversation-collapse-message = 收起消息
conversation-preview-generated-image = 预览生成的图片
conversation-loading-generated-image = 正在加载生成的图片…
conversation-generated-image-unavailable = 生成的图片不可用
conversation-preview-name = 预览 { $name }
conversation-remove-name = 移除 { $name }
conversation-preview-appshot = 预览 { $app } 截图：{ $title }
conversation-appshot = 截图
conversation-uploading-appshot = 正在上传截图…
conversation-loading-appshot = 正在加载截图…
conversation-appshot-unavailable = 截图不可用
conversation-uploading-percent = 正在上传 { $percent }%
conversation-uploading = 正在上传…
conversation-forked-from = 分叉自
conversation-question = 提问
conversation-awaiting-answer = 等待你的回答…
conversation-subagent = 子智能体
conversation-message-from = 来自 { $name } 的消息

# 输入框：占位文字、命令面板、补全弹层，以及它上报的失败。
# 命令名、模型 id 和路径保持原样。
conversation-do-anything = 说点什么…
conversation-answer-placeholder = 输入你自己的回答，或选择上方选项
conversation-answer-placeholder-picked = 输入你自己的回答；留空则使用所选选项
conversation-select-multiple = 可选择多个选项。
conversation-wizard-back = 返回
conversation-wizard-next = 下一步
conversation-wizard-submit = 提交
conversation-send-message = 发送消息
conversation-queue-message = 加入队列
conversation-attach-images = 添加图片
conversation-attach = 添加
conversation-commands-group = 指令
conversation-skills-group = 技能
conversation-command-model = RCode：选择智能体、模型与推理
conversation-command-new = RCode：开始新会话
conversation-command-resume = RCode：搜索并打开会话
conversation-command-settings = RCode：打开设置
conversation-command-diff = RCode：打开变更
conversation-command-files = RCode：打开项目文件
conversation-command-terminal = RCode：打开终端
conversation-command-rename = RCode：重命名此会话
conversation-command-stop = RCode：停止当前运行
conversation-no-files-available = 没有可用的文件
conversation-no-matching-files = 没有匹配的文件
conversation-attached-image = 已附加图片
conversation-attached-file = 已附加文件
conversation-no-skills-project = 此项目没有可用的技能
conversation-agent-no-skills = 该智能体未提供技能
conversation-no-commands-or-skills = 没有可用的命令或技能
conversation-no-slash-commands = 该集成中没有可用的斜杠命令
conversation-no-matching-skills = 没有匹配的技能
conversation-no-matching-commands-or-skills = 没有匹配的命令或技能
conversation-no-matching-commands = 没有匹配的命令
conversation-search-older-device = 会话设备运行的是较旧的 RCode — 更新后即可搜索其文件
conversation-device-unreachable = 无法连接到会话设备
conversation-file-search-failed = 文件搜索失败
conversation-skills-need-update = 技能需要所选设备上的引擎更新。更新后请重启该设备的 RCode。
conversation-commands-need-update = 命令需要所选设备上的引擎更新。更新后请重启该设备的 RCode。
conversation-couldnt-load-skills = 无法加载该智能体的技能
conversation-couldnt-load-commands = 无法加载该智能体的命令
conversation-command-discovery-needs-connection = 发现智能体命令需要连接
conversation-update-to-send-references = 更新所选设备的 RCode 后才能发送文件、命令或技能引用。你的草稿已保留。
conversation-engine-not-connected = 引擎未连接
conversation-update-engine-to-queue = 更新会话引擎，以便在回复期间排队消息。
conversation-couldnt-stage-attachment = 无法在本地暂存附件。
conversation-couldnt-upload-attachment = 无法上传附件 — 设备可能离线。
conversation-couldnt-create-side-chat = 无法创建侧边会话。
conversation-remove-appshot-limit = 请先移除一张截图再添加（暂存截图上限 96 MB）。
conversation-offline-will-send = 离线 — 恢复联网后会自动发送消息。
conversation-will-send-on-reconnect = 连接恢复后消息会自动发送。
conversation-send-failed = 发送失败：{ $error }
conversation-send-failed-no-queue-id = 发送失败：队列未返回 id
conversation-stop-failed = 停止失败：{ $error }
conversation-answer-failed = 回答失败：{ $error }
conversation-image-load-failed = 无法加载图片。请重试打开。

# 待发送队列。其中的设备 id 是数据。
conversation-queue-editing-on = 正在 { $device } 上编辑
conversation-queue-needs-review = 待复核
conversation-queue-edit = 编辑
conversation-queue-removing = 正在移除…
conversation-queue-remove = 移除
conversation-queue-save = 保存到队列
conversation-queue-saving = 正在保存…
conversation-queue-editing-in-composer = 正在输入框中编辑
conversation-queue-steer = 引导
conversation-queue-send-next = 下轮发送
conversation-queue-send-now = 立即发送
conversation-queue-steer-hint = 引导（不打断当前工作）
conversation-queue-send-next-hint = 在下一轮发送（该智能体不支持中途引导）
conversation-queue-send-now-hint = 立即发送（中断当前工作）
conversation-queue-waiting-capabilities = 等待服务商能力
conversation-more-attachments = 另有 { $count } 个：{ $names }
conversation-more-attachments-hint = { $names }；编辑消息以查看全部
conversation-queue-removal-unsupported = 会话主机不支持安全移除队列项
conversation-queue-already-left = 该消息已离开队列
conversation-queue-remove-failed = 无法移除该消息
conversation-queue-edit-unsupported = 更新会话主机后才能安全编辑排队消息
conversation-queue-load-failed = 无法加载排队的附件或截图上下文。请检查连接并更新会话主机。
conversation-queue-invalid-lease = 会话主机返回了无效的编辑租约
conversation-queue-locked = 该排队消息正在其他设备上编辑
conversation-queue-gone = 该排队消息已不可用
conversation-queue-connect-to-edit = 连接到会话主机后才能编辑此消息
conversation-queue-lease-lost = 编辑租约已丢失；你的文本仍在编辑器中
conversation-queue-changed-elsewhere = 该消息已在其他设备上更改；你的编辑已保留在本地
conversation-queue-removed-locally = 排队消息已被移除；你的编辑已保留在本地
conversation-queue-lease-changed = 编辑租约已变更；你的文本仍在编辑器中
conversation-queue-host-unreachable = 无法连接到会话主机；你的编辑仍在编辑器中
conversation-queue-protection-expired = 编辑保护已过期；发送前请检查该消息
conversation-queue-actions-unsupported = 会话主机不支持队列操作
conversation-queue-reorder-failed = 无法重新排序队列
conversation-queue-send-failed = 无法发送该消息
conversation-queue-edit-removed = 排队消息已被移除；你的编辑仍在输入框中

# 输入框上方的待办列表托盘。条目文字来自智能体，不翻译。
conversation-todo-expand = 展开待办列表
conversation-todo-collapse = 收起待办列表
conversation-todo-all-done = 全部完成
conversation-todo-dismiss = 关闭待办列表
conversation-todo-earlier = 之前 { $count } 项
conversation-todo-later = 之后 { $count } 项
conversation-todo-show-earlier = 显示之前 { $count } 项
conversation-todo-hide-earlier = 隐藏之前 { $count } 项
conversation-todo-show-later = 显示之后 { $count } 项
conversation-todo-hide-later = 隐藏之后 { $count } 项

# 上下文占用弹层。used、total、remaining 是已分组好的数字。
conversation-context-window = 上下文容量
conversation-context-tokens = { $used }/{ $total } ({ $percent }%)
conversation-context-used = 已用 { $used } tokens
conversation-context-limit-unknown = 未上报上下文上限
conversation-context-capacity = 容量 { $total } tokens
conversation-context-waiting = 正在等待上下文用量
conversation-context-not-reported = 尚未收到上下文用量

# 会话用量与性能统计。
conversation-stats-title = 会话统计
conversation-stats-counts = { $turns } 轮 { $steps } 步
conversation-stats-recorded-counts = 已记录 { $turns } 轮 { $steps } 步
conversation-stats-recorded-usage = 已记录 Token 用量
conversation-stats-model-time = 模型用时
conversation-stats-ttft = 首 Token 平均（TTFT）
conversation-stats-speed = 输出速度（TPS）
conversation-stats-seconds = { $seconds }秒
conversation-stats-milliseconds = { $value }毫秒
conversation-stats-minutes = { $value }分
conversation-stats-hours = { $value }小时
conversation-stats-days = { $value }天
conversation-stats-tokens = { $value } Token
conversation-stats-tokens-thousand = { $value }千 Token
conversation-stats-tokens-ten-thousand = { $value }万 Token
conversation-stats-tokens-million = { $value }百万 Token
conversation-stats-tokens-hundred-million = { $value }亿 Token
conversation-stats-tokens-billion = { $value }十亿 Token
conversation-stats-token-rate = { $tokens }/秒
conversation-stats-session-usage = 会话 Token 用量
conversation-stats-turn-usage = 本轮用量
conversation-stats-turn-summary = 用量 { $tokens }
conversation-stats-model = 提供方 / 模型
conversation-stats-input = 输入
conversation-stats-uncached-input = 未缓存输入
conversation-stats-cache-read = 缓存读取
conversation-stats-cache-write = 缓存写入
conversation-stats-output = 输出
conversation-stats-cache-hit = 缓存命中率
conversation-stats-cache-summary = 缓存命中 { $percent }%

conversation-goal-active = 进行中
conversation-goal-paused = 已暂停
conversation-goal-complete = 已完成
conversation-goal-pause = 暂停
conversation-goal-resume = 继续
conversation-goal-clear = 清除
conversation-goal-iteration = 第 { $count } 轮
conversation-goal-command = 设置或管理会话目标
conversation-goal-input-hint = 目标或 pause / resume / clear
conversation-goal-active-title = 进行中的目标
conversation-goal-paused-title = 已暂停的目标
conversation-goal-complete-title = 已完成的目标

conversation-goal-edit = 编辑目标
conversation-goal-save = 保存
conversation-goal-reload = 重新载入
conversation-goal-missing = 目标已移除
conversation-goal-invalid = 目标内容需要 1 至 4000 个字符
conversation-goal-duration-seconds = { $count }秒
conversation-goal-duration-minutes = { $count }分
conversation-goal-duration-hours = { $count }时
conversation-goal-duration-days = { $count }天

conversation-load-earlier = 加载更早消息

# 移动端会话控件。模型与用户正文保持原文。
mobile-question-answered = 已回答问题
mobile-question-awaiting = 等待你的回答…
mobile-forked-from = 分支自 { $title }
mobile-show-less = 收起
mobile-show-more = 展开更多
mobile-new-file = 新文件
mobile-diff-truncated = 差异已截断，仅显示 { $total } 行中的前 { $limit } 行
mobile-more-lines = … 还有 { $count } 行
