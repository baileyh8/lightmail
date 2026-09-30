# Blitz HTML/CSS reader probe

独立 workspace，验证 `blitz-html + blitz-dom` 的解析、Stylo CSS、Taffy 布局和 hit testing，并编译 `blitz-paint`。不依赖 `blitz-shell`、Winit、WebView 或网络。正式 GPUI 绘制仍需接 `blitz-paint` 的 AnyRender 命令到 GPUI。

运行 `./scripts/research-blitz-reader.ps1`。默认使用合成样本；`-PrivateDirectory build/...private...` 可读取已提取的本机缓存 HTML。不会访问邮箱数据库、凭证或网络。
