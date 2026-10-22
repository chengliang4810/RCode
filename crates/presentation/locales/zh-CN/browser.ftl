browser-title = 浏览器

browser-starting = 正在启动浏览器…
browser-cache-unavailable = 无法找到浏览器缓存目录。
browser-startup-failed = 无法启动浏览器：{ $reason }。请检查 WebKitGTK 4.1 运行时与显示环境。
browser-startup-invalid = 浏览器返回了无效的启动响应，请重新打开此标签页。
browser-startup-stopped = 浏览器初始化意外中止，请重新打开此标签页。
browser-profile-invalid = 浏览器配置需要使用绝对路径的存储目录。
browser-profile-unwritable = 无法创建或写入浏览器配置目录。
browser-profile-locked = 浏览器配置正被占用或无法锁定，请关闭其他 RCode 实例后重新打开此标签页。
browser-window-required = 浏览器需要原生 Windows 窗口。
browser-main-thread-required = 浏览器必须在主线程中创建。
browser-parent-missing = 浏览器所在窗口已不可用。
browser-temporary-logins = 此 macOS 版本上的登录仅临时保留。请使用 macOS 14 或更高版本，或在默认浏览器中打开此页面以保留登录。
browser-back = 后退
browser-forward = 前进
browser-reload = 刷新
browser-url-placeholder = 网址或 localhost:3000

# 地址栏与工具栏。
browser-go-to-address = 前往该地址
browser-open-external = 用默认浏览器打开
browser-opens-externally = 将在默认浏览器中打开

# 地址校验。显示在地址栏下方；地址处理的其余部分（协议检查、链接拒绝）
# 不产生可读文案。
browser-address-required = 请输入网址或 localhost 地址。
browser-address-invalid = 此地址包含无效字符。
browser-address-not-valid = 请输入有效的网址或 localhost 地址。
browser-address-scheme = 仅支持 http 和 https 地址。
browser-address-credentials = 请使用不含用户名和密码的地址。

# 空白标签页：预览列表与初始状态。
browser-running-locally = 本地运行中
browser-open = 打开
browser-open-preview = 打开 { $name } 预览
browser-finding-servers = 正在查找开发服务器…
browser-no-servers = 在此项目中启动开发服务器，它会自动出现在这里，随时可打开。
browser-connecting = 正在连接预览发现服务…
browser-enter-address = 输入地址
browser-enter-website = 输入网站地址
browser-or-enter-website = 或输入网站地址
browser-empty-title = 预览你的成果
browser-empty-external-title = 已在浏览器中打开
browser-empty-error-title = 无法加载此页面
browser-empty-body = 预览你的本地应用，或把网站放在对话旁边。
browser-empty-external-body = 用默认浏览器打开网站或本地应用。内置浏览已在 macOS 和 Linux 上提供。
browser-retry-page = 重试加载页面
browser-remote-loopback = localhost 只在本机打开。要访问另一台设备，请从新标签页打开检测到的预览。

# Linux 上页面的右键菜单：辅助进程发送动作标识，外壳负责绘制行。网页自带的
# 选项文案不在此处——那是站点自己的文字，原样显示。
browser-menu-open-link = 在新标签页打开链接
browser-menu-copy-link = 复制链接地址
browser-menu-paste = 粘贴
browser-menu-select-all = 全选

# 失败状态。原因原样显示（来自原生宿主自身的消息）。
browser-open-failed = 无法打开此页面：{ $reason }
browser-error-file = 无法在此预览此文件。请用默认浏览器打开。
browser-error-connect = 请检查地址并确认服务器正在运行，然后重试。
browser-error-interrupted = 连接已中断。请重新加载此页面。
browser-error-unresponsive = 页面未响应。请重新加载以继续。
browser-error-stopped = 浏览器已停止。请重新加载以继续。
browser-error-webview = 在这里显示页面需要 Microsoft Edge WebView2 运行时。
browser-error-helper = 浏览器辅助进程已停止。请确认已安装 WebKitGTK 4.1，然后重新打开标签页。
browser-error-webkit = 无法启动 WebKitGTK：{ $reason }。请为你的发行版安装 WebKitGTK 4.1 运行时。
