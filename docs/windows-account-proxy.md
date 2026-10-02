# Windows 单个邮箱的代理配置

日期：2026-10-02；分支 `feat/windows-account-proxy`。保留 GPUI Kit + Blitz 主界面，无 WebView；凭证继续使用 Windows Credential Manager。

## 使用方式

添加或编辑邮箱时，在「此邮箱的连接代理」选择：

| 模式 | 行为 |
| --- | --- |
| 跟随系统 | 默认值，应用连接使用平台提供的系统代理；Google 网页授权使用默认浏览器 |
| 不使用代理 | 此邮箱的应用请求直连；Google 授权在独立 Edge/Chrome 窗口中使用直连参数 |
| HTTP 代理 | 填写代理主机和端口，收发和 Google 授权使用该 HTTP 代理 |
| SOCKS5 代理 | 填写代理主机和端口，收发和 Google 授权使用该 SOCKS5 代理 |

主机不包含协议、端口、路径或用户名/密码。例如填写 `127.0.0.1` 与 `7897`。可为 Gmail 指定代理，并为 QQ、163 或其他邮箱选择不使用代理；连接同一服务器的两个账号也互不覆盖。不修改 Windows 全局代理或其他浏览器配置。

本轮支持无认证的固定 HTTP / SOCKS5 代理。跟随系统仍不支持 PAC；选择明确的代理或直连时不查询 PAC。代理失败会报告错误，不自动改成直连。

## 覆盖范围

- IMAP 同步、正文、附件、标记操作、IDLE，以及 SMTP 与已发送副本。
- Google 授权码换令牌、身份检查、令牌刷新。
- 主动允许的邮件外部图片。图片仍默认禁止；换邮件或保存账号时淘汰旧图片队列和结果。
- Google 网页授权：直连或指定代理时，用独立 Edge/Chrome 进程及临时配置目录，传入对应代理参数。跟随系统时维持默认浏览器行为。

OAuth 回调是浏览器访问此设备的 `127.0.0.1` 监听器，保持直连，不交给远端代理。独立浏览器的代理绕过列表仅包含本机地址；账号明确指定的代理没有直连回退。

独立浏览器需要可用且未被代理/配置目录策略锁定的 Edge 或 Chrome。策略会覆盖命令行时，报告无法使用该独立配置；可选择跟随系统或应用专用密码。支持标准安装路径和 Windows App Paths 注册。授权结束或失败后终止本次进程树并清理本次配置目录，不关闭用户已有的浏览器窗口。

LLM 翻译服务和手动打开邮件超链接保持各自现有网络行为；它们不作为邮箱协议连接的一部分。

## 核心实现与上游边界

`AccountProxySettings`、验证、账号配置持久化和请求路由在共享 Rust 核心。配置存于 SQLite `settings` 的 `account-proxy:<account_id>`，与账号记录在同一事务保存，移除账号时删除。旧账号没有配置时默认跟随系统；旧 `save_account` 接口保存账号时保留已有代理配置。

移除 `credential()` 写入按主机共享路由的做法。传输层建立连接时读取当前账号设置；系统模式查询 `PlatformServices`。IMAP 连接复用键包含实际代理路由与认证模式，代理变化不会把旧连接归类为新路由。HTTP 请求使用账号范围的平台适配器；新建账号的 Google 授权可在保存账号前使用本次填写的配置。

Windows 只负责表单、系统代理、凭据管理器和外部浏览器。新增接口目前为 Rust-only：`MailEngine::account_proxy` / `set_account_proxy`、`MailApplication::save_account_with_proxy` / `account_platform`。未改 Swift、生成绑定或现有 UniFFI 签名。系统模式从取凭证时更新改为连接时查询，也会改善 Mac 现有系统代理的生效时机；Mac 界面与默认选项不变。

## 验证

- 核心 61 项回归通过，4 项按原约定忽略；新增端点验证、IPv6、账号隔离、重启恢复、事务保存、错误配置拒绝连接等检查。
- Windows 29 项回归通过；安装浏览器测试默认忽略，本机单独运行通过。
- 本地 TLS 协议集在 direct / SOCKS5 / HTTP 三种配置下通过，包含 IMAP、附件、标记、IDLE、SMTP 成功/拒绝/结果未知、撤销与重复发送保护；新增相同服务器上的混合代理账号、已发送副本及切换直连验证。
- 本地 Google OAuth 模拟服务器验证授权码、身份检查、并行账号刷新使用不同路由；本机回调未经过代理。
- 本机实际安装浏览器在隐藏的 headless 模式加载本地合成页面，HTTP / SOCKS5 / 直连三组路由和直连回调均通过，结束后检查临时配置目录已删除。未访问 Google 登录页面或使用真实账号。
- 原生窗口 44 项验收通过，另有正文像素检查，覆盖四种模式的控件点击、保存/重新打开以及其他账号不受影响；保留链接、托盘、正文选择和 DPI 验收。

证据保存于忽略的 `build/windows-account-proxy-20261002-verified/`。未运行真实邮箱发信，未安装或卸载，未推送。真实服务商授权和代理规则仍需用实际账号使用验证。

### 本机 10808 实网验证（2026-10-02）

按用户指定，使用本机 `127.0.0.1:10808` 的 HTTP 和 SOCKS5 分别访问真实 Google/Gmail 服务，每种协议 6 项，共 12 项通过：

- Gmail IMAP 993：项目实际代理连接函数、服务器证书验证及未登录欢迎消息。
- Gmail SMTP 465 / 587：项目实际 SMTP 隧道和 Lettre，分别完成隐式 TLS / STARTTLS 与 NOOP；不配置凭据，不发送邮件。
- Google 公开发现文档：HTTP 200 且授权端点正确。
- Google 令牌端点：明确无效的 grant 类型返回预期 HTTP 400；身份端点无凭据返回预期 HTTP 401。这证明服务可达，不代表完成 OAuth 登录或刷新。

HTTP 请求通过已持久化的诊断账号和 `MailApplication::account_platform`，同时将基础系统路由设为不可用，确认明确的账号配置覆盖系统路由；独立直连策略仍返回直连。没有读取真实邮箱数据库、登录信息或更改用户账号。

报告保存在 `build/windows-live-proxy-10808-20261002-verified/` 的 `result.json`、`mail-tls.json` 和 `google-http.json`。可用 `scripts/check-live-account-proxy.ps1 -ProxyPort 10808` 重复测试，默认新建带时间的报告目录，失败返回非零状态。诊断默认忽略，日常测试不会访问外网；本轮只修改诊断和文档，已提供的 `88e930a` 测试包无需替换。

依据：[Google 桌面 OAuth 与本机回调](https://developers.google.com/identity/protocols/oauth2/native-app)、[Edge 代理参数](https://learn.microsoft.com/en-us/deployedge/edge-learnmore-cmdline-options-proxy-settings)、[Chromium 代理说明](https://chromium.googlesource.com/chromium/src/+/main/net/docs/proxy.md)、[Edge 代理策略](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-policies/ProxySettings)、[配置目录策略](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-policies/UserDataDir)、[Windows Job Objects](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)。
