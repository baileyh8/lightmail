# Windows 正文阅读器：WebView2 与当前 Blitz 的本机对照

日期：2026-10-01。结论：避免整套 Electron 网页界面的初衷合理；直接调用 WebView2 仍有明显浏览器开销，不能把它当成与原生阅读器相同的资源成本。当前 Blitz 在本机这组输入上更省内存。采用 WebView2 的理由应当是浏览器排版与交互能力、减少自行维护阅读器的工作，而不是声称它比 Blitz 轻。

## Electron、Tauri 与直接 WebView2 的区别

Electron 使用 Chromium 多进程模型，主进程在 Node.js 环境运行；保留 GPUI 窗口、控件与 Rust 服务，只嵌入正文阅读控件，可以避免引入 Electron 的整套应用结构。具体节省多少必须测同样功能的应用，本轮没有测 Electron。[Electron 官方进程模型](https://www.electronjs.org/docs/latest/tutorial/process-model)

Tauri 在 Windows 上本来就使用 WebView2。直接调用 COM API 能控制生命周期、内容和宿主集成，但不会替换浏览器内核，也不会自动消除浏览器、渲染、GPU 与辅助进程。不能承诺仅去掉 Tauri 框架便显著减少这些内存。[Tauri 官方说明](https://v2.tauri.app/reference/webview-versions/)、[微软进程模型](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/process-model)

共享 Evergreen Runtime 主要避免每个安装包携带一份浏览器运行时；磁盘共享不等于多个应用运行时免费共享全部 RAM。[微软运行时分发说明](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution)

## 测试方法与结果

本机 Windows 11 专业版，Build 26300，i5-14600KF、32 GiB RAM。实际 WebView2 Runtime 为 `154.0.4258.37`。Blitz 固定 `ed03fe183a9f129965ead6435c0caa8e9c49c401`，Vello CPU `0.17.0`。测试为独立 release EXE 的两种运行模式，不是整个 Lightmail 应用的 A/B 测试。

两种模式使用同一个最小 Win32 宿主、720×640 像素、100% 缩放、浅色主题。输入为此前经用户授权提取的本机缓存 HTML，按文件名排序取前 20 封；不打开账号数据库、不读取凭证、不收发邮件。页面脚本和外部图片、CSS、字体、子框架均阻止。每种模式交替顺序运行三轮，共 **120 次真实正文加载**；每轮单独进程，WebView2 使用新数据目录，并在该轮复用一个控件。

下表是三轮范围。私有内存指进程树的 **private committed bytes**，不是物理驻留 RAM；两者都包括宿主。工作集另列，因为共享页可能重复计数。全部数字都不包含 GPUI 窗口、邮箱服务、完整应用或独立 GPU 显存。

| 检查点 | 当前 Blitz 管线 | 一个复用的 WebView2 控件 |
| --- | ---: | ---: |
| 读完 20 封后的私有内存 | 6.77–8.29 MiB | 184.82–186.59 MiB |
| 静置 3 秒后的私有内存 | 6.77–8.29 MiB | 185.93–189.29 MiB |
| 观测到的私有内存峰值 | 33.09–33.22 MiB | 196.06–208.95 MiB |
| 隐藏 3 秒后的私有内存 | 5.00–6.52 MiB，释放位图 | 163.50–164.98 MiB，挂起成功 |
| 释放阅读器及其环境 3 秒后 | 5.07–6.52 MiB | 4.07–4.08 MiB |
| 阅读后的工作集总和 | 21.76–22.34 MiB | 332.76–338.11 MiB |
| 阅读后的进程数，含宿主 | 1 | 7 |

进程树每 100ms 采样，Blitz 额外在绘图、PNG 编码、解码缓冲区存活时观测宿主内存。峰值仍可能漏掉更短瞬态，是观测下限，不是上限或资源承诺。最后三轮均验证释放后只剩测试宿主；没有终止其他应用。

| 操作耗时 | Blitz | WebView2 |
| --- | ---: | ---: |
| 阅读器初始化与首个合成文档 | 8.11–9.88ms | 252.08–643.23ms |
| 后续 60 次真实正文加载中位数 | 24.29ms | 66.75ms |
| 后续加载 P95，nearest rank | 68.97ms | 116.61ms |

**耗时结束点不同，不能据此给用户可见首帧速度排名。** Blitz 包含 HTML 解析、三次 resolve、整封 CPU 绘图、与产品相同设置的 PNG 编码、图像解码和 GDI 绘制；WebView2 从 `NavigateToString` 到 `NavigationCompleted`，不等于屏幕已完成呈现。没有清空 OS 文件缓存，初始化也不是冷启动数据。Blitz 的生产实现通过 GPUI 上传图像，本探针没有这部分成本；也没有测 WebView2 与 GPUI 同时运行的 GPU 和窗口集成成本。

另外各运行一次视觉检查，每次也读 20 封，对第 1 / 20 封保存首屏图。WebView2 用 `CapturePreview`，Blitz 保存实际生成的首屏像素。截图验证有实际正文，没有只凭导航成功判断显示正常；不构成 20 封逐像素保真验收。初次检查发现 WebView2 跟随系统深色主题造成一封邮件黑底黑字，最终对照已统一浅色主题并重跑。图片被阻止时，两种引擎的占位/替代文本也不同，不能称完全相同排版。

## 对选型与后续工作的影响

若首要目标是最低常驻资源，当前原生方向有真实收益。不能把已完成的 CSS、图片权限、后台绘图等调研称为全部浪费。但当前整封 PNG 只是过渡方案：选区/复制、窄窗口与 DPI 重排、可访问性、长邮件分块绘制以及引擎兼容缺口仍需要工程投入，不能用约 2.6ms 的 `resolve` 时间代表成品阅读器性能。

若产品需要浏览器级的 HTML/CSS 排版和常见阅读交互，建议保留 **Rust 核心 + GPUI Kit 应用界面 + 单个按需 WebView2 正文控件**。这个建议接受阅读时约百余 MiB 的额外浏览器私有内存，以减少自建排版与交互适配的工作；没有证据支持“直接 WebView2 与原生一样省”或“已经比 Tauri 更快”。仍需正式应用内的增量验收。

采用这条路线时，迭代应按以下顺序推进：

1. 首次打开 HTML 正文才初始化控件，整个列表、设置、写信和托盘继续由 GPUI 与 Rust 承担；一个控件反复切信，不按邮件创建环境。普通纯文本/Markdown 使用现有原生视图。
2. 默认阻止外部资源和脚本，正文继续经过 Rust 的 MIME/清理管线。图片获准后复用 Rust 下载策略；正文控件不持有邮箱凭证或业务服务。
3. 先完成 GPUI 宿主的 HWND、焦点、键盘、链接、弹窗遮挡与 DPI 验收，再启用产品默认路径。复杂 HTML 不再通过自行扩写 GPUI 排版来追求完整 CSS。
4. 托盘隐藏先挂起，达到闲置阈值后关闭最后一个正文控件并释放阅读器环境引用，邮箱核心服务继续工作；恢复时按需重建。阈值需要测重开体验，不能直接把隐藏视为已释放。
5. 对正式应用复测默认禁图/允许图片、连续切信、缩放、长邮件、选区复制与托盘释放。资源统计纳入整个进程树，不能只报 `Lightmail.exe`。单独记录重新初始化延迟与 GPUI 基线，避免把探针数字当成最终应用数字。

微软建议对不可见 WebView2 尝试挂起以减少工作，但释放效果是尽力而为。本机三轮全部挂起成功，仍保留约 164 MiB 私有内存；关闭并释放环境后才降到约 4 MiB。因此“托盘只保留原生 UI 与核心，空闲后释放正文引擎”是该方案能否满足低常驻要求的关键验收项。[微软性能建议](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/performance)

本轮只新增独立对照工具和文档；没有将 WebView2 加入产品、没有替换当前 Blitz 阅读器、没有修改 Mac 或共享业务实现，也没有推送。复现方式见 [探针说明](../tests/reader-performance-probe/README.md) 与 [对照脚本](../scripts/compare-windows-readers.ps1)。最终原始数字保存在忽略的 `build/reader-performance-20261001/light-final/summary.json`；正文、截图及运行时目录不提交。
