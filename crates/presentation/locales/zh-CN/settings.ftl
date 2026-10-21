settings-title = 设置
settings-appearance-effort-theme = 思考强度主题
settings-appearance-effort-accent = 跟随主题
settings-appearance-effort-purple = 紫色
settings-appearance-effort-terracotta = 陶土橙
settings-appearance-effort-deep-blue = 深蓝
settings-appearance-effort-sky-blue = 天蓝
settings-appearance-effort-aurora = 极光渐变
settings-appearance-effort-silver = 银白
settings-appearance-effort-gold = 金色
settings-language = 语言
settings-language-system = 跟随系统
settings-language-en = English
settings-language-zh = 简体中文
settings-language-restart = 重启后生效。
# 切换语言后的确认框：语言在启动时确定，只有新进程才能显示新语言。
settings-language-restart-title = 重启以切换语言？
settings-language-restart-body = RCode 在启动时确定显示语言。立即重启即可看到新语言；否则将在下次打开 RCode 时生效。
settings-language-restart-action = 立即重启
# 设置 → 通用 → 数据位置。这一行写入的指针文件由进程在启动时读取，所以改动在重启后
# 生效；配置目录（设备 id、锁、provider 密钥、设置）不会跟着搬。
settings-data-location = 数据位置
settings-data-location-default = 默认
settings-data-location-detail-default = 会话、附件、克隆的仓库与 worktree 都放在配置目录里，和你的设置在一起。
settings-data-location-detail-custom = 会话、附件、克隆的仓库与 worktree 都放在上面这个目录里。设置与 provider 密钥仍留在配置目录。
settings-data-location-change = 更改…
settings-data-location-use-default = 恢复默认位置
settings-data-location-restart-title = 重启以使用新位置？
settings-data-location-restart-body = RCode 在启动时读取数据位置。立即重启即可搬过去；否则将在下次打开 RCode 时生效。
settings-data-location-restart-action = 立即重启
settings-data-location-rejected = 这个目录不能用：{ $reason }
settings-memory = 跨会话记忆
settings-skills = 技能
settings-skills-empty = 没有找到技能
settings-hooks = 钩子

# 设置 → 通知。各开关本身就是开关控件，因此行标题同时作为其无障碍名称；
# 只有子选项需要单独的文案。
settings-notifications-title = 通知
settings-notifications-desktop-section = 桌面
settings-notifications-sounds-section = 提示音
settings-notifications-session-sounds = 会话提示音
settings-notifications-completed = 任务完成
settings-notifications-completed-toggle = 任务完成提示音
settings-notifications-input = 需要输入
settings-notifications-input-toggle = 需要输入提示音
settings-notifications-errors = 错误与断连
settings-notifications-errors-toggle = 错误与断连提示音
settings-notifications-desktop = 桌面通知
settings-notifications-background = 仅在后台时
settings-notifications-background-toggle = 仅在 RCode 处于后台时通知
settings-notifications-unavailable = 父级设置关闭时不可用

# 设置 → 外观。主题、强调色和壁纸相关行。主题名和强调色预设名属于主题 crate 的数据，
# 保持原样；这里只放它们周围的标签。
settings-appearance-title = 外观
settings-appearance-color-scheme = 配色方案
settings-appearance-material = 材质与背景
settings-appearance-motion = 动态效果
settings-appearance-fonts = 字体与布局

settings-appearance-conversation-width = 对话宽度
settings-appearance-conversation-width-detail = 消息和输入框的最大宽度。
# 滑杆两端的标签：数字加单位，随语言变化的只有数字的分组方式。
settings-appearance-width-min = 560 px
settings-appearance-width-max = 1200 px

settings-appearance-font-interface = 界面字体
settings-appearance-font-interface-detail = 菜单与对话
settings-appearance-font-terminal = 终端字体
settings-appearance-font-terminal-detail = 终端输出 · 仅限等宽字体
settings-appearance-font-code = 代码与差异字体
settings-appearance-font-code-detail = 代码、差异和文件
settings-appearance-font-size = { $font }字号
settings-appearance-font-picker-aria = { $font }：{ $value }
settings-appearance-font-search = 搜索字体
settings-appearance-font-no-match = 没有匹配的字体
settings-appearance-font-none = 没有可用字体
settings-appearance-font-missing = 此设备上没有 { $font }“{ $requested }”，已改用 { $effective }。

settings-appearance-light-theme = 浅色主题
settings-appearance-dark-theme = 深色主题
settings-appearance-light-themes = 浅色主题
settings-appearance-dark-themes = 深色主题
settings-appearance-variant-light = 浅色
settings-appearance-variant-dark = 深色

settings-appearance-accent = 强调色
settings-appearance-accent-wallpaper = 已启用壁纸取色；关闭后将使用此强调色。
settings-appearance-glass = 玻璃
settings-appearance-surface-theme-default = 跟随主题
settings-appearance-surface-frosted = 磨砂
settings-appearance-surface-opaque = 实心
settings-appearance-surface-default-frosted = 跟随主题：磨砂
settings-appearance-surface-default-opaque = 跟随主题：实心
settings-appearance-surface-translucent = 半透明表面
settings-appearance-surface-solid = 实心表面
settings-appearance-match-wallpaper = 匹配壁纸颜色
settings-appearance-match-wallpaper-detail = 用壁纸颜色作为强调色、高亮和细微的表面色调。

settings-appearance-reduce-motion = 减少动态效果
settings-appearance-reduce-motion-system-on = 跟随系统的减少动态效果设置。活动指示改用柔和的亮度脉冲。
settings-appearance-reduce-motion-system-off = 跟随系统，当前允许动态效果。
settings-appearance-reduce-motion-on = 动画直接跳到最终状态。
settings-appearance-reduce-motion-off = 即使系统要求减少动态效果，动画也照常播放。
settings-appearance-pause-animations = 后台时暂停动画
settings-appearance-pause-animations-detail = RCode 不是当前活动窗口时暂停动画。

settings-appearance-background = 新会话背景
settings-appearance-background-adjust = 调整
settings-appearance-background-replace = 更换图片
settings-appearance-background-remove = 移除
settings-appearance-background-choose = 选择图片
settings-appearance-background-unavailable = 图片不可用
settings-appearance-background-choose-replacement = 重新选择一张或移除。
settings-appearance-background-none = 未选择图片
settings-appearance-choose-background = 选择新会话输入区背景

settings-appearance-adjust-background = 调整背景
settings-appearance-adjust-background-detail = 拖动调整位置，滚动或双指缩放。
settings-appearance-close-adjustment = 关闭背景调整
settings-appearance-background-zoom = 背景缩放
settings-appearance-background-changed = 背景已更换。请关闭后重新调整新图片。
settings-appearance-background-preparing = 正在准备预览…
settings-appearance-apply = 应用

settings-appearance-wallpaper-folder = 壁纸文件夹
settings-appearance-wallpaper-folder-none = 选择存放壁纸图片的文件夹。
settings-appearance-wallpaper-folder-detail = { $folder } · 按 { $shortcut } 随机换一张
settings-appearance-choose-folder = 选择文件夹
settings-appearance-choose-wallpaper-folder = 选择壁纸文件夹
settings-appearance-shuffle = 换一张
settings-appearance-background-effect = 背景效果

settings-appearance-theme-library = 主题库
settings-appearance-add-theme = 添加主题
settings-appearance-imported = 已导入
settings-appearance-linked = 已关联
settings-appearance-self-contained = 自包含快照
settings-appearance-variant-count = { $count } 个变体
settings-appearance-theme-status = { $source } · { $variants } · { $path }
settings-appearance-theme-warning = 使用上次可用的版本 · { $message }
settings-appearance-theme-reload = 重新加载
settings-appearance-theme-reveal = 在文件夹中显示
settings-appearance-theme-review = 查看
settings-appearance-theme-duplicate = 复制为可编辑
settings-appearance-theme-unlink = 取消关联
settings-appearance-theme-remove = 移除
settings-appearance-theme-mapping = 主题映射
settings-appearance-done = 完成

settings-appearance-import-title = 添加主题
settings-appearance-import-detail = 把本地主题导入主题库，或保持与源文件的关联。
settings-appearance-import-source = 来源
settings-appearance-import-source-placeholder = 主题文件、package.json 或扩展文件夹
settings-appearance-import-source-required = 请选择本地主题文件或扩展文件夹。
settings-appearance-import-choose-source = 选择主题来源
settings-appearance-import-browse = 浏览…
settings-appearance-import-keep-updated = 保持更新
settings-appearance-import-copy = 导入副本
settings-appearance-import-copy-detail = 与源文件相互独立。
settings-appearance-import-link = 关联源文件
settings-appearance-import-link-detail = 从磁盘上的文件重新加载变更。
settings-appearance-import-detected = 检测到的主题
settings-appearance-import-select-variant = 请至少选择一个要导入的变体。
settings-appearance-import-details = 详情
settings-appearance-import-hide-details = 收起详情
settings-appearance-import-analyze = 分析主题
settings-appearance-import-selected = 导入所选
settings-appearance-import-auto-detect = RCode 会自动识别浅色和深色变体。
settings-appearance-import-compile-failed = { $name } 无法编译 · { $message }

# 映射报告：导入器做了什么的诊断列表。其中的角色名、源键和消息来自主题 crate，
# 不翻译；这里只翻译引出它们的标签。
settings-appearance-report-summary = 已映射 { $mapped } · 已调整 { $adjusted } · 推断/回退 { $fallback } · 不支持 { $unsupported } · 警告 { $warnings } · 校验 { $validation }
settings-appearance-report-adjusted = 已调整 · { $role } { $original } → { $resolved } · { $reason }
settings-appearance-report-fallback = 回退 · { $message }
settings-appearance-report-warning = 警告 · { $message }
settings-appearance-report-unsupported = 不支持 · { $message }
settings-appearance-report-validation = 校验 { $category } { $severity } · { $message }
settings-appearance-report-mapping = { $role } ← { $key }

# 设置 → 通用，以及快捷键表。
settings-general-title = 通用
settings-shortcuts-title = 键盘快捷键
settings-shortcuts-subtitle = 点击一个快捷键，然后按下新的组合键。
settings-shortcuts-restore = 恢复默认
settings-shortcuts-unique = 快捷键不能重复。
settings-shortcuts-press-escape = 按 Esc 取消。
settings-shortcuts-press-keys = 请按快捷键…
settings-shortcuts-reset-aria = 重置 { $name } 快捷键
settings-shortcuts-change-aria = 修改 { $name } 快捷键：{ $combo }
settings-shortcuts-navigate-pane = 在当前聚焦的面板内导航。
settings-shortcuts-send-with = 发送消息的按键
settings-shortcuts-compact-mode = 紧凑模式
settings-shortcuts-compact-mode-detail = 折叠思考过程和工具调用。
settings-shortcuts-compact-picker = 紧凑模型选择器
settings-shortcuts-compact-picker-detail = 用滑块调整推理强度，需要时再展开模型列表。
settings-shortcuts-escape = 用 Esc 停止智能体
settings-shortcuts-escape-detail = 没有对话框或菜单打开时。
settings-shortcuts-escape-aria = 用 Esc 停止运行中的智能体
settings-shortcuts-refused-key = 请使用 { $modifiers } 加字母、数字、功能键或导航键。
settings-shortcuts-modifiers-platform = Control、Option 或 Command
settings-shortcuts-modifiers-other = Control 或 Alt
settings-shortcuts-refused-reserved = { $combo } 已保留给输入框。
settings-shortcuts-refused-taken = { $combo } 已分配给 { $owner }。
settings-shortcuts-refused-taken-page = { $combo } 已分配给「设置 → { $page }」中的 { $owner }。
settings-links = 链接
settings-links-pull-requests = 打开 Pull Request 的位置
settings-links-pull-requests-detail = 适用于 PR 徽标和 Pull Request 面板。

# 快捷键表的分组标题。键名来自页面自身的分组，稳定不变；这里只是标题的文字。
settings-shortcuts-group-appearance = 外观
settings-shortcuts-group-files = 文件
settings-shortcuts-group-browser = 浏览器
settings-shortcuts-group-panels = 面板
settings-shortcuts-group-sessions = 会话
settings-shortcuts-group-projects = 项目
settings-shortcuts-group-jump = 跳转到会话
settings-shortcuts-group-appshots = Appshots

# 设置 → 文件。
settings-files-title = 文件
settings-files-autosave = 自动保存
settings-files-autosave-delay = 自动保存延迟
settings-files-word-wrap = 自动换行
settings-files-show-hidden = 显示隐藏和忽略的文件

# 设置 → Appshots。能力描述来自 appshots 模块；这里是行标题和捕获设置文案。
settings-appshots-title = Appshots
settings-appshots-capture = 捕获 Appshot
settings-appshots-capture-detail = 截取后会先暂存待确认，绝不自动发送。
settings-appshots-sound = 捕获提示音
settings-appshots-sound-detail = Appshot 就绪时播放提示音。
settings-appshots-shortcut = 全局快捷键
settings-appshots-shortcut-preferred = 首选快捷键
settings-appshots-destination = 目标位置
settings-appshots-destination-auto = 使用当前打开的会话；没有打开会话时使用新建会话输入区。
settings-appshots-destination-last = 使用当前打开的会话；没有则回到上次捕获 Appshot 的会话。
settings-appshots-destination-new = 在新会话输入区中暂存截图，不影响已有草稿。
settings-appshots-window-capture = 窗口捕获
settings-appshots-application-text = 应用文本
settings-appshots-open-settings = 打开系统设置
settings-appshots-allow-window = 允许窗口捕获
settings-appshots-enable-text = 允许文本捕获
settings-appshots-check-again = 重新检查
settings-appshots-check-again-aria = 重新检查权限
settings-appshots-check-hint = 改过权限？回到 RCode 后再点一次「重新检查」。

# 设置 → 设备。

# 设置 → 通用 → 记忆与会话命名。
settings-memory-section = 记忆
settings-personalization-instructions = 自定义指令
settings-personalization-instructions-placeholder = 填写希望助手遵循的规则和偏好，例如回复语言、代码风格和工作方式。
settings-marketplace = 插件市场
settings-marketplace-official = 官方市场
settings-marketplace-search = 搜索插件
settings-marketplace-refresh = 刷新
settings-marketplace-cached = 正在显示缓存的市场目录
settings-marketplace-installed = 已安装
settings-marketplace-installing = 安装中…
settings-marketplace-unsupported = 暂不支持 LSP
settings-marketplace-empty = 没有找到匹配的插件
settings-marketplace-load-failed = 无法加载插件市场：{ $error }
settings-marketplace-install-failed = 无法安装插件：{ $error }
settings-marketplace-install-aria = 安装插件 { $name }
settings-personalization-reload = 重新加载并丢弃未保存修改
settings-personalization-load-failed = 无法加载自定义指令：{ $error }
settings-personalization-save-failed = 无法保存自定义指令：{ $error }
settings-memory-detail = 关闭后，下一条消息不再加载记忆。已保存的内容保留在磁盘上。
settings-thread-naming = 会话命名
settings-thread-naming-reset-aria = 用会话所属的智能体命名会话
settings-thread-naming-follows = 每个会话由它自己的智能体命名。
settings-thread-naming-model = 用小模型命名，快速又省成本。


# 设置 → 技能。两个受管理的技能根目录：项目的 `.agents/skills` 与
# 个人的 `~/.rcode/skills`。页面标题和空列表沿用上面的
# `settings-skills` 与 `settings-skills-empty`。
settings-skills-subtitle = 智能体可以按需加载的 Markdown 指令。项目技能存放在项目的 `.agents/skills` 文件夹；个人技能存放在你的用户文件夹，可跨项目使用。
settings-skills-project-section = 项目技能
settings-skills-user-section = 个人技能
settings-skills-other-section = 其他
settings-skills-new = 新建技能
settings-skills-new-aria = 新建技能
settings-skills-enable-aria = 启用 { $name }
settings-skills-disable-aria = 停用 { $name }
settings-skills-edit = 编辑
settings-skills-edit-aria = 编辑 { $name }
settings-skills-delete-aria = 删除 { $name }
settings-skills-empty-detail = 创建一个技能，给智能体一条可复用的指令。
settings-skills-loading = 正在加载技能…
settings-skills-load-failed = 无法加载技能：{ $error }
settings-skills-create-title = 新建技能
settings-skills-edit-title = 编辑技能
settings-skills-name-label = 名称
settings-skills-name-placeholder = skill-name
settings-skills-description-label = 描述
settings-skills-description-placeholder = 这个技能做什么、什么时候用
settings-skills-body-label = 指令内容
settings-skills-body-placeholder = 技能被使用时智能体遵循的指令
settings-skills-body-keeps-existing = 留空表示保留现有指令。
settings-skills-scope-label = 保存位置
settings-skills-scope-project = 项目
settings-skills-scope-user = 个人
settings-skills-scope-project-detail = 保存到本项目的 `.agents/skills` 文件夹。
settings-skills-scope-user-detail = 保存到你的个人技能文件夹，所有项目共用。
settings-skills-name-invalid = 只能使用字母、数字、连字符和下划线。
settings-skills-description-required = 描述不能为空：智能体靠它判断何时使用这个技能。
settings-skills-create-failed = 创建技能失败：{ $error }
settings-skills-update-failed = 保存技能失败：{ $error }
settings-skills-delete-title = 删除技能？
settings-skills-delete-body = “{ $name }”将被永久删除，且无法撤销。
settings-skills-delete-failed = 删除技能失败：{ $error }
settings-skills-toggle-failed = 更新 { $name } 失败：{ $error }


# 设置 → 子智能体。原生 agent 的 `Task` 工具按定义文件派生：项目级
# `.rcode/agents` 与个人级 `~/.rcode/agents`。正文是子 agent 的系统提示；
# 模型选择器是用户在定义自带 `model:` 之上的覆盖。
settings-subagents = 子智能体
settings-subagents-subtitle = 智能体可以把任务委派给的具名代理。定义决定子代理的指令、可用工具和运行模型。
settings-subagents-project-section = 项目子智能体
settings-subagents-user-section = 我的子智能体
settings-subagents-other-section = 其他
settings-subagents-new = 新建子智能体
settings-subagents-new-aria = 创建子智能体
settings-subagents-enable-aria = 启用 { $name }
settings-subagents-disable-aria = 停用 { $name }
settings-subagents-edit-aria = 编辑 { $name }
settings-subagents-delete-aria = 删除 { $name }
settings-subagents-empty = 没有子智能体
settings-subagents-empty-detail = 创建一个子智能体，让助手把专注的任务交给独立上下文。
settings-subagents-loading = 正在加载子智能体…
settings-subagents-load-failed = 无法加载子智能体：{ $error }
settings-subagents-create-title = 新建子智能体
settings-subagents-edit-title = 编辑子智能体
settings-subagents-name-label = 名称
settings-subagents-name-placeholder = agent-name
settings-subagents-description-label = 何时使用
settings-subagents-description-placeholder = 用模型能读懂的话说明这个代理的用途
settings-subagents-prompt-label = 指令
settings-subagents-prompt-placeholder = 子智能体运行时的系统提示
settings-subagents-tools-label = 允许的工具
settings-subagents-tools-placeholder = Read, Grep, Glob（留空表示允许全部）
settings-subagents-disallowed-tools-label = 禁止的工具
settings-subagents-disallowed-tools-placeholder = Bash, Write（在允许列表之后应用）
settings-subagents-tools-summary = 工具：{ $tools }
settings-subagents-disallowed-tools-summary = 禁止：{ $tools }
settings-subagents-model-label = 模型
settings-subagents-model-detail = 短别名，按你配置的服务商解析；"inherit" 表示跟随当前对话的模型。
settings-subagents-model-aria = 修改 { $name } 的模型，当前为 { $model }
settings-subagents-scope-project-detail = 保存到本项目的 `.rcode/agents` 文件夹。
settings-subagents-scope-user-detail = 保存到你的代理文件夹，可跨项目使用。
settings-subagents-name-invalid = 只能使用字母、数字、短横线和下划线。
settings-subagents-description-required = 请说明助手何时应该委派给这个代理。
settings-subagents-create-failed = 无法创建子智能体：{ $error }
settings-subagents-update-failed = 无法保存子智能体：{ $error }
settings-subagents-delete-title = 删除子智能体？
settings-subagents-delete-body = “{ $name }”将被永久删除，无法撤销。
settings-subagents-delete-failed = 无法删除子智能体：{ $error }
settings-subagents-toggle-failed = 无法更新 { $name }：{ $error }
settings-subagents-model-failed = 无法修改 { $name } 的模型：{ $error }

# Presence label still used by device rows in state.rs.
settings-devices-never-seen = 从未上线

# 快捷键行名称（分组标题的 key 已在上方）。
settings-shortcut-name-random-wallpaper = 随机壁纸
settings-shortcut-name-capture-appshot = 截取 Appshot
settings-shortcut-name-save-file = 保存文件
settings-shortcut-name-browser-reload = 重新加载浏览器页面
settings-shortcut-name-toggle-sidebar = 切换左侧栏
settings-shortcut-name-toggle-changes = 切换右侧栏
settings-shortcut-name-toggle-files = 切换文件面板
settings-shortcut-name-toggle-terminal = 切换终端
settings-shortcut-name-new-session = 新建会话
settings-shortcut-name-new-project = 新建项目
settings-shortcut-name-open-model-picker = 打开模型选择器
settings-shortcut-name-next-session = 下一个会话或右侧标签页
settings-shortcut-name-prev-session = 上一个会话或右侧标签页
settings-shortcut-name-archive-session = 归档会话
settings-shortcut-name-jump-session = 跳转到会话 { $slot }
settings-shortcut-name-pull-request-view = 内置 Pull request 视图
settings-shortcut-name-default-browser = 默认浏览器

# 设置 → 通用 → 链接：Pull request 打开的位置。
settings-pr-destination-view = 内置视图
settings-pr-destination-browser = 默认浏览器

# Appshot destination select options.
settings-appshots-destination-automatic = 自动
settings-appshots-destination-last-session = 上一个会话
settings-appshots-new-session = 新建会话

# 设置 → MCP。服务器配置写在 `mcp.json`，与其他 MCP 客户端同一形状，
# 所以页面只展示状态，不接管每个字段。
settings-mcp = MCP
settings-mcp-subtitle = 来自 Model Context Protocol 服务器的工具。服务器条目就是其他 MCP 客户端读取的那份 JSON；智能体看到的名字是 `mcp__服务器__工具`。
settings-mcp-project-section = 项目服务器
settings-mcp-user-section = 我的服务器
settings-mcp-empty = 没有 MCP 服务器
settings-mcp-empty-detail = 点击“新建 MCP 服务器”添加一个服务器。
settings-mcp-loading = 正在加载服务器…
settings-mcp-load-failed = 无法加载 MCP 服务器：{ $error }
settings-mcp-write-failed = 无法保存服务器：{ $error }
settings-mcp-configured = 已配置
settings-mcp-not-connected = 未连接
settings-mcp-edit = 编辑
settings-mcp-edit-title = 编辑服务器
settings-mcp-json-placeholder = "command": "npx", "args": ["-y", "server"]
settings-mcp-json-invalid = 这不是合法的 JSON：{ $error }

# Rows an installed plugin contributes; read-only.
settings-skills-plugin-section = 来自插件
settings-subagents-plugin-section = 来自插件

# Servers an installed plugin contributes; read-only.
settings-mcp-plugin-section = 来自插件

# 设置 → 插件。插件是一个带清单的目录，贡献目录就在旁边；页面管理生命周期，
# 不编辑插件自己的文件。
settings-plugins = 插件
settings-plugins-subtitle = 已安装的扩展。插件可以贡献技能、子智能体、钩子和 MCP 服务器；它自己的文件在这里是只读的。
settings-plugins-empty = 没有安装插件
settings-plugins-empty-detail = 前往插件市场安装插件。
settings-plugins-loading = 正在加载插件…
settings-plugins-load-failed = 无法加载插件：{ $error }
settings-plugins-remove = 移除
settings-plugins-enable-aria = 启用 { $name }
settings-plugins-disable-aria = 停用 { $name }
settings-plugins-toggle-failed = 无法更新 { $name }：{ $error }
settings-plugins-uninstall-failed = 无法移除插件：{ $error }

# 外观 → 数学。
settings-appearance-math-size-detail = 相对于周围文字。

settings-mcp-new = 新建 MCP 服务器
settings-mcp-form = 表单
settings-mcp-name = 名称
settings-mcp-transport = 连接方式
settings-mcp-command = 命令
settings-mcp-args = 参数
settings-mcp-env = 环境变量（JSON 对象）
settings-mcp-url = URL
settings-mcp-headers = HTTP 请求头（JSON 对象）
settings-mcp-json-label = 服务器配置
settings-mcp-saving = 正在保存…
settings-mcp-name-required = 请填写服务器名称，或在 JSON 配置中包含名称。
settings-mcp-name-exists = 已存在同名服务器。
settings-mcp-args-invalid = 请使用空格分隔的参数或 JSON 字符串数组，并检查引号是否闭合。
settings-mcp-env-invalid = 环境变量必须是值为字符串的 JSON 对象。
settings-mcp-headers-invalid = 请求头必须是值为字符串的 JSON 对象。
settings-mcp-object-required = 服务器配置必须是 JSON 对象。
settings-mcp-single-server = 每次只能新增一个 MCP 服务器。
settings-mcp-config-invalid = 服务器配置无效：{ $error }
settings-mcp-context-changed = 引擎连接已变化，请关闭窗口后重试。

settings-subagents-inherit = 继承默认
settings-subagents-all-tools = 全部工具
settings-subagents-tool-count = { $count } 个工具
settings-subagents-group-count = { $count } 项
settings-subagents-builtin = 内置子智能体



settings-subagents-model-deleted = 已删除，继承对话模型

settings-commands = 命令
settings-commands-name = 名称
settings-commands-body = Markdown 提示词
settings-commands-new = 新建命令
settings-commands-reload = 重新加载
settings-commands-delete = 删除命令
settings-commands-edit = 编辑
settings-commands-enable = 启用
settings-commands-disable = 停用
settings-commands-loading = 加载中…
settings-commands-empty = 暂无命令。
settings-commands-plugin = 插件

settings-marketplace-previous = 上一页
settings-marketplace-next = 下一页
settings-marketplace-page = 第 { $page } / { $pages } 页
