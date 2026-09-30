# Windows 原生 HTML 排版调研与本机邮件验证

日期：2026-09-30。分支：`feat/windows-gpui-kit`。约束：保留 GPUI Kit，不采用 WebView；Mac 实现不改。

## 结论

**可以在 GPUI 内做真正的 HTML/CSS 阅读器；Kit 的 TextView 不足以承担这个任务。推荐以 litehtml 做有明确验收范围的原型，但不能声称它完整支持 CSS。**

要完整实现浏览器级 CSS，需要持续维护选择器与级联、字体与双向文本、所有布局模型、绘制及最新规范的兼容性。litehtml 官方也明确不完整兼容 HTML/CSS。[官方说明](https://github.com/litehtml/litehtml)

本机真实邮件的结果支持更换原文 HTML 的渲染路径：Kit 独立预览对 26 封中的 2 封越界崩溃；另有复杂邮件正文被压成行内/表格内容，失去原来的版式。litehtml 对全部 26 封完成了多宽度、多缩放布局与绘制，嵌套表格、字体层次、卡片和按钮的视觉结构明显更完整。不过窄阅读区仍有横向溢出，因此这次验证是**候选可行性证明，不是新阅读器已完成或所有邮件都保真**。

如果“完整 CSS”是绝对要求，litehtml 不能满足；Blitz 也不能先承诺完整兼容。应进一步用同一套样本实编、评测 Blitz/Servo 类引擎的兼容性和成本，再作决定。本轮 Blitz 只有源码与架构评估，没有 Windows 构建或同样本性能数据。

## 核对版本与构建结果

| 对象 | 本轮核对范围 |
| --- | --- |
| GPUI Kit / Base | 锁定的 `0.7.0`，底层 `gpui-pre 0.3.7`；当前应用实际版本 |
| litehtml Rust bindings | `franzos/litehtml-rs@662ed3a6cbad72be8b9d76a8fc01e13faa8258e8`；`litehtml 0.2.6` / `litehtml-sys 0.2.5` |
| litehtml C++ | 绑定所锁定的 `8836bc1bc35ca0cfd71dc0386ef841d5cbc3bd5e`；不是当日最新引擎版本 |
| Blitz | `DioxusLabs/blitz@ed03fe183a9f129965ead6435c0caa8e9c49c401`，`0.3.0-beta.2` 源码 |
| 本机 | Windows x64、Rust 1.97.0、MSVC 14.50；探针 release 静态 CRT |

候选 Rust 绑定原样在 MSVC 上不能完成构建，本轮在 `.tooling/litehtml-research` 的研究副本修复了四类问题：

1. Gumbo 没有加入官方 CMake 使用的 `visualc/include`，导致找不到 `strings.h`。MSVC 的 C 标准选项也改为 C11，避免未知的 C99 选项。
2. 缺少上游 CMake 使用的 `/permissive-`，导致 C++ `not` 关键字被当成标识符；同时补 `/utf-8`。
3. 非 macOS 一律链接 `stdc++`，不适用于 MSVC；MSVC 分支交由其工具链链接标准库。
4. C 包装器 `lh_document_add_stylesheet` 将 `const char*` 直接传给模板函数，产生未解析的模板实例；改为 `std::string`。

没有改 C++ 引擎源码。修正后，候选绑定的 61 项库单测通过，release 探针成功运行。这些修正由 [research-html-reader.ps1](../scripts/research-html-reader.ps1) 从固定上游版本重现。绑定当前 CI 只验证 Ubuntu，正式采用前应固定来源、保留 MIT/BSD/Apache 许可并增加 Windows CI，不应依赖浮动 master。[候选绑定](https://github.com/franzos/litehtml-rs)

## 合成样本：64 次运行

8 种输入，各运行 360px / 720px、100% / 150% / 200% 的四个组合；分别测试原始 HTML 和通过项目 `MailEngine::import_eml`、MIME 清理、正文缓存后的 HTML，共 64 次。全部完成，无解析错误。

| 检查 | 结果与边界 |
| --- | --- |
| 嵌套表格、两列账单、colspan / rowspan | 坐标检查通过；核心清理后仍保留 |
| 单元格内段落和 `<br>` | 分段及三行高度检查通过 |
| 样式表类选择器、背景色、媒体查询字体 | 通过；不能由此推断所有 CSS 属性都支持 |
| 窄窗口 | 固定 640px 表格只写 `max-width:100%` 仍溢出；加入媒体查询将 width 改为 100% 后通过 |
| 旧式 `cellpadding=24` | 未生效；用明确的单元格 `padding:24px` 后通过。需要有界的旧属性适配，不能把第三方邮件预处理整体复制进 UI |
| 超长无空格 URL | `overflow-wrap` / `word-break` 样本仍溢出。锁定引擎的默认分词主要按空白和 CJK；C++ 有 `split_text` 扩展点，但 Rust 绑定未暴露 |
| 中文选区、高亮矩形 | `Alpha 中文 Beta` 精确复制通过。未证明复杂 emoji、双向文本或跨块选区已满足产品要求 |
| 含转义参数的签名链接 | 命中后 URL 与预期逐字一致，没有重写 token |
| 图片和外部 CSS | 仅进入容器回调，探针不获取任何资源；清理后的 CSS `@import` 和背景图仍可能触发回调，适配层必须明确拒绝或按权限处理 |
| 400 段长邮件 | 文档约 14,452px 高；尾部可绘制，仍只分配 640px 高视口 |

合成样本示例（200% 输出；不含真实邮件）：

![合成账单：嵌套表格、跨行跨列、段落和按钮](screenshots/native-html-layout-probe.png)

源码与固定依赖在 [tests/html-reader-probe](../tests/html-reader-probe/README.md)，不进入应用工作区的依赖树。探针用 CPU Pixbuf，尚未接入 GPUI 的绘制管线。

## 用户授权的本机真实邮件验证

从当前邮件数据库以 SQLite `mode=ro` / `query_only` 读取已经缓存的 `bodies.data`。不调用 `MailEngine::new`，避免触发启动恢复；不读取凭证、账号设置或执行收发。26 封缓存正文均有独立 HTML，最大约 90 KB，最深 7 层表格。

- litehtml：26 封 × 5 组尺寸/缩放，共 **130 次**布局与绘制，全部完成，无解析错误、无空白文本绘制结果。
- 当前 Kit：26 封在 720px 的独立预览中，24 个进程正常结束，**2 个在 `gpui-base 0.7.0/src/text/inline_flow.rs:457` 越界崩溃**。两个样本单独复核也触发同一路径；崩溃根因仍需最小合成复现，未改依赖源码。正常退出不能等同于排版正确。
- 视觉核对：复杂通知邮件在 Kit 中主体的分块、对齐和标题层次丢失，部分正文显示成合并的表格/尾部内容；litehtml 保留了对应的卡片、标题、间距及按钮结构。没有与网页邮箱逐像素对照，不据此计算“保真率”。

| litehtml 视口（100%） | 有横向溢出的邮件 | 本轮解析+布局中位数 | 最大解析+布局 | 最大绘制 |
| --- | --- | --- | --- | --- |
| 360px | 9 / 26 | 21.4ms | 65.0ms | 8.5ms |
| 600px | 5 / 26 | 20.8ms | 64.5ms | 12.3ms |
| 720px | 0 / 26 | 20.9ms | 75.3ms | 13.3ms |

数字来自最后一轮本机固定视口探针，受机器、字体与缓存影响；前一轮最大布局约 187ms，故不能把表内最大值当作延迟上限。这些都是 CPU Pixbuf 数据，不能当作 GPUI 集成后的性能承诺。当前远程图片全部阻止，因此未验证图片加载后的重排或图文最终高度。

真实正文及截图仅保存在本机忽略的 `build/html-reader-private-20260930/`。私人指标报告只写尺寸、耗时、计数和匿名样本序号，不写主题、地址、正文和 URL。此文档只有聚合结论，不包含真实邮件或私密截图。

## 资源成本的已知范围

合成输入的独立探针 EXE 约 3.93 MiB，包括 litehtml、Cosmic Text、Tiny Skia、PNG 输出和报告代码；启用共享核心输入后约 8.61 MiB。**都不是应用安装包的增量**。正式 GPUI 后端预计不需要 Pixbuf/Cosmic Text，但尚未实编测量这个增量。

最后一轮真实样本进程的工作集峰值约 76.9 MiB、采样私有内存峰值约 57.4 MiB。文档最高约 5009px，绘图面仍固定高度。720×640 的 RGBA 视口约 1.76 MiB；200% 视口约 7.03 MiB。不能将这些样本推断为长期稳定性或所有邮件的资源上限。

合成 400 段长邮件的解析+布局约 41–50ms，首屏绘制约 5–8ms、尾部约 5–7ms。字体初始化单次约 18–20ms，正式实现应复用字体缓存，避免每封重建字体系统。

## GPUI 接入方式

已实编并运行 [html_font_probe.rs](../desktop/examples/html_font_probe.rs)：无需窗口，用 `WindowTextSystem` 测量 Segoe UI / 微软雅黑的中英文、组合字符及混合文本，输出 12 组字体数据。说明锁定版本公开了所需度量和字符定位 API；不等于真实的 HTML 绘制/选区已完成。

| litehtml 回调/职责 | GPUI 或现有项目接口 |
| --- | --- |
| create_font、字体宽度 | GPUI `WindowTextSystem` 的字体解析、ascent/descent/x-height、shape_line；测量和绘制用同一套系统，避免排版和显示发生偏差 |
| draw_text | 记录具有字号、颜色、基线、坐标的绘制项，由 GPUI `ShapedLine` 绘制 |
| 背景、边框、裁剪、图片 | GPUI quad/path/image 和剪裁；所有坐标用逻辑像素，DPI 转换由 GPUI 完成 |
| 滚动 | GPUI 阅读区维护偏移与视口；只绘制可见区域，固定宽邮件保留水平滚动 |
| 图片获取与尺寸变化 | 复用核心下载权限和 Windows 图片缓存，需要暴露解码后尺寸及有界重排；不能将 URL 直接交给默认加载器 |
| 链接 | litehtml 命中回调，经当前协议过滤后按原字符串打开；选区拖动不触发链接 |
| 选区与复制 | Kit 窗口选择服务协调阅读区焦点；使用与布局相同的 glyph 数据或有严格生命周期的引擎选区，重排/切信时清理 |

`Document<'a>` 是单线程对象，借用容器；不能随意加 `Send` 或用 transmute 保存在异步 ViewModel 中。建议让专用 reader worker 在自己的线程创建并持有容器/文档，通过命令维护布局、点击和资源变更，将拥有所有权的绘制结果发回 GPUI。消息、宽度、DPI 和图片权限都带代次，旧结果不能覆盖当前邮件。需要验证字体度量在此线程上的可用性以及 worker 内对象的生命周期。

MIME、HTML 清理、翻译、正文缓存、下载权限继续在共享 Rust 层。阅读器只处理排版、绘制与交互，不复制邮箱业务规则。

## Blitz 对照

Blitz 的核心由 Stylo（CSS）、Taffy（盒布局）、Parley（文本）组成，绘制通过 AnyRender；默认外壳使用 Winit，不能把整套 shell 嵌进当前 GPUI 客户端。[官方架构](https://github.com/DioxusLabs/blitz)

源码已确认有表格布局、colspan/rowspan、文本选择、AccessKit，以及默认不获取资源的 `DummyNetProvider`。可以只组合 `blitz-html` / `blitz-dom` / `blitz-paint`，不使用其 shell、网络或脚本模块；但 GPUI 的字体、GPU 命令、事件与无障碍仍要桥接。[核心依赖](https://github.com/DioxusLabs/blitz/blob/ed03fe183a9f129965ead6435c0caa8e9c49c401/packages/blitz-dom/Cargo.toml)

其当前 README 将状态定义为 beta，并明确存在缺失功能；主线还固定了 Taffy 和 Parley 的 Git 修订。因此“Rust 原生”不意味着完整兼容或能直接替换 Kit 控件。本轮没有测其 Windows 编译、包体或内存，不能断言它比 litehtml 大多少，也不能以 litehtml 的样本结果代表它。

### Blitz 的 Windows 实测

本轮把 `blitz-html + blitz-dom + blitz-paint` 接到一个独立 probe workspace，完全不依赖 `blitz-shell`、Winit、WebView 或网络。固定源码为 Blitz `ed03fe1`、`0.3.0-beta.2`；Stylo、Taffy、Parley 和 AnyRender 均在本机 Rust 1.97 / MSVC 下编译。`blitz-paint` 编译有一个上游 `dead_code` warning（`BackgroundSizeComputeMode::Intrinsic`），没有错误。

布局 probe 使用 `DummyNetProvider`、串行 style resolve、物理视口 360 / 600 / 720px 以及 100% / 150% / 200% DPI；真实缓存 HTML 共 26 封 × 5 组，共 **130 次**。结果：

| Blitz 视口（100%） | 横向溢出 | resolve 中位数 | resolve 最大值 |
| --- | --- | --- | --- |
| 360px | 1 / 26 | 2.75ms | 21.31ms |
| 600px | 1 / 26 | 2.58ms | 6.91ms |
| 720px | 0 / 26 | 2.59ms | 7.20ms |

26 封真实邮件的表格、嵌套内容、长 URL、中文文本和命中测试都完成；没有失败的 probe check。最大正文布局高度约 5076px。横向溢出的样本是同一封复杂邮件，宽度为 640px；它需要邮件规则预处理或阅读区水平滚动，不能隐藏溢出。

这比 litehtml 的本机结果更接近当前需求：litehtml 在 600px 有 5 封、360px 有 9 封溢出，布局中位数约 20.8ms；Blitz 在同样输入下分别是 1 封、1 封和约 2.6ms。这个差异来自真实运行结果，不是仅根据项目介绍推测。

**推荐路线因此调整为 Blitz DOM + Blitz Paint + GPUI 适配。** 保留 GPUI Kit 作为窗口、控件和应用框架，只接入 Blitz 的 DOM/CSS/layout 和 PaintScene。GPUI 适配器负责把 AnyRender 的 `PaintScene` 命令转成 GPUI quad、path、图片和字体绘制；`blitz-shell` 不纳入依赖。Blitz 的 DOM 已有表格、选择、hit testing 和 `DummyNetProvider`，图片、CSS、字体和链接仍由 Lightmail 的权限与平台服务控制。

当前验证仍有边界：没有把 `blitz-paint` 的 AnyRender 命令实际转换为 GPUI 绘制，也没有做真实截图逐像素比较、图片加载重排或完整复制选区。因此下一阶段是 GPUI PaintScene 适配原型，不是直接替换产品阅读器。

当前分支已经完成第一版产品接入：原文 HTML 默认通过 Blitz DOM/Paint 生成受 GPUI 托管的 PNG 阅读面；用户点击「显示外部图片」后，Blitz 的资源提供器调用现有核心 `fetch_resource`，完成图片尺寸重排。默认仍阻止网络资源，data 图片按邮件权限处理。该版本仍是第一阶段适配：绘制结果暂存为整封有界图片，选区和链接命中还没有接回 Blitz DOM，当前渲染会在 reader 更新路径同步执行；正式采用前要把 PaintScene 直接接到 GPUI、把渲染任务彻底移出 UI 状态更新，并补选区/链接。

选择建议：优先实现 Blitz 的 GPUI 绘制适配。litehtml 保留为较小的 fallback/对照方案；只有当 Blitz 的适配成本明显超过预期时才重新考虑 litehtml。仍不需要 WebView。

## 下一轮实施与退出条件

1. **构建与生命周期**：固定绑定及引擎版本，收敛四项补丁到可审查的本地适配；默认应用不引入 Pixbuf 或第二套窗体，reader worker 不跨线程传 C++ 指针。
2. **排版缺口**：把旧表格属性有界地规范化；暴露/实现长文本分词扩展，保持复制文本和 href 原样；窄阅读区同时提供水平滚动。不能用隐藏 overflow 把内容截掉作为修复。
3. **GPUI 原型**：文字、背景、边框、圆角、图片占位与固定视口先跑通，再实现选区、点击及图片重排。保持 Markdown/翻译分支可用，原文 HTML 只切换渲染器。
4. **采用前验证**：合成样本和本机授权的 26 封邮件重测；分别检查 360 / 600 / 720px、100 / 150 / 200% DPI、尾部内容、选区及图片权限。加字体/绘制缓存和连续阅读内存检查，量出真正的应用增量。
5. **更广 CSS 需求**：若关键样本仍需大规模扩写引擎，转为 Blitz 的 Windows 实编与相同样本对照；不要把 TextView 或 litehtml 逐渐扩成一个自行维护的完整浏览器引擎。

本轮没有替换产品阅读器、没有改共享核心或 Mac 实现。当前产品里已知的 Kit HTML 崩溃仍未修复，下一轮应优先避免这条路径或完成其最小复现；本轮没有因为读取本机真实邮件就把产品修复状态标为完成。
