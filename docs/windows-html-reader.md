# Windows 邮件 HTML 阅读器：能力与后续方案

核对日期：2026-09-30；分支 `feat/windows-gpui-kit`；实际依赖 `gpui-kit =0.7.0`、`gpui-base =0.7.0`。继续坚持 GPUI Kit，不引入 WebView / WebView2。

## 明确结论

**Kit 有轻量 HTML 组件：`gpui_kit::base::TextView::html`，当前项目已经使用。但它不是完整 HTML/CSS 排版引擎，不能承担复杂邮件的原样排版。** 图标、托盘是缺失的功能；复杂 HTML 排版则是上一轮对渲染组件能力的判断不足。

之前原文阅读默认 `plain_reading = true`，把核心生成的 Markdown 交给 TextView，HTML 分支藏在「更多」里。这解释了为什么邮件看起来没有按 HTML 渲染。本轮默认改用 HTML（无 HTML 时仍回退 Markdown），菜单明确称为「简化 HTML / 清爽阅读」。**这只修复默认阅读路径，不代表复杂布局已解决。** 清爽阅读、译文、双语对照仍适合使用 TextView。

## 核对源码得到的限制

以锁定版本的 [HTML parser](https://docs.rs/crate/gpui-base/0.7.0/source/src/text/format/html.rs) 为准，而不是把官方「支持 HTML」理解为支持浏览器排版：

- 标题、段落、粗体、斜体、链接、图片、列表及简单表格会转成原生文本节点。
- `parse_table_cell` 把单元格内容压成 `Paragraph`，其子项经 `parse_paragraph` 合并；嵌套表格和块布局无法保留完整结构，单元格内段落与换行也可能丢失。
- `TableCell` 只保留内容与宽度；此解析路径不处理 `colspan` / `rowspan`。
- `style_attrs` 主要服务图片/单元格尺寸及 mark 背景等少量属性；没有通用 CSS 级联、样式表选择器和媒体查询布局。`TextViewStyle` 是控件统一样式，不是邮件 CSS 解释器。
- 核心 `clean_html` 保留了安全的表格和 CSS；Windows 阅读器无法消费这些样式，并非 MIME 层把所有 HTML 都删掉了。

官方文档也将其定位为 [Markdown 与简单 HTML 的 TextView](https://github.com/longbridge/gpui-kit/blob/main/website/component/text-view.md)。之前验收只检查正文字符串、图片策略和可见像素，能发现空白阅读区，却无法证明真实邮件的布局正确；因此不能用那些测试来宣称 HTML 保真度达标。

## 无 WebView 的选项

| 方案 | 适用范围 | 接入代价与结论 |
| --- | --- | --- |
| Kit TextView | 内容阅读、Markdown、简单 HTML | 已接入。继续用于清爽阅读和翻译；不再对复杂 HTML 作原样显示承诺 |
| litehtml + GPUI 绘制适配 | HTML/CSS 文档，包括邮件常用表格与样式 | 建议下一步优先做验证。不是 Kit 内置组件，需要字体测量、绘制、图片、链接命中和选择的适配；不能只换一个构造函数 |
| Blitz | 独立、模块化 HTML/CSS 引擎 | Rust 路线候选，需要评估与 GPUI 的绘图、字体、事件系统集成；本轮未接入，也没有证明其资源占用更适合本项目 |

[litehtml 官方说明](https://github.com/litehtml/litehtml)明确其职责是 HTML/CSS 解析与布局，绘制由调用方实现 `document_container`；它并不声称完整兼容浏览器。已有 [Rust bindings 候选](https://github.com/franzos/litehtml-rs)，但本项目还没有验证其 Windows 构建、维护状态及邮件兼容性，不能当成可直接上线的依赖。[Blitz 官方仓库](https://github.com/DioxusLabs/blitz)同样是独立引擎，不是 Kit 的内置 HTML 阅读组件。

建议保留外层 GPUI Kit 和现有 `MailApplication`，仅对「原文 HTML」阅读区验证独立的 litehtml 适配。字体、绘制、选择、滚动属于阅读器；MIME、HTML 清理、图片下载权限、缓存和翻译继续留在共享 Rust 层。不要在桌面层再写一份邮件业务逻辑，也不建议不断给 TextView 填补 CSS 子集来逐渐重写排版引擎。

## 下一轮可验收的验证范围

1. 用合成邮件覆盖嵌套布局表格、收据数据表、跨行跨列、单元格段落/换行、内联样式、样式表类选择器、长链接、中文、图片链接和窄窗口。不得提交真实邮件。
2. 先验证 litehtml 的 Windows MSVC 构建和上述布局，再实现 GPUI 字体度量与绘图、垂直/水平滚动、选区复制、链接命中；不能把整封邮件栅格化成一张无限长图片作为完成标准。
3. 所有资源获取经过现有权限和下载器。默认零远程请求；点击「显示图片」只对本封生效；本地文件、外部 CSS、脚本及未授权资源保持禁用。独立引擎不会自动替应用完成这些约束。
4. 对合成样本检查布局坐标和可见内容，补截图与人工比对；分别验证 100%/150%/200% DPI、缩放窗口和长邮件。原先的像素非空检查继续保留，但只作为最低检查。
5. 记录可执行文件增量、首屏耗时、切换延迟及连续阅读内存，再决定是否替换简化 HTML 分支。当前没有性能数据，不能先承诺它一定更小或足够快。

本轮没有引入新的 HTML 引擎；复杂邮件布局仍是明确的未解决项。

后续实编、合成样本和用户授权的本机邮件验证见 [原生 HTML 排版调研](windows-html-renderer-research.md)。Blitz 的同样本布局验证已经完成，结果优于 litehtml；下一步验证 AnyRender 到 GPUI 的绘制适配。当前 Kit 对部分真实 HTML 会越界崩溃；调研结果不等同于产品阅读器已替换。
