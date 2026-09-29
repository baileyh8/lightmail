# 参与轻邮

欢迎修复问题、改进邮件兼容性，或让阅读体验更轻、更顺手。提交大功能前，请先在 Issue 中说明使用场景。

## 本地开发

需要 Apple Silicon Mac、macOS 15+、Swift 6+、Rust stable、Python 3 和 Apple Command Line Tools。没有 Rust 时，`bash scripts/bootstrap-rust.sh` 会将工具链安装到项目 `.tooling/`。

```sh
bash scripts/build.sh
python3 scripts/check.py
```

`core/` 是 Rust 邮件内核，`Sources/Lightmail/` 是原生界面。`Generated/FFI/` 和 `Sources/Lightmail/Generated/` 由 UniFFI 生成；修改导出接口后重新运行构建脚本，不直接编辑生成代码。

## 提交前

- 邮件解析、缓存或同步变化应添加针对性回归样例。样例只使用 `example.com` / `example.test` 等保留域名和虚构内容。
- 不提交真实 EML、账号数据库、OAuth 凭证、API Key、授权链接或含私人信息的截图。
- 界面变化附演示模式截图；说明 macOS、芯片、复现步骤和实际检查结果。
- 收发与翻译测试优先用本地 fixture；真实发信需要账号所有者明确同意。
- 不通过关闭 TLS 校验、扩大邮件脚本权限、无上限缓存来修复兼容问题。

PR 请描述问题、最终行为、验证方法和仍未覆盖的情况。保持修改集中，避免顺带改动无关功能。

贡献按项目的 **GPL-3.0-or-later** 许可证提供。第三方内容须保留原许可与署名，详见 [第三方许可](THIRD_PARTY_NOTICES.md)。
