# PR #3 修复与再次审核

日期：2026-10-04。当前迭代基于作者最新主分支 `2c74c4315da5405bc9048a705a1dee6bca192ad8`，分支 `fix/upstream-review-followup-20261004`。

## 上游状态与处理范围

[PR #3](https://github.com/baileyh8/lightmail/pull/3) 已由作者合并。作者随后提交 `2fb2cb2`，修复审核中的四项问题；后续提交补齐 v0.0.5 源码包、Windows 图片 fixture 请求时序和像素断言。本轮先保留此前本地实现，再从最新主分支建立独立迭代，采用作者已经完成的接口和产品修复。

本地旧实现保留在 `checkpoint/pr3-local-fixes-20261004` 的 `e0f885d`，未推送。当前分支继续使用 GPUI Kit + Blitz 和 Windows Credential Manager。当前追加变更没有修改 Swift 产品实现或生成绑定。

## 四项作者意见的复核

| 意见 | 作者的修复 | 复核结论 |
| --- | --- | --- |
| Windows 窄阅读栏拖动区返回 HTCLIENT | 按阅读栏宽度采用紧凑按钮、缩小间距和左边距，保留 24px 最小拖动区域；验收检查拖动宽度和窗口按钮覆盖 | 作者 CI 已通过完整 UI/安装验收；本机增加四宽度和 DPI 组合复测 |
| Mac 先写 grant 再保存账号，身份变更时状态不一致 | 导出核心 `begin_google_login` / `finish_google_login` 及账号代理记录；Mac 接入共享事务，授权后不再次保存账号 | 准备阶段拒绝身份变化，失败时恢复原 grant；Mac 新桥接已由作者 CI 编译运行 |
| OAuth 刷新覆盖新授权 | 删除直接持久化的旧 `GoogleLogin::finish`；两个前端均通过应用的账号凭据 lane 提交；加入二次读取后的暂停测试 | 单服务实例的原竞态已修复。多服务实例仍存在独立 lane，本轮补齐共同锁 |
| CID/data 图片被 Mac CSP 拒绝 | 明确允许图片时 `img-src` 增加 `data:`；WKWebView 合成 MIME fixture 检查默认禁图及 opt-in 后 `naturalWidth` | 作者 Mac CI 实际通过 10 项阅读器检查。本轮补充 SVG/HTML data 与其他 CSP 边界回归 |

## 本轮追加修复

### 1. 跨应用实例的凭据串行边界

复现使用同一内存凭据库的两个平台适配器及三个应用服务：第一个刷新暂停在二次读取旧值之后；第二个服务尝试刷新；第三个服务完成合成 Google 授权并提交。作者主分支的各服务分别创建 lane，第二个服务能够并发刷新，新增回归在未修复代码上实际失败。

`core/credentials.rs` 按实际 `account:<id>` 凭据键的账号 ID 提供进程内共同异步锁。refresh、授权事务、密码保存、账号删除、失败补偿沿用原临界区，通过 lane 取得同一把锁。注册表保存 Weak 引用，创建新条目时清理已失效条目；不同账号保持独立。

回归用 channel 精确暂停比较快照，用有期限的未完成断言确认其他刷新和授权提交等待，再放行刷新。最终新客户端 ID/secret 保留，另一服务复用刷新结果，网络总请求只有一次刷新及一次授权/身份校验。网络请求全部是本机 fixture，没有访问 Google 或真实凭据。

此锁协调同一进程中的写入口；不声明能锁住其他进程或外部直接改写的 Credential Manager/Keychain。原二次读取比较仍保留。正常桌面实例由已有数据库目录锁约束；平台适配器仍只负责一次凭据操作。

### 2. OAuth 会话与提交配置一致

`begin_google_login` 记录经过规范化的代理配置和发起的 MailEngine。`finish_google_login` 在等待回调和交换 token 之前确认它们一致，拒绝把另一个数据库准备的会话或使用不同代理完成的授权提交到当前配置。

新增回归检查身份修改、另一个引擎和代理变更都在网络交换之前拒绝，原账号和凭据保持不变。沿用作者已导出的 API；当前生成 Swift、C header、module map 重新生成并规范化后逐字一致。

### 3. 可重复的 Windows 窄窗口验收

示例验收窗口允许 984px 最小宽度，以便较大的本机桌面也能复现 hosted runner 的窄窗口。正常产品窗口保留原最小宽度设置。矩阵请求 984/1040/1120/1280 逻辑宽度，等待连续两个相同的 viewport/控件布局，记录实际 viewport、DPI、控件边界与 Win32 命中码。

归档、删除、菜单、翻译、复制均要求有可见点击区域、不侵入窗口控制区，并返回 HTCLIENT；空白拖动区至少 24px 且返回 HTCAPTION。没有减弱原命中断言或只靠延长等待掩盖布局问题。

### 4. data 图片策略的核心回归

新增测试在正式 HTML 清洗和 `reader_document` 上验证：受限 PNG data 源保留；SVG data、HTML data 超链接被移除；默认禁图；明确允许后仅图片资源获得 data 能力，默认资源、连接、iframe、表单和 base URI 仍禁用。

## 验证记录

上游基线证据来自 [CI run 37137739694](https://github.com/baileyh8/lightmail/actions/runs/37137739694)，SHA 精确为 `2c74c43`：Windows/Linux/Mac 核心各 69 项通过；Mac 27 项 self-tests、10 项 WKWebView 阅读器检查及协议/内存检查通过；Windows 34 项单元测试、44 项原生 UI 检查、安装启动/关闭/卸载数据保留检查通过。另一次 [run 37139267820](https://github.com/baileyh8/lightmail/actions/runs/37139267820) 也成功。上游 Windows artifact 中拖动区域 92px，复制按钮右边界 842px，小于窗口控件左边界 854px。

本轮本机当前补丁：

- `cargo test --locked --lib`：72 通过、5 ignored。需要 fixture 的翻译/协议测试已另外执行；两个显式公网诊断和大数据 benchmark 本轮未运行。
- `cargo test --locked --release -p lightmail-desktop`：34 通过、1 ignored。忽略项为需要安装浏览器的显式 loopback 浏览器代理验收。
- `python scripts/check.py --protocols-only`：翻译 fixture 及 direct、HTTP CONNECT、SOCKS5 的 TLS 邮件协议、不同账号路由、CID 选择性获取、发送结果及防重复提交通过。
- Headless 示例：无 UI 读取并导出成功。
- UniFFI：用当前 DLL 重新生成，三份规范化输出与已提交绑定逐字一致。
- Windows 原生 UI：最终 46 项检查及额外正文像素检查通过；峰值 141.34 MiB。四宽度×三 DPI 共 12 个请求组合记录实际几何；100% 和 150% 的四种实际宽度全部覆盖，200% 的后三个请求实际仍为 984px，没有声称验证了 200% 下的其他大窗口宽度。所有实际布局的拖动区约 82.67–220px，均返回 HTCAPTION，其他工具栏按钮均返回 HTCLIENT 且没有侵入窗口控制区。抽查 984px/200% 截图，紧凑工具栏完整可见。

当前补丁的 Mac 原生程序和 Linux 二进制没有在本机运行；上游成功的 Mac/Linux CI 对应上游基线，不能当作当前新增 Rust 代码的完整跨平台 CI。本轮复测没有重新安装或卸载用户现有应用，本机验收使用 `-SkipInstaller`；安装/卸载结论来自上游隔离 runner。

本地证据位于忽略目录 `build/pr3-fixes-20261004/`：未修复跨实例回归失败日志、上游 CI 日志/artifacts、当前核心/桌面测试日志、生成绑定、headless 输出、隔离 UI JSON 和合成截图。没有提交账号数据库、真实邮件、凭据或私人截图。

## 再次审核结论

已复查所有已知账号凭据写入、锁生命周期、事务补偿、取消路径、会话配置、生成接口及窄工具栏命中。作者原四项缺陷已有对应修复和证据，本轮补齐了多实例共同锁及会话配置边界。没有发现仍未修复的阻断项。

当前补丁的托管跨平台 CI 将通过后续独立 PR 执行；本地审核和定向修复已完成。真实服务商持续收发、授权和投递未由本轮合成 fixture 证明。
