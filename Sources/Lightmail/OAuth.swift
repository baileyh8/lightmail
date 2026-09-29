import AppKit
import CryptoKit
import Darwin
import Foundation

struct GoogleTokens: Codable {
  var accessToken: String
  var refreshToken: String
  var expiresAt: Double
}

final class LoopbackCallback: @unchecked Sendable {
  private let lock = NSLock()
  private var socketFD: Int32 = -1
  let port: UInt16
  init() throws {
    let fd = Darwin.socket(AF_INET, SOCK_STREAM, 0)
    guard fd >= 0 else { throw MailAppError.message("无法建立 Google 登录回调") }
    var address = sockaddr_in()
    address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
    address.sin_family = sa_family_t(AF_INET)
    address.sin_addr.s_addr = inet_addr("127.0.0.1")
    address.sin_port = 0
    let status = withUnsafePointer(to: &address) { p in
      p.withMemoryRebound(to: sockaddr.self, capacity: 1) {
        Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
      }
    }
    guard status == 0, Darwin.listen(fd, 4) == 0 else {
      Darwin.close(fd)
      throw MailAppError.message("无法监听本机 Google 回调")
    }
    var length = socklen_t(MemoryLayout<sockaddr_in>.size)
    withUnsafeMutablePointer(to: &address) { p in
      p.withMemoryRebound(to: sockaddr.self, capacity: 1) {
        _ = Darwin.getsockname(fd, $0, &length)
      }
    }
    port = UInt16(bigEndian: address.sin_port)
    socketFD = fd
  }
  func close() {
    lock.lock()
    let fd = socketFD
    socketFD = -1
    lock.unlock()
    if fd >= 0 {
      Darwin.shutdown(fd, SHUT_RDWR)
      Darwin.close(fd)
    }
  }
  deinit { close() }
  func wait(state: String) async throws -> String {
    try await withTaskCancellationHandler {
      try await Task.detached(priority: .userInitiated) { [self] in
        defer { close() }
        let deadline = Date().addingTimeInterval(180)
        while Date() < deadline {
          let fd = lock.withLock { socketFD }
          guard fd >= 0 else { throw CancellationError() }
          var descriptor = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
          guard Darwin.poll(&descriptor, 1, 1000) > 0 else { continue }
          let client = Darwin.accept(fd, nil, nil)
          guard client >= 0 else { continue }
          defer { Darwin.close(client) }
          var clientPoll = pollfd(fd: client, events: Int16(POLLIN), revents: 0)
          guard Darwin.poll(&clientPoll, 1, 3000) > 0 else { continue }
          var buffer = [UInt8](repeating: 0, count: 8192)
          let count = Darwin.read(client, &buffer, buffer.count)
          guard count > 0, let request = String(bytes: buffer.prefix(count), encoding: .utf8),
            let first = request.components(separatedBy: "\r\n").first
          else { continue }
          let pieces = first.split(separator: " ")
          guard pieces.count >= 2, pieces[0] == "GET",
            let components = URLComponents(string: "http://127.0.0.1" + pieces[1]),
            components.path == "/oauth/callback"
          else { continue }
          let values = Dictionary(
            components.queryItems?.map { ($0.name, $0.value ?? "") } ?? [],
            uniquingKeysWith: { _, latest in latest })
          guard values["state"] == state else { continue }
          let ok = values["code"] != nil && values["error"] == nil
          let html =
            "<!doctype html><meta charset=utf-8><title>轻邮</title><body style='font:20px -apple-system;padding:80px;color:#226451'>\(ok ? "授权已完成，可以关闭此页并返回轻邮。" : "授权未完成，请返回轻邮重试。")</body>"
          let response =
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: \(html.utf8.count)\r\nConnection: close\r\n\r\n\(html)"
          response.withCString { p in _ = Darwin.write(client, p, response.utf8.count) }
          guard ok, let code = values["code"] else {
            throw MailAppError.message("Google 登录已取消或未获授权")
          }
          return code
        }
        throw MailAppError.message("Google 登录等待超时，请重试")
      }.value
    } onCancel: {
      self.close()
    }
  }
}

enum GoogleOAuth {
  static func base64url(_ data: Data) -> String {
    data.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(
      of: "/", with: "_"
    ).replacingOccurrences(of: "=", with: "")
  }
  static func tokenRequest(_ fields: [String: String], previous: String = "") async throws
    -> GoogleTokens
  {
    var request = URLRequest(url: URL(string: "https://oauth2.googleapis.com/token")!)
    request.httpMethod = "POST"
    request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
    request.timeoutInterval = 30
    var components = URLComponents()
    components.queryItems = fields.map { URLQueryItem(name: $0.key, value: $0.value) }
    request.httpBody = components.percentEncodedQuery?.replacingOccurrences(of: "+", with: "%2B")
      .data(using: .utf8)
    let (data, response) = try await URLSession.shared.data(for: request)
    guard let http = response as? HTTPURLResponse, http.statusCode == 200,
      let obj = try JSONSerialization.jsonObject(with: data) as? [String: Any],
      let access = obj["access_token"] as? String
    else { throw MailAppError.message("Google 令牌获取失败，请检查 OAuth 客户端配置或重新授权") }
    let refresh = obj["refresh_token"] as? String ?? previous
    guard !refresh.isEmpty else { throw MailAppError.message("Google 未返回长期授权，请重新登录并允许离线访问") }
    return GoogleTokens(
      accessToken: access, refreshToken: refresh,
      expiresAt: Date().timeIntervalSince1970 + (obj["expires_in"] as? Double ?? 3600))
  }
  @MainActor static func signIn(account: Account, clientID: String, clientSecret: String)
    async throws
  {
    guard !clientID.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
      throw MailAppError.message("请先在设置中填写 Google OAuth Desktop Client ID")
    }
    let callback = try LoopbackCallback()
    let state = UUID().uuidString
    var random = [UInt8](repeating: 0, count: 48)
    guard SecRandomCopyBytes(kSecRandomDefault, random.count, &random) == errSecSuccess else {
      throw MailAppError.message("无法生成安全登录参数")
    }
    let verifier = base64url(Data(random))
    let challenge = base64url(Data(SHA256.hash(data: Data(verifier.utf8))))
    let redirect = "http://127.0.0.1:\(callback.port)/oauth/callback"
    var auth = URLComponents(string: "https://accounts.google.com/o/oauth2/v2/auth")!
    auth.queryItems = [
      "client_id": clientID, "redirect_uri": redirect, "response_type": "code",
      "scope": "openid email https://mail.google.com/", "access_type": "offline",
      "prompt": "consent select_account", "state": state, "code_challenge": challenge,
      "code_challenge_method": "S256", "login_hint": account.address,
    ].map { URLQueryItem(name: $0.key, value: $0.value) }
    guard let url = auth.url, NSWorkspace.shared.open(url) else {
      throw MailAppError.message("无法打开系统浏览器")
    }
    let code = try await callback.wait(state: state)
    var fields = [
      "client_id": clientID, "code": code, "redirect_uri": redirect,
      "grant_type": "authorization_code", "code_verifier": verifier,
    ]
    if !clientSecret.isEmpty { fields["client_secret"] = clientSecret }
    let tokens = try await tokenRequest(fields)
    var identity = URLRequest(url: URL(string: "https://openidconnect.googleapis.com/v1/userinfo")!)
    identity.setValue("Bearer \(tokens.accessToken)", forHTTPHeaderField: "Authorization")
    identity.timeoutInterval = 20
    let (data, response) = try await URLSession.shared.data(for: identity)
    guard (response as? HTTPURLResponse)?.statusCode == 200,
      let obj = try JSONSerialization.jsonObject(with: data) as? [String: Any],
      let address = obj["email"] as? String,
      address.caseInsensitiveCompare(account.address) == .orderedSame
    else { throw MailAppError.message("Google 授权账号与填写的邮箱不一致，请选择对应账号") }
    try await SecretStore.save(
      String(decoding: JSONEncoder().encode(tokens), as: UTF8.self), for: "account:\(account.id)")
  }
}

actor CredentialProvider {
  private var pending: [String: Task<String, Error>] = [:]
  func credential(account: Account, clientID: String) async throws -> String {
    if account.provider == "demo" || account.provider == "local" { return "" }
    guard let value = try await SecretStore.read("account:\(account.id)") else {
      throw MailAppError.message("\(account.name) 尚未授权，请在账号设置中完成登录")
    }
    guard account.authKind == "oauth" else { return value }
    let token = try JSONDecoder().decode(GoogleTokens.self, from: Data(value.utf8))
    if token.expiresAt > Date().timeIntervalSince1970 + 120 { return token.accessToken }
    if let task = pending[account.id] { return try await task.value }
    let task = Task<String, Error> {
      let clientSecret = try await SecretStore.read("google-client-secret") ?? ""
      var fields = [
        "client_id": clientID, "grant_type": "refresh_token", "refresh_token": token.refreshToken,
      ]
      if !clientSecret.isEmpty { fields["client_secret"] = clientSecret }
      let refreshed = try await GoogleOAuth.tokenRequest(fields, previous: token.refreshToken)
      try await SecretStore.save(
        String(decoding: JSONEncoder().encode(refreshed), as: UTF8.self),
        for: "account:\(account.id)")
      return refreshed.accessToken
    }
    pending[account.id] = task
    defer { pending[account.id] = nil }
    return try await task.value
  }
}
