Lightmail for Windows (x64)

Requires Windows 10 1809 / Windows 11. No WebView2 or other runtime is needed.
Source, support and configuration: https://github.com/baileyh8/lightmail
License: GPL-3.0-or-later. See LICENSE and THIRD_PARTY_NOTICES.md.

Install with the per-user setup, or extract this ZIP and run Lightmail.exe.
The binary is not code-signed. No administrator access is needed.
Data: %LOCALAPPDATA%\Bailey\Lightmail\data\Mail (exact path may depend on OS conventions).
Credentials: Windows Credential Manager; removing an account clears its credentials.
Uninstall preserves user mail, drafts and settings.

Closing the window keeps Lightmail running in the notification area.
Click its tray icon to reopen; right-click for Open, Receive now,
Automatic receiving (checked by default), and Exit. Manual receiving
leaves sending and other commands available. The preference persists.
Windows may put new tray icons inside the overflow menu near the clock.

HTML mail uses the native Blitz HTML/CSS reader by default. It supports
common email layouts; complete browser CSS compatibility is not promised.
Use the reading menu to switch to clean Markdown reading when needed.
Clicking a message link shows its target website and complete address.
Use Copy URL to copy the full address while keeping the confirmation open.
Confirm to open HTTP/HTTPS links in your default browser; Cancel or Esc
keeps them closed. Mailto links use your default mail application after confirmation.

Gmail: configure your Google OAuth Desktop client, then sign in in your browser.
QQ / 163: enable IMAP/SMTP in webmail; use the generated client authorization code.
Translation: configure an OpenAI Chat Completions compatible service.
Apple system translation is only available in the native macOS app.
Fixed system HTTP/SOCKS proxies are supported; PAC is not yet supported.
