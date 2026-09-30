# 给上游的提议：Windows 分支需要共同决定的事项

| 项 | 内容 |
| --- | --- |
| 基线 | 上游 `baileyh8/lightmail` `5ab113a` |
| 分支 | `feat/windows-gpui-kit`（本地分支，未推送） |
| 日期 | 2026-09-30 |
| 相关 | [迭代计划](windows-iteration-plan.md) I6，问题 #15–18 |

本分支修改共享核心、Windows 客户端及相关构建、打包、文档，没有改 Swift、生成的绑定和 Mac 专属资源；另外从现有图标转出了 Windows 的 `Resources/AppIcon.ico`。导出给 Swift 的接口签名没有变化，所以提交里的 Swift 绑定不用重新生成。下面几件事需要改导出接口或 Swift 代码，整理出来供你决定。

## 1. 已经对 Mac 生效的核心行为变化

这些修复在共享核心里（提交 `821263d`），Mac 不用改代码也会生效：

| 场景 | 以前 | 现在 |
| --- | --- | --- |
| 删除仍有待发或正在提交邮件的账号 | 直接删除，正在提交的邮件被中断，`delivery_unknown` 记录一起消失 | 拒绝删除，提示"此账号有待发送或正在提交的邮件，请先撤销待发送邮件或等待发送结果"。先删数据库记录，最后清理凭证 |
| OAuth 账号保存时带着密码参数 | 隐藏密码框里残留的值会覆盖 Google 令牌（两端都有，Mac 在 `SettingsView`） | 核心忽略 OAuth 账号的密码参数 |
| 新建密码账号不填密码 | 能保存，之后同步失败 | 提示"请填写密码或客户端授权码" |
| 保存账号时数据库出错 | 新凭证已经写入，不回滚 | 恢复原来的凭证 |
| OAuth 刷新和重新登录同时发生 | 刷新结果可能覆盖刚完成的登录 | 写回前比较存储值，已被更新就保留新值 |

Mac 端能看到的变化只有第一条：删除这类账号会收到错误提示，不会再静默删除。

## 2. 建议导出给 Swift 的接口

本分支新增了几个核心接口，目前只作为 Rust 公开接口给 Windows 用，没有加 `#[uniffi::export]`：

| 接口 | 位置 | 能替代 Mac 端的什么 |
| --- | --- | --- |
| `MailApplication::load_older(account_id, folder_id, scope)` | `core/application.rs` | `MailStore.loadOlder`（`Sources/Lightmail/MailStore.swift:302`）。Swift 按 `folder.role == current.role` 选文件夹，星标视图的角色匹配不到任何文件夹，所以永远补拉不到。核心版会为星标选 All Mail（没有就选除垃圾箱、垃圾邮件外的所有文件夹），跳过停用、演示和本地账号，以及草稿和待发送 |
| `DraftState` 及其 `editable`、`withdrawable`、`retryable`、`resolvable`、`deletable`、`in_outbox` | `core/composition.rs` | Swift 里按 `status` 字符串判断能否编辑、撤销、重试、删除的地方 |
| `MailApplication::resolve_delivery(id, delivered)` | `core/application.rs` | Mac 目前没有处理 `delivery_unknown` 的入口，这类邮件会一直留在待发送里。它只改状态，不会发送 |
| `provider_color(provider)` | `core/presentation.rs` | 两端各自写的新账号默认颜色 |
| `MailApplication::set_automatic_receiving(enabled)` / `automatic_receiving()` | `core/application.rs` | 仅暂停后台收信和预加载，保留手动操作及发件计时器。Windows 托盘使用，默认开启；现有 Mac 行为不变 |

导出时 `load_older` 按现有惯例做成 async，经 `run()` 执行，future 被丢弃时会中止任务。导出后需要重新生成绑定（`bash scripts/build.sh`），再把 `loadOlder` 的编排换成一次调用。`reader_content`、`fetch_resource` 是给原生阅读器用的，Mac 用 WKWebView，不需要导出。

## 3. 类型化错误（#16）

`MailError` 只有 `Failure { message }` 一个变体（`core/models.rs:4`）。两端都只能把文字显示出来，没法区分"已取消""需要重新登录""网络问题"，只能匹配中文文案。

建议在保留 `Failure` 的前提下增加变体，每个变体仍然带面向用户的中文消息：

```rust
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MailError {
    #[error("{message}")] Failure { message: String },
    #[error("操作已取消")] Cancelled,
    #[error("{message}")] Busy { message: String },
    #[error("{message}")] NeedsSignIn { account_id: String, message: String },
    #[error("{message}")] Network { message: String },
    #[error("{message}")] Server { message: String },
    #[error("{message}")] Invalid { message: String },
}
```

`fail()` 继续产生 `Failure`，现有调用不用一次改完，可以先改取消、授权、网络这几处。Swift 端的 `userError(error)` 改成按 case 处理，例如 `NeedsSignIn` 直接打开对应账号的登录。这会改动导出的错误类型，需要重新生成绑定。

## 4. 拆分事件类型（#15）

同步成功时，核心用 `Problem` 事件、`failed: false` 来传递结果消息并清除错误（`core/application.rs:832`）。客户端必须先检查 `failed`，才知道这是不是问题。

建议：`Problem` 只表示失败；成功结果放进 `SyncFinished` 的 `message`，或新增 `Recovered`。两端事件处理里清除账号错误的逻辑要跟着改：Windows 在 `desktop/src/app.rs` 的 `consume`，Mac 在 `MailStore` 的事件分发。

## 5. 代理路由改为连接时查询（#17，观察项）

`credential()` 每次取凭证时查询 IMAP、SMTP 主机的代理，写进引擎全局的 `Routes`（`core/application.rs:728-732`，`core/store.rs:290`）；建立连接时再读这张表（`core/transport.rs:138`、`1123`）。系统代理改了之后，要等下一次取凭证才会生效，而且代理状态放在了引擎里。

建议由传输层在建立连接时向 `PlatformServices::proxy_for` 查询，去掉引擎里的路由缓存。目前没有发现实际故障，可以放在类型化错误之后。

## 6. 同步结束时的重复清理（#18，低优先级）

完整同步时，收件箱先同步一次并清理正文缓存，全部文件夹同步完再清理一次（`core/application.rs:818-825`）；随后触发的预加载结束时又清理一次（`core/application.rs:881`）。结果是对的，只是多做了功。可以只保留同步结束和预加载结束时的清理。

## 7. 内核级单实例保护

Windows 在打开数据库前，对数据目录里的 `.lightmail-instance.lock` 加操作系统独占锁（`desktop/src/instance.rs`），第二个进程提示后退出。Mac 没有这层保护：两个进程打开同一个库时，后启动的进程会做崩溃恢复，把前一个进程的待发邮件改回草稿，两边的发件计时器也会同时运行。

建议把锁放进核心，在 `MailEngine::new` 取得、引擎释放时放开，两端就都有保护。需要注意：Mac 切换示例库和真实库时会创建新引擎，旧引擎可能短暂还在。两者目录不同（`Preview` 和 `Mail`），不会互相阻塞，但切回同一个目录前要先 `stop()` 并释放旧引擎。

## 8. 发版时一起更新

- `README.md` 的"从 v0.0.4 开始"和 `docs/getting-started.md` 写着 Windows 需要 WebView2 Runtime。这对已发布的 v0.0.4 安装包是对的，所以本分支没改；新版本发布时要去掉。
- 下一版的发布说明：原生阅读器替代 WebView2、无边框窗口、同一数据目录单实例、超长凭证分片、列表查询移出界面线程，以及第 1 节的核心行为变化。
- CI 的 Rust 固定为 1.97.0，Inno Setup 固定为 6.7.1（Chocolatey 上 6.7 系列的最新版）。本地验证用的是 Inno Setup 6.7.3。
- `THIRD_PARTY_NOTICES.md` 已按当前锁文件重新生成。清单按脚本约定包含测试依赖，其中有 Kit 测试支持带来的 `proptest` 等仅用于测试的包。

## 9. 仍需人工验证

自动检查不能代替这些，结果记录在 `docs/validation/`（该目录不提交）：

- 中文输入法：微软拼音和一种第三方输入法的组合串、候选框位置、中英混输。
- Narrator：账号、邮件行、按钮、输入框、窗口按钮的名称和状态。
- DPI：100%、150%、200%，以及跨屏拖动窗口。
- 无边框窗口：悬停最大化按钮时的贴靠布局、双击拖动区最大化、右键拖动区弹出系统菜单、Tab 到窗口按钮后回车。
- 真实服务商：QQ、163 授权码账号和 Gmail OAuth 的收信。发信需要账号所有人明确授权。
