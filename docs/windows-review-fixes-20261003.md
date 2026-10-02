# Windows Review 整改与验证

日期：2026-10-03；分支 `fix/windows-review-blockers`。对应 [整体 Review](windows-final-review-20261003.md) 的 R1–R5，保持 GPUI Kit + Blitz、Windows Credential Manager，不修改 Swift 界面或生成绑定。

## 修复结果

| 问题 | 处理 | 验证 |
| --- | --- | --- |
| R1 授权保存不能完整回滚 | Windows 使用核心的 `begin_google_login` / `finish_google_login`。授权结果暂存，账号/代理先在 SQLite 事务中验证和暂存，再写凭据；提交失败恢复原凭据。表单不提前写全局 Google 配置，新令牌包含自己的客户端配置，刷新优先使用该配置 | 延迟外键错误强制数据库 commit 失败，旧 grant 恢复；成功授权不改全局配置；两个账号刷新分别使用自己的客户端 ID/secret |
| R2 更换身份保留旧同步状态 | 核心拒绝在同一账号 ID 下更换邮箱地址、提供商、IMAP 主机或端口；Windows 编辑时身份字段只读，普通名称/代理设置继续可保存 | 地址/提供商/主机/端口变更均在写凭据之前拒绝，原账号和凭据保留；名称和代理修改成功 |
| R3 示例模式未隔离 | 核心 `new_offline` 使用独立内存凭据库，拒绝真实账号接入/授权、网络凭据请求和在线翻译，不启动后台连接。Windows 示例也使用内存平台适配器；账号明确直连或指定代理仍不能绕过离线模式 | 示例界面提交真实账号不启动任务、不改 Google 设置；核心启动及切换自动收信后无 worker；离线账号资源适配器拒绝请求 |
| R4 另一条图片路径缺少像素预算 | 两个阅读器共用图片头/格式/动画容器检查。限制单张及累计像素，GIF/APNG/WebP 按完整画布×帧数计费；TextView 的 data 与网络图片均在交给 GPUI 前检查，内嵌图片也计入数量上限 | 约 100 KB、6000×4000 PNG 被拒绝；多张合法图片超过累计预算后拒绝；GIF 多帧及损坏容器回归通过 |
| R5 授权不能取消 | 保存 GPUI 任务与请求序号，取消、Esc、关闭设置或切换账号时丢弃授权 future。独立浏览器退出通知与核心等待竞争，取消后释放本机监听；原有浏览器会话 guard 负责清理自己的进程与目录 | 取消后控件恢复、旧请求失效；核心 future 取消后原回调端口不再接受连接且 grant 不变；Win32 fixture 进程退出被检测并清理临时目录 |

旧的全局 Google 配置仅作为没有客户端元数据的旧令牌的兼容回退。Windows 新授权不更改该全局配置。Mac 保留原 `GoogleLogin::finish` 调用和 UniFFI 签名；本轮没有把 Mac 表单接入新的授权保存事务，不能把 Windows 的授权事务验收称为 Mac 表单已整改。共享核心的邮箱身份拒绝规则、刷新配置及 MIME 行为会同时作用于 Mac，Swift 代码未改。

## 内嵌图片

- MIME 清洗仅对 `img src` 开放受限 raster data URI 和 CID；超大 data、SVG、HTML、文件图片及 data/CID 超链接仍被拒绝。
- 本地/导入 MIME 按 Content-ID 解析附件；IMAP 只选择性获取 HTML 引用的 CID 部分，不下载其他普通附件。
- 单封邮件转换的 CID 资源累计不超过 2 MiB、最多 64 项；PNG/JPEG/GIF/WebP/BMP 转为本地 data URI，并继续受 Windows 阅读器的压缩字节、像素与动画预算约束。
- 仍沿用当前阅读器的图片显示开关。无匹配资源的 CID 留作不可用图片，不变成网络请求；缺失/无效图片不意味着能完整模拟浏览器。
- 旧缓存中已经被清洗掉的 CID 信息无法凭空恢复，重新取得正文后才可使用新映射。本轮没有清理真实账号缓存。

## 检查结果

- 核心 **67 项通过，5 项默认忽略**；Windows **34 项通过，1 项默认忽略**。忽略项按既有约定单独执行，非全部功能已证明稳定。
- `scripts/check.py --protocols-only` 通过：direct / SOCKS5 / HTTP 三组 TLS IMAP、附件、IDLE、SMTP 及混合账号检查；本轮增加真实协议 fixture 的 CID 选择性获取断言，普通附件保持按需。
- 单独执行安装浏览器检查，HTTP / SOCKS5 / 直连页面路由、本机回调、临时配置清理通过。
- 本机 `127.0.0.1:10808` 的 **12 项**实网无登录检查再次通过，包含 Gmail 993/465/587 和 Google 公开 OAuth 服务；未登录或真实发信。
- 原生窗口 **44 项**通过，另有正文像素检查；包含代理保存、示例资源路由不能绕过离线策略，以及此前链接确认/复制、托盘、正文选择、DPI 等。整应用工作集峰值约 **118.66 MiB**，专用 GPU 显存未计入；这不是长时间真实邮箱测试。
- 20 封此前授权提取的邮件，三种宽度×三种 DPI，共 **180 组**实际产品 worker 绘图/PNG 解码/选择检查通过，默认禁图；没有新访问真实邮箱数据库。
- `cargo fmt --all --check`、`git diff --check` 通过；Swift 与生成绑定无 diff，未推送，未安装/卸载。

证据目录为忽略的 `build/windows-review-fixes-20261003-verified/`、`build/windows-review-fixes-real-private-20261003-verified/`、`build/windows-review-fixes-live-proxy-20261003/`。真实邮件与截图不提交。

这轮关闭已复现的 Windows review 问题。正式稳定版仍需真实账号完整授权、持续收信、授权发信/附件、安装升级，以及实机多屏/输入法/Narrator 验收。
