# 视觉与反馈约定

此文记录 RCode 原生 GPUI 界面当前使用的阅读尺度与反馈实现。设计参考只用于取舍，不代表已经复现第三方产品的所有视觉效果。

## 阅读尺度

本轮接入的文字角色使用 `typography::UiTextRole` 公共入口，数值以用户界面字号设置为 16px 时为基准：

| 角色 | 字号 | 行高 | 用途 |
| --- | ---: | ---: | --- |
| Body | 15 | 25.5 | 对话正文 |
| Primary | 14 | 20 | 侧栏条目、详情主要内容 |
| Secondary | 12 | 18 | 次要内容 |
| Caption | 11 | 16 | 时间、数量、辅助状态 |
| Section | 12 | 18 | 分组标签 |
| Heading | 16 | 24 | 区块标题 |

这些角色使用 rem，跟随已有的界面字号设置。本轮覆盖聊天正文、主侧栏会话行、详情主信息、公共菜单及统计浮层，未把全项目的局部字号全部迁移为角色。主文字的字号接近，正文通过行距和段间距表达阅读层次。代码、Diff、终端继续使用各自既有字体和字号配置；不得为了视觉一致而修改等宽字符的测量结果。

例如界面字号设置为 14px 时，正文实际为 13.125px，侧栏与详情主要内容为 12.25px，比例为 15:14；时间和数量仍采用 Caption。代码字号设置仍按各模块既有基线联动，而非所有代码表面使用相同像素：设置 12.5px 时，Markdown 代码块为 12.5px、Diff 为 12px、文件编辑器为 13px、普通文件预览为 11.5px。此轮未修改这些历史比例。终端使用独立的终端字号设置。

字体仍由 `typography::font` 和已有用户字体选择解析。Windows 的中文回退依次包含内置 Sarasa UI SC、Microsoft YaHei UI、Microsoft YaHei、Noto Sans CJK SC；系统最后回退仍由平台处理。此轮没有替换内置字体、强制系统字体优先或重排用户的西文字体选择。

侧栏行高与拖拽占位使用同一计算入口，缩小字号时仍为固定尺寸状态图标保留空间。避免文字变大但拖拽区域、选中背景仍采用旧行高。

## 几何与边界

`Theme` 集中提供控件高度与常用圆角。Composer 使用 22px 外圆角，停靠到会话时过渡到 18px；公共菜单卡片使用 12px 圆角和 4px 内容内距，菜单行圆角由外框、1px 边框和内距推导为 7px。未全量迁移其他面板、按钮的局部尺寸。

边界仍由现有单一容器绘制。没有统一把 1px 改成 0.5px：物理像素对齐需要在 Windows 分数 DPI 下单独验证，不能仅按 CSS 示例套用。

## 公共反馈

- 复用公共菜单导航的选择器区分当前选项与 Hover／方向键定位项；保留焦点环、方向键、Enter、Escape 和关闭期间的命中遮挡。
- 用量与上下文统计浮层使用 350ms 打开意图等待、180ms 离开宽限；这两项不是动画时长。公共菜单进入／退出动画仍为 140ms／100ms，侧栏会话 Hover 卡片保留自身的等待规则。
- 模型与思考强度触发器补齐键盘可见焦点；重试按钮复用公共样式。
- 队列使用实际 pending、移除与结束状态表达忙碌。失败回复进入现有错误反馈，不能一直停留在忙碌状态。
- 文件编辑器只在实际 `DocumentPhase::Saving` 时显示保存中；成功、失败、修改冲突与版本保护仍由原状态机决定。

## 局部动效

`motion::PropertySpring` 提供与 GPUI 有界动画进度分开的属性弹簧。支持位置和速度连续的中途改目标、欠阻尼／临界／过阻尼、阈值停帧；不把超出 0..1 的弹簧值送入 GPUI 的 easing 通道。

主 Composer 的发送／停止按钮整体绘制圆和图标，按压时两者一起缩放，命中区域保持 28px。键盘与无障碍激活复用 GPUI 原生 click 路径，避免重复发送。

`motion::Presence` 管理稳定身份的进入／退出。输入区引用 Chip 只对背景与单色图标做错峰反馈，不改变文字布局、光标、选区或命中区域；编辑前缀不会让已有 Chip 重新进入。

附件删除立即更新业务集合。短暂保留的退出视图不可发送、不可预览、不可再次操作；清理任务独立于绘制执行。切换会话、切换队列草稿或清空时清除旧动画，避免图片资源和视图残留。

使用 `ScrollHandle` 的滚动列表，羽化带依据实际隐藏距离连续增长至上限，没有溢出时不遮淡内容。只提供溢出布尔值的既有虚拟列表仍按溢出状态启停羽化；公共组件另提供真实距离回调供接入。密集骨架使用同步呼吸，少量文字占位使用轻量扫光；截图卡片仅在 Hover 时显示局部微光。

减少动效开启时，新增进入／退出与弹簧立即到达最终状态，骨架保持静态。动画结束后不持续申请帧。

这里的“整体缩放”已经用于自绘动作按钮的完整图形。GPUI 普通 Div 没有通用子树 transform；当前没有新增整窗缩放功能，也没有通过逐帧修改字体大小模拟缩放。现有界面字号设置与 Windows DPI 是不同机制。

## 原生文字与终端修补

Windows DirectWrite 对明确的键帽 Emoji 序列（`#`、`*`、`0..9` 后跟 VS16 与 U+20E3）使用系统已有的 Segoe UI Emoji 整段 shaping。普通数字、文本形式 VS15、中文及用户字体选择不变。原生探针确认 `1️⃣` 修补前分属两个字体，修补后是同一 glyph/cluster；`👩‍💻` 在该原生探针中修补前后都已经是同一 cluster，不能把它称为已确认的原生 ZWJ 拆分。

Windows IME 的取消与空组合通知只删除实际 marked range；没有组合区时保留普通选区。属性读取的负值错误码不会再转为超大的分配长度；正数短读只检查实际返回的属性，未读尾部的零值不再被误认作输入区。

终端快照保留模拟器中的零宽尾字符，重音与变体选择符可以进入真实绘制。宽字块光标按模拟器的一列/两列标记绘制。每个文字片段裁切在其实际 VT 槽位内，避免宽字形覆盖紧邻字符或选择区域。

终端仍遵守模拟器原有列宽：一列 `1️⃣` 的右侧可能裁切；`👩‍💻` 仍可能由两个双列单元展示。这里没有改变终端 Unicode 宽度或跨格合字策略，不能用正文 Emoji 已合成来推断终端也占两列。

## 隔离验证入口

```powershell
cargo run -p rcode-ui --locked --offline --features appshots-fixture --example visual-feedback-fixture
cargo run -p rcode-ui --locked --offline --features appshots-fixture --example visual-feedback-fixture -- --dark --reduced-motion --font-size 20
cargo run -p rcode-ui --locked --offline --features appshots-fixture --example visual-feedback-fixture -- --business --font-size 14
cargo run -p rcode-ui --locked --offline --features appshots-fixture --example text-surfaces-fixture -- --ui-font-size 14 --wide-cursor
```

测试窗口使用临时设置，加载真实公共字体、Markdown、ComposerInput、菜单、骨架、滚动羽化及动效基础设施。不连接引擎、账号、网络或真实聊天，不模拟发送成功。它是组件验证工具，不能替代完整 Shell 业务验收。

`--business` 挂载真实 Composer、Queue、FilesSurface，使用明确注入的待确认/保存失败测试状态。`text-surfaces-fixture` 挂载真实 Changes 和 TerminalPanel；终端直接接收固定 VT 测试字节，没有启动 PTY 或 shell。两个入口都不读取实际用户配置。

业务验证的 Composer 通过隔离构造入口跳过 `RCODE_ATTACH` / `RCODE_ATTACH_PREVIEW`，避免继承的开发环境变量读取外部附件；普通应用的构造行为保持原样。验证入口仅在显式开启 `appshots-fixture` 时编译。

支持 `--font-size 12|14|16|20`、`--scale 0.5..2`、`--dark`、`--reduced-motion`。`--scale` 只改变应用 rem，状态栏单独报告实际 `window.scale_factor()`；不得把它作为 Windows 100%、125%、150%、200% DPI 的验证结果。

## 运行验证边界

源码和单元测试可以确认文字角色、几何计算、状态迁移、退出资源清理和动效数学。实际运行应另外检查：

- Windows 100%、125%、150%、200% DPI 下的中文、英文、数字、全半角标点、粗体与 Emoji。
- 代码／Diff／终端的中英文字符宽度、光标落点、选中背景、横向滚动和行号对齐。
- 中文 IME 组合过程、候选框位置与取消组合后的编辑。
- 完整会话中的发送、停止、队列失败、保存成功／失败、附件跨行删除、主题与减少动效切换。
- 复杂 Emoji 的组合字形；基础 Emoji 能显示不等于 ZWJ 与 keycap 序列全部正常。

每次验收应记录构建版本、实际平台 DPI、用户字体与字号，区分组件实测与完整应用实测。
