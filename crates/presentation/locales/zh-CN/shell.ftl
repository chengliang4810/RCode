shell-new-session = 新建会话
shell-search-placeholder = 搜索会话
shell-empty-title = 还没有会话
shell-empty-body = 新建一个开始吧。
shell-archive = 归档
shell-unarchive = 恢复
shell-scope-local = 本地

# 启动画面的状态行，出现在任何窗口有内容之前。
shell-boot-status = 正在准备 RCode 环境

# 原生菜单栏。在语言确定之后、启动时构建一次。
shell-menu-about = 关于 RCode
shell-menu-check-for-updates = 检查更新…
shell-menu-settings = 设置
shell-menu-services = 服务
shell-menu-hide = 隐藏 RCode
shell-menu-hide-others = 隐藏其他
shell-menu-show-all = 全部显示
shell-menu-quit = 退出 RCode
shell-menu-edit = 编辑
shell-menu-undo = 撤销
shell-menu-redo = 重做
shell-menu-cut = 剪切
shell-menu-copy = 复制
shell-menu-paste = 粘贴
shell-menu-select-all = 全选
shell-menu-view = 显示
shell-menu-appearance-system = 外观：跟随系统
shell-menu-appearance-light = 外观：浅色
shell-menu-appearance-dark = 外观：深色
shell-menu-window = 窗口
shell-menu-minimize = 最小化
shell-menu-zoom = 缩放
shell-menu-close-window = 关闭窗口

# 设置左侧导航栏。这些是导航项打开的页面名；每个页面自己的标题用
# `settings-*` 键。导航项的元素 id 来自 `SettingsSection::slug`，不来自这些
# 标签，所以改动文案不会影响测试选择器。
shell-settings-nav-general = 通用
shell-settings-nav-appearance = 外观
shell-settings-nav-notifications = 通知
shell-settings-nav-shortcuts = 快捷键
shell-settings-nav-providers = 模型设置
shell-settings-nav-skills = 技能
shell-settings-nav-subagents = 子智能体
shell-settings-nav-mcp = MCP
shell-settings-nav-plugins = 插件
shell-settings-nav-personalization = 个性化
shell-settings-nav-devices = 设备
shell-settings-nav-files = 文件
shell-settings-nav-appshots = Appshots
shell-settings-nav-archived = 已归档会话

# 输入区的选择器：模型、项目、引用列表，以及紧凑模式下的芯片条。
# 其中的模型名、项目名、分支名都是用户数据，不翻译。
pickers-no-project = 不使用项目
pickers-no-project-selected = 未选择项目
pickers-new-project = 新建项目
pickers-no-refs = 没有找到引用。
pickers-no-agents = 没有可用的 agent
pickers-no-agents-hint = 在「设置 → 模型设置」中启用智能体。
pickers-add-model = 添加模型
pickers-default = 默认
pickers-fast-mode = 快速模式
pickers-reasoning-effort = 推理强度

# 会话卡片：这个会话在哪里运行、它的侧边对话操作，以及项目的运行操作。
# 设备名、项目名和分支名都是用户数据，不翻译。
shell-session-unknown-device = 未知设备
shell-session-no-project = 无项目
shell-session-new-side-chat = 新建侧边对话
shell-session-fork-chat = 复刻对话
shell-session-actions = 操作

shell-engine-offline = 引擎未连接

# 自定义侧栏分组。
shell-sidebar-syncing = 侧栏仍在同步，请稍后再试。
shell-sidebar-section-placeholder = 分组名称
shell-sidebar-section-options = 分组选项
shell-sidebar-drop-here = 将会话拖到这里
shell-sidebar-edit-section = 编辑分组
shell-sidebar-new-section = 新建分组
shell-sidebar-archive-all = 全部归档
shell-sidebar-group-hint = 按你喜欢的方式给会话分组
shell-sidebar-create-section = 创建分组
shell-sidebar-archive-failed = 有 { $count } 个会话未能归档，请重试。

# 输入区上的拖放提示。
shell-drop-to-attach = 拖放到此处添加附件
shell-drop-to-mention = 拖放到此处引用
# 关掉最后一个窗口在 Windows 和 Linux 上会结束进程，正在跑的会话也随之结束。
# （macOS 上窗口只是视口，引擎继续运行，所以关窗口不会问。）
shell-exit-busy-title = 还有会话在运行
shell-exit-busy-body = 关闭 RCode 会中断它们，以及它们尚未做完的事。仍要关闭吗？
shell-exit-busy-confirm = 仍要关闭

# 侧边栏与工作区操作：此前没有本地化字符串。
shell-onboarding-add-project = 添加一个项目开始使用
shell-onboarding-add-project-detail = 项目是你某个设备上的一个文件夹。
shell-drop-to-unpin = 拖到这里取消置顶
shell-star-on-github = 在 GitHub 上加星
shell-scroll-to-bottom = 滚动到底部
shell-run-failed = 运行失败
shell-sending = 发送中…
shell-files-tab = 文件
shell-diffs-tab = 差异
shell-history-tab = 历史
shell-back = 返回
shell-sidebar-view-options = 侧边栏视图选项
# 侧边栏视图选项菜单：三个子菜单，以及每个选项一行。
shell-sidebar-view-group-organize = 整理
shell-sidebar-view-group-sort = 排序
shell-sidebar-view-group-show = 显示
shell-sidebar-view-by-project = 按项目
shell-sidebar-view-one-list = 无
shell-sidebar-view-last-updated = 最近更新
shell-sidebar-view-created = 创建时间
shell-sidebar-view-branch = 分支
shell-sidebar-view-pull-request = 拉取请求
shell-sidebar-view-harness = 智能体
shell-sidebar-view-project-icon = 项目图标
shell-sidebar-view-project-label = 项目名称
shell-sidebar-view-project-filter = 项目筛选
shell-sidebar-view-archived = 已归档
shell-in-home = 在个人目录
shell-compact = 紧凑
shell-create-section = 新建分组
shell-all-projects = 全部项目
shell-new-project = 新建项目
shell-loading-locations = 正在加载位置…
shell-show-hidden-folders = 显示隐藏文件夹
shell-rename-menu = 重命名…
shell-remove-menu = 移除…

# 输入区引用选择器里的触发标签与切换提示。
pickers-select-ref = 选择引用
pickers-from-ref = 基于 { $ref }
pickers-no-ref = 无引用
pickers-switching = 正在切换…

# 纯图标控件的无障碍名称。
shell-aria-settings-sections = 设置栏目
shell-aria-close-tab = 关闭标签页
shell-aria-new-chat-in-project = 在项目中新建会话
shell-aria-add-project = 添加项目

# Fallback labels when the registry has not supplied a name yet.
shell-unknown-device = 未知设备
shell-new-session-fallback = 新建会话

# Subagent pane: stopping one child without stopping the conversation.
shell-subagent-stop = 停止子智能体
shell-subagent-stop-aria = 停止子智能体 { $name }
shell-subagent-already-finished = 该子智能体已经结束。
shell-subagent-stop-failed = 无法停止子智能体：{ $error }

# 输入区的权限档位 chip：下一条消息在不询问的情况下可以做什么。
# 搜索框占位文案，每个选择器一个。
pickers-search-placeholder = 搜索…
pickers-search-refs = 搜索引用…
pickers-search-projects = 搜索项目…
pickers-search-models = 搜索模型…
pickers-search-locations = 搜索位置…
pickers-search-folders = 搜索文件夹…
# 侧边栏分组列表的标题。
shell-sessions-header = 会话
settings-worktrees-home = 主目录
settings-worktrees-use-this-folder = 使用此文件夹
settings-worktrees-browse = 浏览
surface-chrome-close-tab-aria = 关闭标签页
files-workspace-files-aria = 工作区文件
files-refresh-workspace-aria = 立即刷新工作区文件
settings-appearance-math-aria = 渲染数学公式
settings-worktrees-use-default-aria = 使用默认 worktree 位置
settings-worktrees-choose-aria = 选择 worktree 文件夹
settings-worktrees-custom-aria = 自定义 worktree 位置
session-info-uncommitted = 未提交的更改
shell-pull-requests = Pull request
surface-chrome-new-tab = 新建标签页
surface-chrome-close-tab = 关闭标签页
settings-worktrees-reset-to-default = 恢复默认
# 侧边栏会话状态标签（行的状态图形及其无障碍名称）。
shell-status-working = 运行中
shell-status-input = 等待输入
shell-status-failed = 失败
shell-status-queued = 排队中
shell-status-done = 完成
shell-status-idle = 空闲
# 置顶写入被拒绝：注册表快照还没到达。
shell-pins-syncing = 置顶列表仍在同步
shell-pin = 置顶
shell-unpin = 取消置顶
shell-remote-session = 远程会话
shell-files-fallback-title = 文件
shell-unrelated-archive-error = 无关的归档错误
shell-side-chats = 侧边对话
# 会话卡中的分组控件。
shell-lanes-collapse-finished = 收起 { $count } 个已结束的子智能体
shell-lanes-expand-finished = 展开 { $count } 个已结束的子智能体
shell-lanes-finished-count = 已结束 { $count } 个
# 项目位置选择器与项目管理对话框。
shell-project-home = 主目录
shell-project-no-locations = 未找到位置
shell-project-no-locations-hint = 尝试选择主目录或输入磁盘名称。
shell-project-no-folders-here = 此处没有文件夹
shell-project-no-folders-here-hint = 按 { $shortcut } 添加此文件夹，或按 ← 返回。
shell-project-searching-home = 正在搜索主目录…
shell-project-searching-home-hint = 正在主目录下查找匹配的文件夹。
shell-project-no-home-matches-yet = 尚无匹配的文件夹
shell-project-home-indexing-hint = 主目录仍在建立索引；请继续输入或重试。
shell-project-no-folder-matches = 没有匹配的文件夹
shell-project-no-folder-matches-hint = 输入类似 ~/code 或 /mnt 的路径以跳转。
shell-project-back-to-commands = 返回命令面板
shell-project-open = 打开
shell-project-select = 选择
shell-project-navigate = 导航
shell-project-close = 关闭
shell-project-adding = 正在添加…
shell-project-add = 添加项目
shell-project-name-placeholder = 项目名称
shell-project-rename-title = 重命名项目
shell-project-remove-title = 移除项目？
shell-project-remove-fallback = 此项目
shell-project-remove-confirmation-one = 移除“{ $name }”将永久删除其 1 个会话。此操作无法撤销。
shell-project-remove-confirmation-many = 移除“{ $name }”将永久删除其 { $count } 个会话。此操作无法撤销。
shell-project-remove-confirm = 移除
shell-tabs-collapse-panel = 收起面板
shell-tabs-expand-panel = 展开面板
shell-tabs-hide-files = 隐藏文件面板
shell-tabs-show-files = 显示文件面板
shell-tabs-hide-session-details = 隐藏会话详情
shell-tabs-show-session-details = 显示会话详情
shell-tabs-hide-pull-requests = 隐藏拉取请求面板
shell-tabs-show-pull-requests = 显示拉取请求面板
shell-pinned = 置顶
shell-archived = 已归档
shell-new-chat-in-project = 在项目中新建会话
shell-clear-project-filter-to-rename = 清除项目筛选后才能重命名此会话
shell-searching-home-status = 正在搜索…
shell-indexing-home-status = 建立索引中…
shell-home-folder-aria = { $parent } 中的 { $name }
shell-settings-open-tooltip = 设置 · { $shortcut }
shell-settings-close-tooltip = 关闭设置 · { $shortcut }
shell-sync-queued = 同步排队中，更改已保存
shell-syncing = 正在同步…
shell-sync-offline = 离线，更改已保存
shell-sync-storage-error = 无法保存更改
shell-sync-offline-saved = 离线，发送内容已保存
shell-sync-reconnecting = 正在重新连接…
shell-aria-account-menu = 账户菜单：{ $name }
shell-aria-settings = 设置
shell-aria-close-settings = 关闭设置
memory-empty = 暂无记忆
memory-list-truncated = 文件列表已截断。

pickers-models-empty = 没有匹配的模型

pickers-context-window = 上下文窗口
pickers-context-standard = 标准
pickers-change-model = 更换模型
pickers-reasoning-keys = 使用左右方向键调整推理强度，Home 和 End 选择首末档位。
pickers-reasoning-minimal = 极低
pickers-reasoning-low = 低
pickers-reasoning-medium = 中
pickers-reasoning-high = 高
pickers-reasoning-xhigh = 极高
pickers-reasoning-max = 最高
pickers-reasoning-ultra = 超强
pickers-reasoning-ultracode = 超级编码
pickers-reasoning-ultrathink = 超级思考

shell-subagent-background = 转为后台

shell-chat-delete-title = 删除会话？
shell-chat-delete-body = 将永久删除“{ $title }”，此操作无法撤销。
shell-chat-delete-disconnected = 引擎未连接，无法删除会话。
shell-chat-delete-failed = 删除会话失败：{ $error }
shell-archived-delete-title = 删除所有已归档会话？
shell-archived-delete-all = 全部删除
shell-archived-delete-body = 将永久删除 { $count } 个已归档会话，此操作无法撤销。
shell-archived-delete-failed = { $count } 个已归档会话删除失败，请重试。

# Remaining UI display text. Stable identifiers are kept in code.
audit-command-new-chat = 新建会话
audit-command-new-project = 新建项目
audit-command-settings = 打开设置
audit-command-theme-system = 跟随系统主题
audit-command-theme-light = 切换到浅色主题
audit-command-theme-dark = 切换到深色主题
audit-command-archive = 归档会话
audit-palette-all = 全部
audit-palette-threads = 会话
audit-palette-commands = 命令
audit-palette-files = 文件
audit-palette-recent-files = 最近文件
audit-palette-quick-actions = 快捷操作
audit-palette-recent-threads = 最近会话
audit-palette-in-files = 文件内容
audit-palette-indexing = 正在建立索引…
audit-palette-searching = 正在搜索…
audit-palette-search = 搜索会话、命令和文件…
audit-palette-empty = 无结果
audit-palette-empty-hint = 可搜索命令、会话、项目或设备。
audit-palette-update-content = 请更新会话所在设备上的 RCode，以搜索文件内容。
audit-palette-update-files = 请更新会话所在设备上的 RCode，以搜索文件。
audit-palette-unreachable = 无法连接会话所在的设备
audit-palette-failed = 搜索失败
audit-notify-finished = 运行已完成
audit-notify-input = 等待你的回答
audit-notify-failed = 运行失败
audit-notify-offline = 设备已离线
audit-notify-reconnecting = RCode 正在重新连接
audit-notify-unavailable = 连接不可用
audit-copy-conversation-link = RCode 会话链接
audit-copy-session-id = 智能体会话 ID
audit-copy-conversation-done = 已复制 RCode 会话链接
audit-copy-conversation-unready = 会话链接尚未就绪
audit-copy-session-done = 已复制智能体会话 ID
audit-copy-path-done = 已复制路径
audit-side-chat = 侧会话
audit-side-chat-rename-parent = 请打开父会话后重命名此侧会话
audit-session-title = 会话标题
audit-engine-disconnected = 引擎未连接
audit-engine-disconnected-period = 引擎未连接。
audit-add-project = 添加项目
audit-unarchive = 取消归档
audit-archive = 归档
audit-session-time = 会话时间
audit-discard-title = 丢弃工作树更改？
audit-discard-body = 丢弃此工作树中的所有未提交更改？此操作无法撤销。
audit-discard = 丢弃更改
audit-discard-failed = 无法丢弃更改
audit-copy-path = 路径
audit-copy-named = 已复制{ $name }
audit-unsaved-tab = { $name }，有未保存的更改
audit-action-cancel = 取消
audit-action-close = 关闭
audit-action-ok = 确定
audit-action-later = 稍后
audit-action-hide = 隐藏
audit-update-checking = 正在检查更新…
audit-update-check-failed = 无法检查更新
audit-update-current = RCode 已是最新版本
audit-update-background = 正在后台下载更新。
audit-update-restart-body = 重启以完成更新，或在下次退出 RCode 时安装。
audit-update-cli = 在终端运行 `rcode update` 以安装更新。
audit-update-unmanaged = 此 RCode 无法自动更新，请下载新版本。
audit-try-again = 重试
audit-download = 下载
audit-update-restart = 重启并更新
audit-update-download-page = 打开下载页面
audit-update-your-version = 当前版本为 RCode { $version }。
audit-update-newest = { $version } 已是最新版本。
audit-update-available = RCode { $version } 可用
audit-update-on-version = 当前版本为 { $version }。
audit-update-ready-title = RCode { $version } 已就绪
audit-update-download-failed = 下载失败：{ $error }
audit-tab-close = 关闭标签页
audit-tab-others = 关闭其他标签页
audit-tab-left = 关闭左侧标签页
audit-tab-right = 关闭右侧标签页
audit-side-chat-start-first = 请先开始会话，再创建侧会话。
audit-pins-unconfirmed = 引擎未确认侧栏更改
audit-pins-offline = 引擎未连接，侧栏未更改。
audit-pins-waiting = 正在等待引擎确认上一项侧栏更改。
audit-pins-cancelled = 无法确认侧栏更改，已取消排队操作。请等待引擎连接后重试。
audit-pins-failed = 无法保存侧栏更改：{ $error }
audit-code-scroll = 使用水平滚动
audit-code-fit = 适应宽度
audit-diagram-complex = 图表超出预览复杂度限制
audit-diagram-large = 图表超出预览大小限制
audit-diagram-failed = 无法渲染图表
audit-more-lines = … 还有 { $count } 行
audit-models-loading = 正在加载模型…
audit-models-no-agents = 没有可用的智能体
audit-models-select = 选择模型
audit-history-unknown = 未知
audit-image = 图片
audit-update-strip-available = 有可用更新 — v{ $version }
audit-update-strip-downloading = 正在下载 v{ $version }…
audit-update-strip-failed = 更新失败：{ $error }
audit-update-strip-cli = 有可用更新 — v{ $version } · 运行 `rcode update`
audit-update-strip-download = 有可用更新 — v{ $version } · 从 GitHub 下载
audit-update-strip-ready = 更新已就绪 — 重启以应用
audit-update-restarting = 正在重启…
audit-update-read-only = 请将 RCode 移至“应用程序”文件夹并重新打开，以启用更新。
audit-update-not-writable = RCode 无权替换 { $path } 中的安装文件。
audit-copy-codex-link = Codex 会话链接
shell-processes-title = 后台进程
shell-processes-stopping = 正在停止
shell-processes-recovered = 已恢复
shell-processes-stop-aria = 停止 { $command }

shell-output-open = 查看 { $command } 的输出
shell-output-running = 运行中
shell-output-failed = 运行失败
shell-output-stopped = 已停止
shell-output-exited = 已结束
shell-output-copy = 复制
shell-output-file = 打开日志文件
shell-output-pause = 暂停跟随
shell-output-follow = 跟随输出
shell-output-retry = 重试
shell-output-empty = 暂无输出
shell-output-error = 无法读取日志：{ $error }
shell-output-rotated = 日志已轮转，仅保留最近输出。
