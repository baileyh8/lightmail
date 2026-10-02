# Windows Blitz 阅读器迭代与验证

日期：2026-10-02。默认路线保持 **Rust 核心 + GPUI Kit + Blitz**。本轮完善正文阅读适配，没有改 Mac 实现、邮箱服务编排或凭证方案；Windows 继续使用 Credential Manager。

## 已实现

- 阅读器使用实际阅读区宽度和 DPI，不再固定 720px。调整窗口时去抖重排，复用当前文档、字体布局上下文与已加载图片；固定宽表格保留横向滚动。
- 一个文档线程只保留当前一封邮件，布局、选区和重排请求均有界，过期结果不能覆盖当前邮件。DOM 不跨线程传递，GPUI 接收拥有所有权的图像与选区几何。
- 支持拖动选择、正反方向中文选择、跨段落复制、全选与复制动作；段落边界保留为换行。选区高亮由 GPUI 叠加，不因拖动重新编码整封图片。链接按行片段命中，拖动链接文字不打开网址。
- 原始 Blitz 区间遍历会把尾部匿名段落按其 DOM 父节点排序，导致跨表格全选为空。本轮用实际布局根的顺序组织区间；字符命中与高亮几何仍来自 Blitz/Parley。匿名端点通过父节点、位置和文本指纹校验，重排后不复制到错误内容；新增合成回归，未提交真实邮件复现。
- 图片默认禁止；逐封允许后通过共享 Rust 下载器获取。独立图片队列先显示正文，图片到达后重排并更新完成的选区。切信丢弃旧图片结果，改变宽度和 DPI 不重复下载。
- 图片来源由 HTML 解析器识别，支持属性空白、大小写和实体编码；只接受匹配的栅格格式。限制为每张 8 MiB、每封缓存 32 MiB 压缩字节、64 个来源，以及每封累计 16,777,216 个解码像素。像素预算按 RGBA8 折算约 64 MiB，不是整个应用或解码瞬态内存的上限。
- 绘图面最多 16,777,216 个像素，尺寸和高度在分配前校验。超限或排版失败回退共享核心准备的 Markdown，不留下空白阅读区；下一封仍可打开。
- 修复托盘唤醒：在窗口自己的 UI 线程同步显示窗口，再请求前台焦点。独立验收中发现的隐藏窗口未恢复问题已复测。
- 补齐此前 vendored `serde_fmt` 源码缺失的 MIT / Apache 许可证。

## 验证记录

| 检查 | 结果与范围 |
| --- | --- |
| Windows 单元与行为回归 | 24 项通过，涵盖 Unicode/DPI、匿名段落与表格、慢图片、缓存复用、过期结果、绘图及图片预算 |
| 共享 Rust 核心 | 56 项通过、4 项按原约定忽略；本轮不修改核心源码 |
| 真实缓存邮件 | 20 封 × 360 / 600 / 720px × 100% / 150% / 200%，共 180 组实际产品 worker 绘图、PNG 解码和全选几何检查通过 |
| 横向内容 | 360 / 600px 下同一封固定宽表格邮件需横向滚动；720px 下无溢出。没有通过裁掉内容来通过检查 |
| 本机截图 | 输出 20 张 720px / 100% 整封截图；真实邮件和截图只在忽略的 `build/` 保存 |
| 原生窗口 | 26 项自动验收，加可见像素检查；包括图标、托盘、恢复窗口、实际阅读区宽度、鼠标中文选择、复制动作、拖动链接不导航、设置与草稿 |
| 连续阅读 | 300 次合成邮件阅读后静置 30 秒；工作集峰值 104.12 MiB，私有提交内存从前五分之一处 137.76 MiB 到末次 142.41 MiB，增长 4.65 MiB，静置后 141.61 MiB |

上面的应用内存是整套 GPUI 应用的测量，不能与此前独立阅读器的 7–8 MiB 相混。专用 GPU 显存没有计入；300 次合成数据也不能证明真实大邮箱或长期运行稳定。实际 GPU 窗口本轮仍为本机 100% 缩放，其他 DPI 的结果来自产品阅读 worker；没有修改用户显示设置进行多屏验收。

原生鼠标输入投递到隔离的测试窗口，复制通过焦点路由的 GPUI 动作与 Windows 剪贴板验证；没有以此声称物理键盘、第三方输入法或 Narrator 已验收。真实邮件矩阵默认禁图；允许图片的下载、重排、切信与缓存复用使用本地合成 HTTP 服务器验证，不证明所有真实 CDN/CID 图片都兼容。

复现命令：

```powershell
$env:CARGO_TARGET_DIR = 'target/kit-dev'
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --locked --release -p lightmail-desktop --features acceptance --bin Lightmail --example blitz_mail_probe
./scripts/check-windows.ps1 -SkipInstaller -Executable target/kit-dev/release/Lightmail.exe -OutputDirectory build/windows-blitz-native-new-run -SoakReads 300
target/kit-dev/release/examples/blitz_mail_probe.exe build/html-reader-private-20260930 build/windows-blitz-real-private-new-run
```

探针只读取此前已提取的 HTML，不打开账号库、读取凭证或执行收发；可加最后一个参数 `6` 等匿名序号单独重测。构建需要 Rust/MSVC 与已配置的 Windows SDK。已有安装时使用 `-SkipInstaller`，本轮没有安装或卸载用户的软件。

## 后续边界

本记录对应基础阅读交互阶段。后续已补有界视口绘制、长邮件尾部阅读与 UI Automation 正文读取，见 [视口与可访问性迭代](windows-blitz-viewport-accessibility.md)。多屏 DPI、完整键盘及 Narrator 体验仍需验收；现有结果不等于完整 CSS 规范覆盖或全部邮件逐像素保真。
