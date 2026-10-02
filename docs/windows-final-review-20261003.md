# Windows 分支整体 Review

整改状态：Windows R1–R5 已在 `fix/windows-review-blockers` 修复并验证，CID/data 图片补上受限支持，见 [整改记录](windows-review-fixes-20261003.md)。下文保留修复前的问题和复现结果。

完成日期：2026-10-03。审查分支 `feat/windows-account-proxy`，代码 `7425ccf`；已重新 fetch 上游，`upstream/main` 仍为 `5ab113a`。审查核心、Windows 界面/适配器、阅读器、OAuth/代理、托盘、任务和打包；未改产品实现、Swift 或生成绑定。

结论：可以保留 GPUI Kit + Blitz + Windows Credential Manager 的路线。共享 Rust 已承担同步、缓存、发件计时、代理和数据存储；主要问题集中在授权保存事务、邮箱身份边界、示例隔离及另一条图片加载路径。当前不宜标记为正式稳定版。

## 需要修复的问题

### R1 · P1 · Google 授权与账号保存不能完整回滚

入口：[desktop/src/app.rs:1052](../desktop/src/app.rs#L1052)、[core/auth.rs:185](../core/auth.rs#L185)、[core/application.rs:465](../core/application.rs#L465)。

账号表单先写全局 `google-client-id` 和凭据管理器里的 `google-client-secret`，随后才校验并等待授权。失败或取消也留下新全局配置；其他 Google 账号刷新会读取它。另一路径中，`GoogleLogin::finish` 先覆盖 `account:<id>` 的令牌，再调用账号保存；OAuth 保存把密码参数清空，因而没有旧令牌的补偿恢复。

两项定向复现：

- 原全局 Client ID 是合成的有效旧值，提交空 Client ID；登录立即失败，但旧值已变为空。
- 用本地合成 OAuth 服务完成另一身份的授权，再用非法 IMAP 主机使保存失败；账号记录保持旧身份，内存凭据库却已经变成新令牌。

这是上游已有流程在 Windows 中保留下来的问题，不应因普通账号保存具备事务而视为已解决。把验证、授权结果暂存、账号/代理保存、凭据写入及失败补偿交给核心的授权保存命令；全局 Google 配置只在明确成功提交时更新，并明确它是应用级还是账号级配置。可先提供 Rust-only 接口，保留现有 Swift 签名。

验收：取消和错误输入不改旧全局配置；成功授权后的数据库失败恢复旧令牌；已有账号仍能刷新。

### R2 · P1 · 更换邮箱身份仍保留旧同步状态与正文

入口：[core/account_proxy.rs:141](../core/account_proxy.rs#L141)、[core/transport.rs:378](../core/transport.rs#L378)。

同一账号 ID 允许修改地址、提供商、IMAP 服务器。保存只更新账号记录并失效连接池，没有清理文件夹 UIDVALIDITY、邮件 UID、正文和译文缓存。若新旧服务器的 UIDVALIDITY 相同，增量同步把旧 UID 当作当前邮箱的已有记录；UID 重叠的旧标题/正文会继续保留，低于旧最大 UID 的新邮件也不进入普通补拉。待发草稿只保存账号 ID，身份变化还会改变其实际发件身份。

定向复现：在隔离库中将旧邮件 UID 设为 500、UIDVALIDITY 设为 7，修改账号地址和 IMAP 主机并成功保存；旧 UID、有效性值及正文缓存均仍在。本轮没有声称连接真实服务器复现漏收，漏收条件由同步代码确认。

优先禁止通过普通编辑替换已有邮箱身份，提供新增账号流程；若支持迁移，则核心显式重建同步命名空间、清理对应缓存，并先处理待发/正在发送草稿。显示名称、代理等普通设置不应触发身份重建。此问题属于遗留的数据模型边界。

验收：相同 UIDVALIDITY、重叠 UID 的两个合成邮箱不会混合；待发邮件不能无提示地跟随新身份。

### R3 · P1 · 示例模式没有隔离真实凭据和网络

入口：[desktop/src/app.rs:175](../desktop/src/app.rs#L175)、[desktop/src/platform.rs:14](../desktop/src/platform.rs#L14)、[desktop/src/app.rs:1054](../desktop/src/app.rs#L1054)。

`--demo` 隔离了数据库并跳过启动时的 `service.start()`，但仍创建实际 Windows Credential Manager 适配器，使用相同服务名和全局 Google secret 键。账号编辑器没有阻止新增真实提供商；保存又会调用核心的 worker reconciliation。因此在写着「示例模式 · 不连接真实邮箱」的窗口中配置账号，仍可触发真实授权/连接，填写 Google secret 还会覆盖真实模式的同名凭据。

该项由调用链和键名确认，未尝试写真实凭据或连接真实账号。问题在上游 Windows 入口已存在，本分支未补齐隔离。

示例模式应使用内存凭据库，并明确限制真实账号操作；网络禁止应由核心运行模式约束，而不能仅依赖启动时不调用 `start()`。若允许从示例进入接入流程，应先显式切换真实模式。

验收：用受监测的假凭据/网络适配器操作完整示例设置界面，实际凭据写入和外部连接次数始终为零。

### R4 · P1 · Markdown/译文的图片路径绕过解码像素预算

入口：[desktop/src/images.rs:90](../desktop/src/images.rs#L90)、[desktop/src/images.rs:140](../desktop/src/images.rs#L140)；对照 [desktop/src/blitz_reader.rs:163](../desktop/src/blitz_reader.rs#L163)。

Blitz 路径检查单张及累计解码像素；Kit TextView 使用的 `RemoteImages` 只限制压缩字节、数量和格式，然后交给 GPUI。选择允许图片后，低压缩体积的大尺寸图片仍能进入该路径；批量图片可能使解码缓存和纹理内存远超预期。切换阅读模式不应改变资源约束。

定向复现：有效 PNG 为 6000×4000，即 2400 万像素，压缩后只有 100,942 字节。实际 resolver 返回 `ImageSource::Image`，超过 Blitz 的 16,777,216 像素预算。仅读取尺寸和创建编码图像对象，没有实际分配 96 MB RGBA 或尝试制造 OOM；内存影响依据 GPUI 解码路径确认。

把尺寸验证与累计像素预算复用到两条加载路径，在交给 GPUI 前拒绝超限输入；同时覆盖动画总帧预算。

验收：原文、Markdown、译文、双语及 data URI/网络图片均执行同一预算，大尺寸高压缩比图片不得进入渲染缓存。

### R5 · P2 · 关闭授权窗口/面板不能取消登录任务

入口：[desktop/src/app.rs:1072](../desktop/src/app.rs#L1072)、[desktop/src/oauth_browser.rs:25](../desktop/src/oauth_browser.rs#L25)、[core/auth.rs:150](../core/auth.rs#L150)。

授权 future 被 detach 后没有保存取消句柄。用户只关闭独立浏览器或设置面板，不会终止等待本机回调；最多仍等待 180 秒，期间 `busy` 保持为 true，不能立即重试。浏览器的清理 guard 也等该 future 结束才释放。这里指关闭窗口，没有否认 Google 返回拒绝授权回调时已有的错误处理。

保存授权任务/会话句柄，提供取消动作，并把独立浏览器关闭事件与 OAuth future 做竞争处理；取消后清理本机监听、临时配置和 busy 状态。该项按源码确认，未耗时等待完整 180 秒。

验收：关闭或取消后短时间恢复可操作状态，没有持续监听或残留授权目录。

## 已知功能缺口及验收范围

- **CID / data 内嵌图片未贯通。** [core/mime.rs:90](../core/mime.rs#L90) 的清洗只允许 http/https/mailto。实际 MIME 定向复现确认，cid 和 data 图片源在阅读器之前已被移除；直接给 worker 注入 data 图片的测试不能证明真实邮件支持。该限制来自上游。CID 需要核心提供 Content-ID 与附件资源映射，data 图片需要限定格式/大小的独立规则，不能简单开放全部 URI。
- 对真实账号完整登录、持续收信、实际授权发信和附件、多屏/输入法/Narrator，以及最新安装器的运行中升级与卸载，仍需正式验收；此前端点 TLS/NOOP 和公开 OAuth 接口检查只证明连接可达。
- 没有发现要求改回 WebView 的证据；以上问题主要属于事务、身份、隔离和资源预算。

## 本轮验证与记录

- 重新 fetch 后上游基线未变化。
- 常规核心 61 项通过、5 项默认忽略；Windows 29 项通过、1 项默认忽略。格式与 diff 检查通过。
- 额外使用五个 review-only 定向探针：授权失败全局配置、账号保存失败令牌、身份变更 UID/正文、图片预算及 MIME 内嵌图片。结果均确认当前行为；它们不是问题修复后的验收。
- 定向探针使用合成数据、内存凭据库/本机 OAuth fixture 或不解码的 PNG metadata；没有真实账号登录或邮件发送。未触发示例模式的真实凭据写入。
- 临时探针已按原始字节恢复，产品源码未修改。证据和源代码快照保存于忽略的 `build/final-review-20261002/`，本轮跨日期完成。
- 原有 44 项原生窗口验收、20 封真实缓存邮件的 180 组阅读检查及本机 10808 的 12 项实网检查作为已有证据，没有冒充本轮重新执行。

建议修复顺序：授权事务与示例隔离 → 邮箱身份/同步命名空间 → 所有阅读模式的图片预算 → 授权取消 → 内嵌图片与真实环境验收。复查相应场景后再进行稳定版发布判断。
