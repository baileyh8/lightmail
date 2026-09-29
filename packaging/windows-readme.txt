Lightmail for Windows (x64)

Requires Windows 10 1809 / Windows 11. No WebView2 or other runtime is needed.
Source, support and configuration: https://github.com/baileyh8/lightmail
License: GPL-3.0-or-later. See LICENSE and THIRD_PARTY_NOTICES.md.

Install with the per-user setup, or extract this ZIP and run Lightmail.exe.
The binary is not code-signed. No administrator access is needed.
Data: %LOCALAPPDATA%\Bailey\Lightmail\data\Mail (exact path may depend on OS conventions).
Credentials: Windows Credential Manager; removing an account clears its credentials.
Uninstall preserves user mail, drafts and settings.

Gmail: configure your Google OAuth Desktop client, then sign in in your browser.
QQ / 163: enable IMAP/SMTP in webmail; use the generated client authorization code.
Translation: configure an OpenAI Chat Completions compatible service.
Apple system translation is only available in the native macOS app.
Fixed system HTTP/SOCKS proxies are supported; PAC is not yet supported.
