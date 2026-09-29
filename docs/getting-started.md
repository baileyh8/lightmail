# 开始使用

## 安装与体验

从 [Releases](https://github.com/baileyh8/lightmail/releases) 获取对应平台的预览包：

- **macOS 15+ / Apple Silicon**：解压后将 `轻邮.app` 放入 Applications。
- **Windows 10 1809+ / Windows 11 x64**：运行 `windows-x64-setup.exe`，或解压便携 ZIP 后运行 `Lightmail.exe`。需要 Microsoft Edge WebView2 Runtime；安装包尚未代码签名。

首次启动可选择「体验示例」；示例与真实邮箱使用独立数据库，不会发送邮件。macOS 也可以在「设置 → 存储 → 打开示例预览」进入演示。

当前包未通过 Apple Developer ID 公证，macOS 可能阻止首次打开。请核对来源与校验和，或直接从源码构建；不要关闭 Gatekeeper。构建命令见主 README。

## 连接邮箱

打开「设置 → 邮箱账号 → 添加邮箱」。每个 Gmail、163、QQ 或 IMAP 账号独立保存和同步。

| 类型 | 配置 |
|---|---|
| Gmail OAuth | 使用自己的 Google OAuth **Desktop app** 客户端；在「Google 登录配置」填 Client ID，客户端提供 Secret 时一并填写 |
| Gmail 应用专用密码 | 仅适用于账号允许该登录方式的情况 |
| 163 / QQ | 先在邮箱网页开启 IMAP / SMTP，使用生成的客户端授权码，而非网页登录密码 |
| 其他 IMAP | 填写服务商提供的 IMAP / SMTP 主机、端口和凭证 |

源码与下载包不附带共享 Google Client ID。Google 配置流程和桌面回环回调要求以 [Google 官方文档](https://developers.google.com/identity/protocols/oauth2/native-app) 为准；应用目前通过 IMAP/SMTP 访问 Gmail，所需权限为 `https://mail.google.com/`。OAuth 发布、测试用户和验证状态由你自己的 Cloud 项目管理，开源发布不代表已通过 Google 验证。

浏览器授权完成、账号保存成功后，设置自动关闭并切至该账号收件箱。同步问题会按账号显示，不需要反复重新添加账号。

QQ 邮箱的授权码获取步骤、填写示例与服务器参数见 [README：QQ 邮箱怎么配置](../README.md#qq-邮箱怎么配置)。

## 全文翻译

「设置 → 翻译」支持以下引擎：

- **Chat Completions 兼容服务**：填写 Base URL、Model、API Key，测试连接后保存。支持术语表、流式响应和可选 JSON / JSON Schema 模式。第三方端点兼容程度及翻译质量取决于其实现与模型。
- **macOS 系统翻译（仅 Mac）**：按需下载系统语言包。「设置 → 翻译 → macOS 离线翻译」及阅读菜单中的语言包入口均可重新打开。Windows 使用 Chat Completions 兼容服务。

点击「全文翻译」后可切换原文、译文、双语。复制按钮按当前模式输出 Markdown；macOS 菜单也可指定模式。LLM 只收到当前主题与正文，不会自动上传整箱邮件或附件。

## 阅读与存储

每个账号各保留最新 20 封正文；新邮件进入后，排在 20 以后的正文和相应译文缓存释放。旧邮件摘要保留，打开时重新读取。搜索范围为已同步摘要与已缓存正文。

HTML 邮件默认保留样式，但不下载远程图片。图片链接仍提供文字入口；需要图片时点击正文上方「显示图片」。

macOS 数据位于 `~/Library/Application Support/Lightmail/`，Windows 位于 `%LOCALAPPDATA%\Bailey\Lightmail\data\`，其中 `Mail/` 为真实邮箱，`Preview/` 为演示数据。密码和 token 分别存入 macOS Keychain 或 Windows Credential Manager。邮件数据库没有额外应用层加密。

Windows 会读取系统静态 HTTP / SOCKS 代理及绕过列表，目前不执行 PAC 脚本。应用界面采用与 Mac 一致的布局和字号层级；系统窗口边框和字体光栅化由各平台负责。

## 常见问题

**为何重新编译后又要求钥匙串授权？** 本地 ad-hoc 签名可能随构建改变身份。只在系统弹窗中输入 macOS 登录密码；不要把密码发给维护者。拥有稳定签名证书的开发者可设置 `LIGHTMAIL_SIGNING_IDENTITY` 构建。

**列表有邮件，正文还没加载？** 摘要先同步，最新 20 封正文在后台补齐；历史正文需要联网读取。检查账号同步状态和网络，避免重复创建账号。

**规则在哪里设置？** 轻邮沿用邮箱服务端规则执行后的文件夹与标签结果；首版不编辑规则，也不导入 Apple Mail / Outlook 的本地规则。

**卸载会清除邮件吗？** 将应用移至废纸篓会保留本地数据与 Keychain 项目。彻底清理前应先保存需要的本地草稿和导入邮件。删除轻邮中的账号只清本地数据与账号凭证，不删除服务端邮件。
