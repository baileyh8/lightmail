import Foundation

enum SelfTests {
  static func runIntegration(baseURL: String) {
    let finished = DispatchSemaphore(value: 0)
    Task.detached {
      var failures = 0
      let cases: [(String, Bool, Bool)] = [
        ("fixture-valid", false, true), ("fixture-valid", true, true),
        ("fixture-truncated", false, false), ("fixture-truncated", true, false),
        ("fixture-dropped", true, false), ("fixture-omitted", false, false),
        ("fixture-denied", false, false), ("fixture-redirect", false, false),
      ]
      for (model, stream, expectSuccess) in cases {
        var c = TranslationConfiguration()
        c.baseURL = baseURL
        c.model = model
        c.stream = stream
        do {
          let result = try await TranslationService.translate(
            subject: "Fixture 2026",
            markdown: "Hello.\n\nAmount USD 12.50. Link https://example.com", hash: "fixture",
            configuration: c, key: ""
          ) { _, _, _ in }
          let valid =
            result.blocks.count == 2 && result.markdown.contains("12.50")
            && result.markdown.contains("https://example.com")
          if expectSuccess && valid {
            print("PASS \(model) stream=\(stream)")
          } else {
            failures += 1
            print("FAIL \(model) unexpectedly completed")
          }
        } catch {
          if expectSuccess {
            failures += 1
            print("FAIL \(model): \(userError(error))")
          } else {
            print("PASS \(model) rejected incomplete/unsafe response")
          }
        }
      }
      print(
        "Chat integration: \(cases.count - failures) passed, \(failures) failed (local synthetic service only)"
      )
      if failures > 0 { exit(1) }
      finished.signal()
    }
    if finished.wait(timeout: .now() + 180) == .timedOut {
      print("Integration test timed out")
      exit(1)
    }
  }
  @MainActor static func run() {
    var failures: [String] = []
    var passed = CredentialTests.run() + CredentialTests.runAuthorizationPresentation()
    func check(_ name: String, _ body: () throws -> Bool) {
      do {
        if try body() {
          passed += 1
          print("PASS \(name)")
        } else {
          failures.append(name)
          print("FAIL \(name)")
        }
      } catch {
        failures.append(name)
        print("FAIL \(name): \(error.localizedDescription)")
      }
    }
    check("Chat endpoint keeps version once") {
      try TranslationService.endpoint("https://example.com/v1/").absoluteString
        == "https://example.com/v1/chat/completions"
    }
    check("Markdown renders block tables instead of pipe text") {
      let blocks = MarkdownDocument.parse("| Item | Price |\n| --- | --- |\n| Book | 12.50 |")
      guard case .table(_, let rows)? = blocks.first else { return false }
      return rows.count == 2 && rows[0].count == 2 && String(rows[1][1].characters) == "12.50"
    }
    check("Markdown table cell line breaks do not leak HTML tags") {
      let blocks = MarkdownDocument.parse("| A | B |\n| --- | --- |\n| One<br>Two | Three |")
      guard case .table(_, let rows)? = blocks.first else { return false }
      return rows.count == 2 && String(rows[1][0].characters) == "One\u{2028}Two"
    }
    check("Markdown retains headings lists quotes and code blocks") {
      let blocks = MarkdownDocument.parse("Title\n===\n\n> Quoted\n\n- One\n- Two\n\n```\nlet x=1\n```")
      let pieces = blocks.compactMap { if case .paragraph(let p) = $0 { return p }; return nil } as [MarkdownPiece]
      return pieces.contains { $0.heading == 1 } && pieces.contains { $0.quoteDepth == 1 }
        && pieces.filter { $0.prefix == "•" }.count == 2 && pieces.contains { $0.code }
    }
    check("Mail follows the configured system HTTPS proxy") {
      let route = try MailProxyRoute.resolve(host: "imap.gmail.com", settings: [
        "HTTPSEnable": 1, "HTTPSProxy": "127.0.0.1", "HTTPSPort": 7897,
      ])
      return route.kind == "http" && route.host == "127.0.0.1" && route.port == 7897
    }
    check("Mail follows a SOCKS-only system proxy") {
      let route = try MailProxyRoute.resolve(host: "smtp.gmail.com", settings: [
        "SOCKSEnable": 1, "SOCKSProxy": "127.0.0.1", "SOCKSPort": 7897,
      ])
      return route.kind == "socks5" && route.port == 7897
    }
    check("Mail respects system proxy bypass rules") {
      try MailProxyRoute.resolve(host: "localhost", settings: [
        "HTTPSEnable": 1, "HTTPSProxy": "127.0.0.1", "HTTPSPort": 7897,
        "ExceptionsList": ["localhost"],
      ]).kind == "direct"
    }
    check("Raw mail sockets prefer SOCKS when both system routes exist") {
      let route = try MailProxyRoute.resolve(host: "imap.gmail.com", settings: [
        "HTTPSEnable": 1, "HTTPSProxy": "127.0.0.1", "HTTPSPort": 7897,
        "SOCKSEnable": 1, "SOCKSProxy": "127.0.0.1", "SOCKSPort": 7897,
      ])
      return route.kind == "socks5"
    }
    check("Existing Chat endpoint is accepted") {
      try TranslationService.endpoint("https://example.com/v1/chat/completions").absoluteString
        == "https://example.com/v1/chat/completions"
    }
    check("Remote plaintext endpoint is rejected") {
      do {
        _ = try TranslationService.endpoint("http://example.com/v1")
        return false
      } catch { return true }
    }
    check("Loopback fixture endpoint is accepted") {
      try TranslationService.endpoint("http://127.0.0.1:9000/v1").host == "127.0.0.1"
    }
    check("URL userinfo credentials are rejected") {
      do {
        _ = try TranslationService.endpoint("https://secret@example.com/v1")
        return false
      } catch { return true }
    }
    check("Protected links and code restore") {
      let p = ProtectedText(
        text: "Email a@example.com and visit https://example.com. Use `x=3`.", prefix: "T")
      return try p.restore(text: p.protectedText()) == "Email a@example.com and visit https://example.com. Use `x=3`."
    }
    check("Missing protected tokens rejected") {
      let p = ProtectedText(text: "https://example.com", prefix: "T")
      do {
        _ = try p.restore(text: "missing")
        return false
      } catch { return true }
    }
    check("Numeric drift rejected") {
      let p = ProtectedText(text: "Due 2026-10-01. USD 12.50.", prefix: "T")
      do {
        _ = try p.restore(text: "Due 2026-10-02. USD 12.50.")
        return false
      } catch { return true }
    }
    check("Long paragraph fully covered") {
      let s = String(repeating: "abc ", count: 5000)
      return TranslationService.blocks(s).map(\.text).joined() == s
    }
    check("Rust bridge and isolated demo database") {
      let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
        "lightmail-test-\(UUID().uuidString)")
      defer { try? FileManager.default.removeItem(at: directory) }
      let core = try MailEngine(directory: directory.path)
      try core.seedDemo()
      let accounts = try core.accounts()
      let rows = try core.listMessages(
        query: MessageQuery(
          accountId: "", folderId: "", scope: "inbox", search: "", unreadOnly: false, limit: 100,
          offset: 0))
      return accounts.count == 4 && rows.count == 7 && accounts.allSatisfy { $0.provider == "demo" }
    }
    print("Self-tests: \(passed) passed, \(failures.count) failed")
    if !failures.isEmpty { exit(1) }
  }
}
