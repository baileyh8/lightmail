import AppKit

// The platform opens the browser. PKCE, callback validation, token exchange,
// identity validation and refresh policy are implemented in Rust.
enum GoogleOAuth {
  @MainActor static func signIn(application: MailApplication, account: Account, clientID: String, clientSecret: String) async throws {
    let proxy = try application.accountProxySettings(accountId: account.id)
    let login = try application.beginGoogleLogin(account: account, clientId: clientID, proxy: proxy)
    guard let url = URL(string: login.authorizationUrl()), NSWorkspace.shared.open(url) else {
      throw MailAppError.message("无法打开系统浏览器")
    }
    try await application.finishGoogleLogin(login: login, secret: clientSecret, proxy: proxy)
  }
}
