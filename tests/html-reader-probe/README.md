# HTML 阅读器调研探针

用于 HTML 阅读器调研，不是产品阅读器。单独 Cargo workspace，不改变应用依赖。

运行 `./scripts/research-html-reader.ps1`（原始合成 HTML）和 `./scripts/research-html-reader.ps1 -CoreInput`（经过共享核心 MIME 清理）。脚本获取固定版本的 Rust bindings 与它锁定的 C++ 子模块，修正候选绑定的 MSVC 构建脚本和 C 包装器，执行布局、选区、链接与资源回调检查。报告和 PNG 放在 `build/html-reader-research/`。已知能力缺口会作为 false 写入 checks，不等于探针执行失败；解析异常则返回非零退出码。

探针使用绑定自带的 CPU Pixbuf 后端；它不等同于 GPUI 绘制适配。图像只分配固定视口，长邮件不分配整封大图；不会请求外部 CSS 或图片。字体来自本机，截图与耗时可能因机器不同而变动。

获得用户明确授权后，可用 `python scripts/sample-cached-html.py DATABASE build/...private...` 只读提取本机已经缓存的 HTML，再执行：

```powershell
./scripts/research-html-reader.ps1 -BuildOnly
./target/html-reader-probe/release/lightmail-html-reader-probe.exe --private build/...private... build/...private.../litehtml
```

私人模式不打开邮箱数据库，只读取提取出的 HTML；跳过二次 MIME 导入，指标报告不写邮件文本和 URL。PNG 含真实正文，只能留在忽略目录，不能提交或作为共享附件。

GPUI 字体 API 探针：`cargo run --locked -p lightmail-desktop --example html_font_probe -- build/html-reader-research/gpui-fonts.json`。

当前 Kit 原生 HTML 预览：`cargo run --locked -p lightmail-desktop --example html_reader_preview -- INPUT.html build/OUTPUT 720`。它打开不抢焦点的短时预览窗口，因为隐藏 D3D 窗口无法用于 PrintWindow 验证；输出 BMP 与指标 JSON，然后退出。真实正文输出前缀必须选在本机忽略的私人目录。此预览不创建 `MailEngine` / `MailApplication`，没有图片获取或导航回调。
