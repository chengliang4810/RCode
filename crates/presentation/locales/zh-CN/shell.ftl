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
shell-drop-to-unpin = 拖到这里取消固定
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
shell-sidebar-view-location = 位置
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
# 固定写入被拒绝：注册表快照还没到达。
shell-pins-syncing = 固定项仍在同步
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
shell-pinned = 已固定
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
