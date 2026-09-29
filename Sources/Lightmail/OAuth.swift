import AppKit

// The platform opens the browser. PKCE, callback validation, token exchange,
// identity validation and refresh policy are implemented in Rust.
enum GoogleOAuth {
  @MainActor static func signIn(account: Account, clientID: String, clientSecret: String) async throws {
    let login = try GoogleLogin(account: account, clientId: clientID, platform: MacPlatformServices())
    guard let url = URL(string: login.authorizationUrl()), NSWorkspace.shared.open(url) else {
      throw MailAppError.message("无法打开系统浏览器")
    }
    try await login.finish(clientSecret: clientSecret)
  }
}
