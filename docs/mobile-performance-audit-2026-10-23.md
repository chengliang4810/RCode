# 移动端性能检查（2026-10-23）

## 范围与结论

检查 `apps/mobile/lib` 的应用入口、全部页面、共用组件、状态/RPC/缓存路径，以及 Android/iOS 剪贴板桥接。Rust 图形内核按 Flutter 的调用边界检查，本次没有重新全量审查或重编译 Rust 内核。

这轮不仅涉及 Debug 包开销。确认了应用级重建、长列表构造、新建页面布局、隐藏原生图形活动、目录缓存 JSON、字符壁纸重复字形布局六类可避免的工作，已分别处理。没有把静态发现的重负载候选都认定为此次手机掉帧的原因，也不声称已经消除全部掉帧。

## 已处理

| 路径 | 触发与证据 | 处理 |
| --- | --- | --- |
| `lib/main.dart`、`lib/theme.dart` | 根 `ListenableBuilder` 订阅所有工作区通知，目录/会话变化会重新构造 `MaterialApp`、两套主题及 seed 调色板 | 主题复用；只有主机、鉴权、语言、主题或模型对象身份改变时重建应用外壳。页面仍正常接收数据更新 |
| `lib/ui/new_session_page.dart` | `IntrinsicHeight` 与 flex spacer 对可滚动输入表单引入固有尺寸测量；路由转场开始时立即 autofocus | 用约束与对齐保留原 3:4 垂直分布，避免额外固有测量；路由进入完成后聚焦，回调再次核对状态 |
| `lib/ui/sessions_page.dart` | `ListView(children: ...)` 每次更新为所有会话创建行组件，包括不可见行 | 保留分组、排序、拖放、折叠语义；使用 sliver builder 按需构造行 |
| `lib/visual/rcode_visual.dart` | 可视区域判断只看几何位置和前后台，未判断被另一条路由覆盖；隐藏时仍以约 16 ms 轮询 | 结合路由可见性与 TickerMode detach 原生图形；隐藏轮询降为 250 ms，恢复可见后回到 16 ms。只影响显示，不影响服务端任务 |
| `lib/data/workspace_controller.dart` | 工作区目录缓存仍在 UI isolate 调用 `jsonEncode/jsonDecode`，与已后台编码的正文缓存不同 | 后台 isolate 编解码，异步返回后再次检查主机 generation/销毁状态，保持原串行写入次序 |
| `lib/wallpaper.dart` | ASCII 效果为每个网格点创建、layout、释放一个 TextPainter，实际字形至多十种 | 一次绘制内复用相同字形的布局；重复点绘制复用 Paint，绘制结束统一释放 |
| `lib/ui/transcript_markdown.dart` | 应用外壳减少重建后，数学字体/大小/开关仍必须明确使 Markdown 内部已解析树失效 | 以数学设置作为解析树 key，保留设置即时更新 |

## 逐页与共用路径核对

| 页面或路径 | 核对结果 |
| --- | --- |
| 连接页 `connection_page.dart` | 网络调用异步、错误局部更新；仍有 IntrinsicHeight，列入下面的候选，不把它误算成已修复 |
| 会话列表/搜索/归档/分组 `sessions_page.dart` | 修复行构造；搜索评分/分组排序仍在 UI isolate，千条目录基准仍存在超过 16 ms 的更新 |
| 新建会话 `new_session_page.dart` | 修复布局与聚焦；设备/模型/项目/分支选择、重试、草稿与手机/平板切换由现有测试回归。模型目录请求为异步，等待网络本身不能直接当作掉帧证据 |
| 会话详情 `session_page.dart` | 正文已为 `ListView.builder`，已存在稳定正文行复用、滚动跟随与草稿回归；长正文/代码解析、图片和图形调用见候选 |
| 设置 `settings_page.dart` | 设置行数量有限；设置持久化异步；核对主题、语言、数学、通知、壁纸设置。没有擅自删除设置或视觉效果 |
| 项目目录 `project_browser_page.dart` | 异步加载目录，UI 仍一次构造返回的目录项；大量子目录是独立的后续重负载场景 |
| 队列/队列编辑 `queue_panel.dart` | 队列列表使用 builder，编辑页状态局部更新，租约/提交/重试有回归；附件 chip 数量随输入增加 |
| 结构化问答 `question_panel.dart` | 当前问题局部状态，选项按当前题构造；大量选项/长题干仍需相应输入压力样本 |
| 消息/工具/子会话/图片预览 `message_view.dart`、`message_text.dart` | 图片请求 Future 保存在 State；子会话列表使用 builder；仍存在大图全尺寸解码、长工具输出等重负载候选 |
| Markdown/代码/数学 `transcript_markdown.dart`、`visual/native_math.dart` | 核对解析库实际更新条件，保留数学配置变化；同步高亮、FFI 数学首次渲染仍需复杂输入测量 |
| 差异 `diff_view.dart` | diff 已通过 `compute` 放到后台，列表为 builder，差异搜索有边界；未再引入额外改动 |
| 附件/剪贴板 `attachments.dart`、`image_clipboard.dart` | 大图规范化已经使用 `compute`，读取限制 24 MiB；仅明确操作时读取剪贴板 |
| 原生图形 `visual/rcode_visual.dart`、`visual_native.dart`、`visual_backend.dart`、`visual_native_stub.dart` | 修复隐藏路由行为；核对异步打开、generation、图片 revision、租约释放和有界空闲池；首次字体注册/大量帧复制仍需独立样本 |
| 壁纸 `wallpaper.dart` | 解码宽度已有 256 上限，重绘已有 RepaintBoundary；修复 ASCII 重复布局。没有移除效果 |
| RPC/会话/缓存 `data/rpc_client.dart`、`session_controller.dart`、`session_cache.dart`、`workspace_controller.dart` | 重连订阅读请求与变更命令分离；正文缓存已有后台编解码、debounce、串行写入；此次补目录缓存后台处理。JSON 接收与 transcript delta 校验仍在 UI isolate |
| 通知/偏好/主题/语言 `notifications.dart`、`preferences.dart`、`theme.dart`、`l10n.dart` | 主题复用；草稿写入不广播普通偏好通知；通知在连接运行期间工作；未发现剪贴板后台探测或页面打开时自动图片读取 |
| Android `MainActivity.kt` | URI 内容读取已有后台线程，结果回到主线程；未发现图片流读取直接堵住 Android 主线程 |
| iOS `AppDelegate.swift` | PNG 不存在时的 UIImage 缩放、PNG 编码仍位于方法处理回调；列为 iOS 待验证候选，不能用 Android 真机得出 iOS 性能结论 |

## 真机对照

设备：Xiaomi 23113RKC6C、Android 16/API 36、arm64、SM8550/Adreno。两组均为 Profile/AOT，同一工具链、相同字体和 CI 原生库。旧 Dart 页面取自 `8f10b01`，通过独立源码快照编译；新组使用本轮页面修复。测试入口不含真实供应商请求。

使用 Flutter `SchedulerBinding.addTimingsCallback` 记录 FrameTiming。六次打开新建页/返回列表，随后八次更新约 1000 个本地会话目录。表中是该次样本，不是 FPS 保证；totalSpan 包含流水线排队，不能直接当作单独 UI 或 GPU 执行时间。

| 指标 | 旧 Profile | 新 Profile |
| --- | ---: | ---: |
| 新建/返回，UI P90 | 10.298 ms | 10.301 ms |
| 新建/返回，UI 最大值 | 25.593 ms | 16.365 ms |
| 新建/返回，totalSpan P99 | 28.425 ms | 20.089 ms |
| 千条目录更新，UI P90 | 185.097 ms | 36.361 ms |

新建转场 P90 基本不变，尾部有改善，不能据此宣称所有切页已经流畅。千条目录仍未稳定落在 16 ms 内；分组/搜索投影、可见列表布局和目录广播应继续按更大数据量分解测量。

原始证据保存在 ignored `build/qa/physical-device`：`frames-navigation-old.json`、`frames-navigation-new.json`、`navigation-old.apk`、`navigation-new.apk`、对应构建日志。旧任务的 Debug/Profile 对照单独保留在 `performance-verification.json`，不能把 Debug 到 AOT 的收益混入本轮同模式对照。

## 仍需压力样本验证的候选

这些有具体代码路径，但尚未确认是用户这次三个普通会话切换时的原因：

1. 大量会话时，分组、搜索、排序和 JSON 帧解析仍为 UI isolate 工作。目录缓存编解码已移出；RPC 接收不能直接改为无序后台解析，否则会破坏订阅帧顺序和重连 generation。
2. 很长的 Markdown、代码高亮、复杂 LaTeX 的解析/FFI 调用可能占用 UI isolate。`Future` 返回并不意味着内部同步 FFI 已在后台运行。当前数学组件保留渲染 Future，但不等于全局缓存或后台数学工作线程。
3. 消息缩略图的 `Image.memory` 未按显示尺寸限制解码；高分辨率远程图片、多个附件可能造成解码和内存压力。全屏缩放需要保留清晰原图，不能简单对全部图片一刀切缩小。
4. 连接表单固有尺寸测量、项目目录一次构造、长模型选择菜单/问答选项需要相应大输入或小屏键盘场景，尚未逐一做帧统计。
5. 图形第一次初始化会注册字体，很多复杂图形的帧/像素数据仍在 Dart UI isolate 复制、解码。隐藏轮询已修复；它不等于消除可见场景内核的计算成本。
6. iOS 剪贴板回调中的缩放/编码需要 macOS/iPhone 验证；本次没有 iPhone 真机或新的 iOS 性能运行。

先前尝试的 IndexedStack→PageView 方案没有测出键盘收益，已撤回；底部背景模糊的普通页面 raster P90 较低，未把删模糊当作已证实修复。上述候选需要单独输入和 trace，不能以统一关闭动画或删除功能代替定位。

## 验证边界

本轮完整 Flutter 测试 91 项通过、5 项环境门控跳过；目录缓存后台化另跑 20 项相关回归通过。Windows 测试使用仓库已有的受版本限制的独立 Flutter 工具副本规避宿主 profiler 问题，安装的 SDK 和测试断言没有更改。新增回归覆盖工作区通知不重建 MaterialApp、语言/主题仍更新、进入路由后聚焦、被遮挡原生图形 detach/降低轮询/恢复。

本机静态检查与 Android Profile 构建通过。真机帧统计覆盖新建往返、目录更新和之前的滚动/键盘场景，不代表全部图片、壁纸、问答、工具输出、原生脚本、所有机型都完成压力验证。最终手机安装的是真实 `lib/main.dart` 入口的 RCode QA，测试快照/本地假数据入口仅在 ignored QA 文件中，恢复连接原 VPS；普通 RCode 及其数据保留。

恢复 VPS 后人工打开原会话、新建页和设置页，确认目录与连接正常，未发现异常或布局溢出日志。新建页另显示“此设备尚未配置模型”，这是本次实际观察到的服务器配置状态，不能把它描述成已经验证了真实供应商新对话。

代码提交：`7730492`（应用与主题）、`ced67b2`（惰性会话行）、`58745a5`（新建页布局/聚焦）、`291fb48`（隐藏图形）、`9cd50bd`（数学设置失效）、`5b19157`（目录缓存）、`393deb3`（字符壁纸）。
