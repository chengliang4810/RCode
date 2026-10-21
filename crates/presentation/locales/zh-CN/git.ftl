git-changes = 变更
git-history = 历史
git-branch = 分支
git-worktree = 工作树
git-pull-request = 拉取请求
git-no-changes = 没有变更
git-diff-empty = 没有可比较的内容

# 变更面板：补丁解析器得出的逐文件提示。
git-notice-mode-changed = 模式已更改为 { $mode }
git-notice-new-file = 新文件
git-notice-deleted-file = 已删除的文件
git-notice-renamed-from = 重命名自 { $from }
git-notice-binary = 二进制文件 — 不显示内容
git-notice-truncated = 差异已截断 — 仅显示 { $total } 行中的前 { $max } 行

# 未提交的工作树差异上方的标题。
git-uncommitted-changes = { $count } 个未提交的更改

# 范围名称，用作面板标题和范围菜单。
git-scope-working-tree = 工作树
git-scope-branch-changes = 分支更改
git-scope-latest-turn = 最近一轮
git-scope-commit = 提交

# 范围标题栏摘要。分支摘要与 git-scope-versus 在代码中拼接，因为基准引用是第二个值，
# 无法与复数选择器共用一条消息。
git-scope-summary-branch = { $count } 个更改的文件
git-scope-versus = 相对 { $base }
git-scope-summary-turn = 本轮有 { $count } 个更改的文件
git-scope-summary-commit = 本次提交有 { $count } 个更改的文件

# 空状态文案，每个范围一条。
git-clean-worktree = 没有未提交的更改
git-clean-branch = 没有分支更改
git-clean-branch-vs = 相对 { $base } 没有更改
git-clean-turn = 本轮没有更改
git-clean-history = 没有找到提交
git-clean-commit = 空提交

# 监视流中断或不可用。
git-stream-interrupted = 差异流中断 — 正在重试
git-watch-unavailable = 差异监视不可用：{ $reason }

# 面板标题栏控件。
git-refresh = 刷新
git-split-view = 并排视图
git-wrap-long-lines = 自动换行
git-discard-changes = 丢弃更改
git-expand-all-files = 展开所有文件
git-collapse-all-files = 折叠所有文件
git-open-in-file-browser = 在文件浏览器中打开

# 分支基准引用选择器。
git-search-branches-placeholder = 搜索分支…
git-no-branches = 没有分支
git-no-matching-branches = 没有匹配的分支

# 作用域差异内容区状态。
git-partial-snapshot = 部分快照
git-no-turn-yet = 尚无记录的一轮 — 请先发送一条消息
git-device-outdated = 此会话所在设备的 RCode 版本较旧 — 请更新后查看分支与轮次差异
git-preparing-diff = 正在准备差异…
git-comment-placeholder = 请求更改…
git-comment-add-aria = 添加评论
git-comment-edit-aria = 编辑评论
git-comment-remove = 移除评论
git-comment-submit = 评论

# 历史面板：引用类型（分支类型复用 git-branch）与引用标签。
git-ref-remote = 远程分支
git-ref-tag = 标签
git-ref-with-label = { $kind }：{ $label }
git-fetch-all = 获取全部
git-fetching = 正在获取…
git-show-all-commits = 显示所有提交
git-show-branch-tips = 显示分支端点
git-search-commits = 搜索提交
git-close-search = 关闭搜索
git-search-placeholder = 搜索
git-commit-count = { $count } 次提交
git-comparison-ahead = 领先 { $count }
git-comparison-behind = 落后 { $count }
git-comparison-tooltip = 相对 { $base }：领先 { $ahead }，落后 { $behind }
git-load-more = 加载更多
git-loading = 正在加载…
git-retry = 重试
git-no-repository = 未选择仓库
git-loading-history = 正在加载历史…
git-no-matching-commits = 没有匹配的提交
git-no-branch-tips = 没有找到分支端点
git-fetch-failed = 获取失败：{ $error }
git-column-author = 作者
git-column-date = 日期
git-column-sha = SHA
git-column-commit = 提交
git-columns = 列
git-column-name = 名称
git-column-reset = 重置
git-branch-expand = 展开 { $label }
git-branch-expand-hidden = 展开 { $label }（隐藏 { $count } 项）
git-branch-collapse = 折叠 { $label }

# 设置 → 已归档会话。
archive-unarchive = 取消归档
archive-unarchiving = 正在取消归档…
archive-unarchive-failed = 取消归档失败：{ $reason }
archive-untitled-session = 未命名会话
archive-nothing = 没有已归档的会话
archive-nothing-detail = 右键点击侧栏中的会话即可将其归档。
archive-previous-aria = 上一页已归档会话
archive-next-aria = 下一页已归档会话
archive-previous = 上一页
archive-next = 下一页
archive-page-range = 第 { $start }–{ $end } 项，共 { $total } 项
archive-title = 已归档会话
archive-subtitle = 在恢复前不会显示在侧栏中。

# 拉取请求徽章（侧栏与输入框）。
pr-state-open = 已开启
pr-state-merged = 已合并
pr-state-closed = 已关闭
pr-badge-label = PR #{ $number } · { $state }
pr-open-aria = 打开拉取请求 { $number }：{ $title }

# 拉取请求列表（pull_requests.rs）。
pr-list-group-needs-attention = 需要处理
pr-list-group-awaiting-review = 等待审查
pr-list-group-approved = 已批准
pr-list-group-drafts = 草稿
pr-list-star = 星标拉取请求
pr-list-unstar = 取消星标
pr-list-search-placeholder = 搜索拉取请求
pr-list-error-detect-repository = 无法检测此项目的仓库。请选择一个仓库以继续。
pr-list-error-enter-repository = 请输入 owner/name 形式的仓库。
pr-list-error-connect-device = 连接到设备以打开其项目仓库。
pr-list-error-device-offline = 此设备已离线。重新连接以打开该项目。
pr-list-error-unreadable-repository = 设备返回了无法读取的仓库。请重试。
pr-list-error-no-remote = 此项目没有 GitHub 远程仓库。请输入 owner/repository 以直接打开。
pr-list-error-read-repository = 无法读取此项目的仓库。请检查设备连接后重试。
pr-list-error-detect-timeout = 仓库检测超时。请重试或输入 owner/repository。
pr-list-detecting-repository = 正在检测仓库…
pr-list-choose-repository = 选择仓库
pr-list-this-device = 此设备
pr-list-repository-aria = 仓库：{ $label }
pr-list-projects = 项目
pr-list-finding-repository = 正在查找仓库…
pr-list-open-project-aria = 在 { $device } 上打开 { $name } 仓库：{ $path }
pr-list-open-project-aria-offline = 在 { $device } 上打开 { $name } 仓库：{ $path }（离线）
pr-list-recent-repositories = 最近的仓库
pr-list-open-recent-aria = 打开 { $repo }
pr-list-open-aria = 打开仓库
pr-list-open = 打开
pr-list-sort-updated-desc = 最近更新
pr-list-sort-updated-asc = 最久未更新
pr-list-sort-opened-desc = 最新创建
pr-list-sort-opened-asc = 最早创建
pr-list-sort-changes-desc = 变更最多
pr-list-sort-changes-asc = 变更最少
pr-list-sort-aria = 排序拉取请求：{ $label }
pr-list-loading-more = 正在加载更多…
pr-list-load-more-count = 再加载 { $count } 个
pr-list-load-more = 加载更多
pr-list-desktop-device-aria = 桌面设备：{ $label }
pr-list-desktop-devices = 桌面设备
pr-list-device-offline-aria = { $name }（离线）
pr-list-offline = 离线
pr-list-empty-choose-repository-title = 选择一个仓库
pr-list-empty-choose-repository-body = 未找到此项目的 GitHub 仓库。请选择一个以加载你打开的拉取请求。
pr-list-empty-idle-title = 准备就绪
pr-list-empty-idle-body = 刷新以在所选设备上加载此仓库。
pr-list-empty-authored-title = 没有你创建的拉取请求
pr-list-empty-authored-body = 你在此仓库中打开的拉取请求将显示在这里。
pr-list-empty-reviewing-title = 没有待审查请求
pr-list-empty-reviewing-body = 等待你审查的拉取请求将显示在这里。
pr-list-empty-all-title = 没有打开的拉取请求
pr-list-empty-all-body = 此仓库没有打开的拉取请求。新的拉取请求将显示在这里。
pr-list-show-all = 显示全部拉取请求
pr-list-load-pull-requests = 加载拉取请求
pr-list-showing-last-results = 显示上次加载的结果。
pr-list-loaded-just-now = 刚刚加载 · 刷新拉取请求
pr-list-loaded-minutes-ago = { $count } 分钟前加载 · 刷新拉取请求
pr-list-refresh-pull-requests = 刷新拉取请求
pr-list-title = 拉取请求
pr-list-loading-pull-requests = 正在加载拉取请求
pr-list-refresh-aria = 从 GitHub 获取最新结果
pr-list-filter-aria = 要显示的拉取请求
pr-list-filter-authored = 我创建的
pr-list-filter-reviewing = 待我审查
pr-list-filter-all = 全部
pr-list-clear-search = 清除搜索
pr-list-needs-you = 需要你处理
pr-list-starred = 已星标
pr-list-shown-of-total = { $shown } / { $total } 个拉取请求
pr-list-nothing-needs-attention = 没有需要你处理的内容
pr-list-no-starred = 没有已星标的拉取请求
pr-list-no-matching = 没有匹配的拉取请求
pr-list-count = { $count } 个拉取请求
pr-list-row-aria = { $title }：{ $repo } #{ $number }。{ $status }。打开拉取请求
pr-list-date-opened = 创建于
pr-list-date-updated = 更新于
pr-list-date-tooltip = { $label } { $relative } · { $date } { $time } UTC
pr-list-opened-when = 创建于 { $when }
pr-list-status-draft = 草稿
pr-list-status-open = 打开
pr-list-status-approved = 已批准
pr-list-unknown-author = 未知作者
pr-list-ci-commit-unavailable = 不可用
pr-list-ci-no-action = 无建议操作
pr-list-ci-commit-line = { $state } · 提交：{ $commit }
pr-list-ci-next-line = 下一步：{ $action }
pr-list-diff-aria = { $additions } 处新增，{ $deletions } 处删除
pr-list-error-cli-title = 此设备上没有 GitHub CLI
pr-list-error-cli-body = 安装 gh 并登录以查看你的拉取请求。
pr-list-error-auth-title = 在此设备上登录 GitHub
pr-list-error-auth-body = 运行 gh auth login，然后刷新此页面。
pr-list-error-rate-title = 已达到 GitHub 速率限制
pr-list-error-rate-body = 请求已暂停 15 分钟。已加载的结果仍可查看。
pr-list-error-network-title = 无法加载拉取请求
pr-list-error-network-body = 请检查仓库名称和你的网络连接，然后重试。
pr-list-error-offline-title = { $name } 已离线
pr-list-error-offline-body = 重新连接设备后重试。
pr-list-error-update-title = 更新 { $name } 上的 RCode
pr-list-error-update-body = 此设备尚不支持拉取请求。
pr-list-compact-now = 刚刚
pr-list-compact-minutes = { $count } 分钟前
pr-list-compact-hours = { $count } 小时前
pr-list-compact-days = { $count } 天前
pr-list-compact-weeks = { $count } 周前
pr-list-compact-months = { $count } 个月前

# 拉取请求详情（pull_request_detail.rs）。
pr-detail-title-aria = 拉取请求 { $number }：{ $title }
pr-detail-refresh = 刷新拉取请求
pr-detail-copy-link = 复制链接
pr-detail-link-copied = 已复制链接
pr-detail-open-browser = 在默认浏览器中打开
pr-detail-tab-summary = 概览
pr-detail-tab-code = 代码
pr-detail-tab-activity = 活动
pr-detail-sections = 拉取请求板块
pr-detail-search-placeholder = 查找改动的文件…
pr-detail-comment-placeholder = 写评论… 输入 @ 提及某人
pr-detail-loaded = 加载于 { $age }
pr-detail-field-status = 状态
pr-detail-field-review = 评审
pr-detail-no-review-decision = 暂无评审结论
pr-detail-status-draft = 草稿
pr-detail-status-open = 已开启
pr-detail-status-merged = 已合并
pr-detail-status-closed = 已关闭
pr-detail-status-approved = 已批准
pr-detail-status-passed = 已通过
pr-detail-status-changes-requested = 请求更改
pr-detail-status-awaiting-review = 等待评审
pr-detail-status-not-reported = 未报告
pr-detail-file-count = { $count } 个文件
pr-detail-checks-label = 检查
pr-detail-checks-aria = 检查，{ $count }，{ $status }
pr-detail-open-check = 打开 { $name }
pr-detail-current-assessment = 当前评估
pr-detail-no-blockers = 未报告阻塞项
pr-detail-assessment-incomplete = 评估不完整
pr-detail-next = 下一步：{ $action }
pr-detail-commit-tooltip-empty = 当前提交不可用
pr-detail-commit-unavailable = 提交不可用
pr-detail-commit = 提交 { $sha }
pr-detail-no-description = 暂无描述。
pr-detail-retry-diff = 重试差异
pr-detail-deleted-account = 已删除的账号
pr-detail-resolved = 已解决
pr-detail-unresolved = 未解决
pr-detail-outdated = 已过期
pr-detail-reply = 回复
pr-detail-open-comment = 在 GitHub 上打开评审评论
pr-detail-activity-unavailable = 行内评审讨论串不可用。请刷新，或在 GitHub 上打开此拉取请求进行查看。
pr-detail-no-activity = 暂无活动
pr-detail-no-conversation = 暂无对话
pr-detail-activity-empty-body = 评论和评审将显示在这里。在下方开始对话。
pr-detail-image-load-failed = 无法加载图片。点击在浏览器中打开 · 按 Escape 关闭
pr-detail-image-loading = 正在加载图片… · 按 Escape 取消
pr-detail-ci-unavailable = CI 不可用
pr-detail-ci-no-checks = 未报告检查
pr-detail-ci-empty-metadata = CI 元数据不可用。请刷新重试。
pr-detail-ci-empty-no-checks = 未报告检查。
pr-detail-ci-empty-open = 检查详情不可用。请在 GitHub 上打开此拉取请求。
pr-detail-error-gh-cli-missing = 请在此设备上安装 GitHub CLI (gh) 并登录。
pr-detail-error-auth = 请在此设备上运行 gh auth login，然后刷新。
pr-detail-error-rate-limited = 已达到 GitHub 的速率限制。请在 15 分钟后重试。
pr-detail-error-timeout = GitHub 未及时响应。请重试。
pr-detail-error-decode = GitHub 的响应过大或无法读取。请在 GitHub 上打开。
pr-detail-error-update-device = 请更新所选设备上的 RCode。
pr-detail-error-generic = 请检查链接和网络连接，然后重试。
pr-detail-error-load-failed = 无法加载此拉取请求。{ $reason }
pr-detail-error-unreadable-detail = 设备返回的拉取请求此版本无法读取。请更新所选设备上的 RCode。
pr-detail-error-connect-detail = 连接到你的设备以加载此拉取请求。
pr-detail-error-load-diff-failed = 无法加载差异。{ $reason }
pr-detail-error-invalid-diff = 设备返回了无效的差异。
pr-detail-error-connect-diff = 连接到你的设备以加载差异。

# 拉取请求详情骨架占位（pull_request_skeleton.rs）。
pr-skel-status = 状态
pr-skel-review = 评审
pr-skel-current-assessment = 当前评估
pr-skel-checks = 检查
pr-skel-handoff-title = 移交给智能体
pr-skel-handoff-body = 打开一个新会话，提示词已就绪可发送。
pr-skel-verification = 验证
pr-skel-checkout = 检出
pr-skel-loading-list = 正在加载拉取请求
pr-skel-loading-detail = 正在加载拉取请求
pr-skel-loading-diff = 正在加载差异

# 拉取请求媒体（pull_request_media.rs）。
pr-media-image-failed = { $alt } · 无法加载图片。在浏览器中打开 ↗
pr-media-image-loading = 正在加载图片…
pr-media-image-unavailable = 此处无法显示图片。请在浏览器中打开此拉取请求进行查看。
pr-media-open-image-aria = 打开图片：{ $alt }
pr-media-load-image-aria = 从 { $host } 加载图片：{ $alt }
pr-media-hosted = { $alt } · 托管于 { $host } · 点击加载

# 拉取请求代码面板（pull_request_code.rs）。
pr-code-choose-file = 选择变更的文件
pr-code-collapse-all = 折叠所有文件
pr-code-copy-diff = 复制差异
pr-code-copy-path = 复制路径
pr-code-expand-all = 展开所有文件
pr-code-file-collapsed = { $path }，已折叠
pr-code-file-expanded = { $path }，已展开
pr-code-file-position = 第 { $position } 个文件，共 { $total } 个
pr-code-files-title = 文件
pr-code-loading = 正在加载变更…
pr-code-next-file = 下一个文件
pr-code-no-changed-files = 没有变更的文件
pr-code-no-matching-files = 没有匹配的文件
pr-code-open-diff-aria = 打开 { $path } 的差异
pr-code-previous-file = 上一个文件
pr-code-show-split = 显示并排差异
pr-code-show-unified = 显示统一差异
pr-code-tree-label = 变更的文件

# 拉取请求评论交互（pull_request_interactions.rs）。
pr-interact-comment-too-long = 评论过长（最多 60,000 字节）。
pr-interact-connect-to-send = 连接到拉取请求所在设备以发送评论。
pr-interact-no-mentions = 没有匹配的参与者。你可以输入任意 @用户名。
pr-interact-post-unconfirmed = 无法确认你的评论是否已发布。{ $reason } 草稿已保留；再次发送前请先在 GitHub 上确认。
pr-interact-post-unread = 评论已发布，但无法读取其回复。刷新以查看。
pr-interact-posting = 正在发布评论
pr-interact-pull-request-image = 拉取请求图片
pr-interact-send = 发送评论
pr-interact-sending = 正在发送评论

# 拉取请求移交外围文案（pull_request_handoff.rs）。发送给智能体的提示词不是资源 ——
# 那是模型输入，不是界面文案。
pr-handoff-action-address-feedback = 处理反馈
pr-handoff-action-fix-checks = 修复失败的检查
pr-handoff-action-resolve-conflicts = 解决冲突
pr-handoff-action-review = 审查
pr-handoff-add-project = 添加项目
pr-handoff-add-project-aria = 在此设备上添加项目
pr-handoff-check-projects-failed = 无法检查此设备的项目。请检查其连接后重试。
pr-handoff-check-timeout = 检查项目超时。请检查设备连接后重试。
pr-handoff-checkout = 检出
pr-handoff-connect = 连接到拉取请求所在设备以开始会话。
pr-handoff-copy-checkout = 复制检出命令
pr-handoff-finding-checkout = 正在查找检出…
pr-handoff-no-checkout = 此设备上没有 { $repo } 的检出。请添加其项目后重试。
pr-handoff-platform-linux = Linux
pr-handoff-platform-macos = macOS
pr-handoff-platform-relevant = 相关平台
pr-handoff-platform-windows = Windows
pr-handoff-start-session-aria = 在新会话中{ $action }
pr-handoff-subtitle = 打开一个新会话，提示已准备就绪，可直接发送。
pr-handoff-summary-checks = 复现并修复失败的检查
pr-handoff-summary-conflicts = 解决合并冲突并验证合并后的改动
pr-handoff-summary-feedback = 完成审查者要求的修改
pr-handoff-summary-review = 查找缺陷、回归和缺失的测试
pr-handoff-title = 交给智能体处理
pr-handoff-verification = 验证
