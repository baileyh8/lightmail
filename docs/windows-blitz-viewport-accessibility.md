# Windows 正文视口绘制与可访问文本

日期：2026-10-02。分支 `fix/windows-blitz-viewport-accessibility`，继续 GPUI Kit + Blitz，Mac 与共享邮箱业务保持原样。

## 本轮变更

- 完整文档保留布局，绘图缓冲区改为当前可见区域及最多 512 个逻辑像素的预绘区域。按 256px 边界推进，滚动不再为整封正文分配 PNG/RGBA；更长正文可以继续以 HTML 阅读。
- 图片面仍有 16,777,216 像素预算；超限优先减少预绘区域，不裁掉可见视口。极端文档布局仍有 2,000,000px 的安全上限。预算约束绘图面，不代表整体内存上限。
- 文档坐标与绘图区域坐标分离，选区、链接及复制跨区域保留；滚动只绘图，不重排文档或复制整份可访问文本。布局改变时再更新文字几何。固定定位内容使用当前精确视口，避免被预绘区域移动。
- 补充 PageUp / PageDown 与 Ctrl+Home / Ctrl+End 的阅读动作及快捷键绑定，支持跳到长正文尾部再返回首屏。
- GPUI 阅读区暴露 Document / ScrollView 角色和稳定文本节点，通过 AccessKit 向 Windows UI Automation 提供 TextPattern；包含当前屏幕外的正文。字段和坐标来自已完成的 Blitz 布局，不创建第二套排版。
- 真实邮件整封截图通过有界区域流式写入 PNG，不要求应用持有整封图片；原始邮件和截图只保存于忽略的 `build/`。

## 验证

- Windows 25 项行为回归通过；共享 Rust 核心 56 项通过、4 项按原约定忽略。
- 新增长邮件回归：20,000px 内容加 640px 尾部，在 100% / 150% / 200% DPI 下校验首屏和尾部实际像素颜色、绘图面大小、文档坐标选区，以及滚动时复用可访问文本快照。
- 本机原生窗口验收验证跳至 20,640px 正文尾部、全选复制首尾中文、返回首屏。独立 UI Automation 客户端限定到测试进程的窗口，通过 TextPattern 读取屏幕外首部和尾部；输出只含合成数据计数。
- 20 封此前授权提取的真实缓存邮件，三种宽度及三种 DPI，共 180 组产品 worker 绘图、解码与选择检查通过；另导出 20 张完整 PNG。默认禁图，不执行账号服务或读取凭证。
- 最终原生验收 30 项通过，另检查可见像素。300 次合成阅读中整应用工作集峰值 93.67 MiB，私有提交内存从前五分之一处 128.57 MiB 到末次 133.46 MiB，增长 4.89 MiB，静置 30 秒后 133.07 MiB。该结果包含合成窗口与 UI Automation，不证明真实大邮箱长期稳定，专用 GPU 显存未计入。
- 真实邮件矩阵中的最大绘图面为 3,240,000 个像素（RGBA8 约 12.36 MiB）；截图导出按区域逐行写入，绘图面不随正文总高度增长。

本记录对应正文读取阶段。后续已补选区读写接口、隔离窗口 DPI 消息切换与原生 PageDown 验证，见 [选区与 DPI 迭代记录](windows-blitz-selection-dpi.md)。物理键盘、多屏与完整 Narrator 仍需人工验证；精细字符坐标和标题/表格层级不据此标记完成。

```powershell
$env:CARGO_TARGET_DIR = 'target/kit-dev'
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --locked --release -p lightmail-desktop --features acceptance --bin Lightmail --example blitz_mail_probe
./scripts/check-windows.ps1 -SkipInstaller -Executable target/kit-dev/release/Lightmail.exe -OutputDirectory build/windows-blitz-viewport-new-run -SoakReads 300
target/kit-dev/release/examples/blitz_mail_probe.exe build/html-reader-private-20260930 build/windows-blitz-viewport-private-new-run
```

Windows 验收脚本使用系统 .NET Framework 的编译器生成独立 UI Automation 测试客户端；这是测试工具的依赖，应用不依赖 .NET。已有安装时仍使用 `-SkipInstaller`。
