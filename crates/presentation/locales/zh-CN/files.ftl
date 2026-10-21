files-title = 文件
files-empty = 此文件夹为空
files-image-preview = 图片预览
files-open-external = 用默认应用打开
files-unsaved = 未保存的更改
files-save-failed = 无法保存：{ $reason }

# 资源管理器外壳与头部。
files-surface-label = 工作区文件
files-label-projectless = { $path } 中的文件 · { $device }
files-label-projectless-device = 文件 · { $device }
files-label-projectless-path = { $path } 中的文件
files-refresh-now = 立即刷新
files-refresh-aria = 立即刷新工作区文件
files-retry = 重试
files-search-placeholder = 搜索文件
files-hide-ignored = 隐藏隐藏和忽略的文件
files-show-all-files = 显示所有文件（包括隐藏文件）
files-mutation-changed = 操作完成前工作区已更改

# 空状态与加载状态。文件树每行一个；搜索结果各自显示；预览居中显示。
files-empty-folder = 空文件夹
files-loading = 加载中…
files-loading-file = 正在加载文件…
files-loading-preview = 正在加载预览…
files-loading-image = 正在加载图片…
files-load-more = 加载更多…
files-no-workspace = 此对话没有可用的工作区。
files-service-starting = 工作区服务正在启动中。
files-service-unavailable = 工作区服务不可用。

# 工作区文件搜索。
files-searching = 搜索中…
files-no-files-found = 未找到文件。
files-search-results-label = 工作区文件模糊搜索结果
files-search-limit = 仅显示前 { $count } 个匹配项

# 文件树、上下文菜单与内联重命名/删除。
files-file-tree-label = 工作区文件树
files-menu-add-to-chat = 添加到对话
files-menu-copy-path = 复制路径
files-menu-copy-paths = 复制路径
files-menu-copy-full-path = 复制完整路径
files-menu-copy-full-paths = 复制完整路径，每行一个
files-menu-rename = 重命名…
files-menu-delete = 删除…
files-menu-delete-many = 删除 { $count } 项…
files-tree-error-retry = { $message } — 重试
files-rename-invalid-name = 请输入不含路径分隔符的单个文件名
files-rename-entry-changed = 条目已更改；请取消并重新打开"重命名"
files-delete-entry-changed = 条目已更改；请重新打开"删除"以确认其当前内容
files-delete-title = 永久删除？
files-delete-message-directory = 永久删除 { $name }？当前文件夹的全部内容都将被删除。编辑器缓冲区将被保留以供恢复。
files-delete-message-file = 永久删除 { $name }？此操作无法撤销。编辑器缓冲区将被保留以供恢复。
files-delete-message-many-directories = 永久删除 { $names }？文件夹及其当前全部内容都将被删除。编辑器缓冲区将被保留以供恢复。
files-delete-message-many-files = 永久删除 { $names }？此操作无法撤销。编辑器缓冲区将被保留以供恢复。
files-drag-move-to-root = 移动到工作区根目录
files-drag-workspace-root = 工作区根目录

# 编辑器上下文菜单。
files-editor-cut = 剪切
files-editor-paste = 粘贴
files-editor-select-all = 全选

# 预览的只读与错误状态。没有专门 Key 的消息回退到 files-cannot-preview；
# 读取失败沿用传输层自身的措辞，只有文件不存在时单独命名。
files-file-not-found = 未找到文件。
files-cannot-preview = 无法预览此文件。
files-readonly-binary = 无法预览二进制文件。
files-readonly-encoding = 不支持此文件编码。
files-readonly-symlink = 符号链接目标为只读。
files-readonly-permission = 权限不足。
files-readonly-too-large = 此文件过大，无法预览。
files-readonly-mixed-endings = 含混合换行符的文件为只读。
files-readonly-outside = 只读：在此对话文件夹之外。
files-truncated-readonly = 大文件预览已被截断且为只读。
files-image-removed = 此图片已从工作区移除。
files-checkout-unavailable = 工作区检出身分不可用
files-image-timeout = 图片预览超时
files-image-too-large = 图片超出预览内存上限
files-image-preview-suspended = 工作区已更改。图片预览已暂停。

# 预览生命周期横幅：关闭或切换目标时的未保存修改，以及编辑器下方在磁盘上
# 发生变化的文件。
files-changed-banner = 工作区已更改。请切换回来保存，或丢弃这些修改。
files-changes-not-saved = 更改无法安全保存。
files-saving-before-close = 关闭前正在保存更改…
files-keep-open = 保持打开
files-discard-changes = 丢弃更改
files-keep-editing = 继续编辑
files-reload-from-disk = 从磁盘重新加载
files-discard-reload = 丢弃并重新加载
files-discard-unsaved = 丢弃未保存的更改？
files-changed-outside = 此文件在 RCode 之外被更改。

# 编辑器面包屑的保存状态标签。
files-save-failed-label = 保存失败
files-save-conflict = 保存冲突
files-conflict-detail = 文件已在磁盘上更改。你的编辑器缓冲区已保留。
files-deleted-on-disk = 已在磁盘上删除
files-deleted-detail = 文件已在磁盘上删除。你的编辑器缓冲区已保留。
files-changed-on-disk = 磁盘上已更改
files-changed-detail = 文件已在磁盘上更改。保存前请先检查。

# 编辑器工具栏按钮。
files-show-markdown-code = 显示 Markdown 代码
files-preview-markdown = 预览 Markdown
files-save-file-aria = 保存文件
files-reveal-in-tree = 在树中显示文件
files-open-default-editor = 用默认编辑器打开
files-disable-word-wrap = 禁用自动换行
files-enable-word-wrap = 启用自动换行

# 编辑器评审评论。
files-remove-comment = 移除评论
files-comment-placeholder = 请求修改…
files-comment-placeholder-add = 添加评论…
files-comment = 评论
files-open-comment-aria = 打开第 { $line } 行的评论
files-comment-line-aria = 在第 { $line } 行评论

# Markdown 预览媒体。
files-media-connection-unavailable = 工作区图片连接不可用
files-image-outside-workspace = 图片路径在工作区之外
files-media-memory-limit = 文档媒体预览已达内存上限
files-mermaid-diagram = Mermaid 图表
files-rendering-diagram = 正在渲染图表…
files-diagram-limit = 文档图表预览数量已达上限
files-open-image-link = 打开图片链接
files-image-limit = 文档图片预览数量已达上限

# 资源管理器底部区段。
files-section-subagents = 子智能体
files-section-chats = 对话
files-new-side-chat = 新建侧边对话
files-fork-this-chat = 复刻此对话
files-fork = 复刻
files-collapse = 折叠
files-expand = 展开
files-subagents-empty = 子智能体创建后将显示在此处
files-side-chats-empty = 侧边对话创建后将显示在此处
files-open-subagent-aria = 打开子智能体 { $title }
files-open-side-chat-aria = 打开侧边对话 { $title }
files-section-toggle-aria = { $action } { $label }
files-group-header-aria = { $action } { $label } 子智能体
files-show-more = 再显示 { $count } 项
files-group-finished = 已完成
files-group-completed = 成功
files-group-failed = 失败

# 资源管理器中的 Git 状态提示行。
files-git-unavailable-incomplete = Git 状态不可用或不完整
files-git-unavailable = Git 状态不可用

# 附件的侧栏文案与暂存错误。随提示词一起发送的纯附件正文与引用尾注不在此处：
# 它们是持久化的消息文本，而非展示文案。
files-rail-attached-image = 附带 { $count } 张图片
files-rail-attached-file = 附带 { $count } 个文件
files-attach-not-image = { $name } 不是有效的图片。
files-attach-unreadable = 无法读取 { $name }。
files-attach-not-file = { $name } 不是文件。
files-attach-too-large = { $name } 过大（上限 24 MB）。
files-attach-unsupported = { $name } 不是受支持的图片格式。

# 共享图片解码路径的失败信息。它们和上面的读取失败落在同一个错误位置，同属展示文案。
files-image-size-limit = 图片超出预览大小上限
files-image-svg-size-limit = 处理后的 SVG 超出预览大小上限
files-generated-image-unsupported = 不支持的生成图片
files-generated-image-mismatch = 生成图片的格式与其元数据不符

# 渲染后的 Markdown 与图片周围的控件：代码围栏、图表，以及内嵌图片的放大入口。
files-copy-code = 复制代码
files-copied = 已复制
files-show-diagram = 显示图表
files-show-source = 显示源码
files-enlarge-image = 放大图片

# 渲染后的 Markdown 中链接的菜单。「在 RCode 中打开」和默认打开方式所在行属于阅读界面，
# 因此和它们放在一起。
files-link-open-rcode = 在 RCode 中打开
files-link-show-in-folder = 在文件夹中显示
files-link-open-external = 在外部浏览器中打开
files-open-links-in-rcode = 链接在 RCode 中打开
files-open-links-in-rcode-checked = 链接在 RCode 中打开，已勾选
files-open-links-in-rcode-unchecked = 链接在 RCode 中打开，未勾选
