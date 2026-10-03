import AppKit
import SwiftUI
import WebKit

enum ReaderRenderingTests {
  @MainActor static func run(directory: String) {
    NSApplication.shared.setActivationPolicy(.prohibited)
    let root = URL(fileURLWithPath: directory)
    func check(_ condition: Bool, _ name: String) {
      guard condition else { print("FAIL reader: \(name)"); exit(1) }
      print("PASS reader: \(name)")
    }
    func pump(_ seconds: Double) { RunLoop.current.run(until: Date().addingTimeInterval(seconds)) }
    func webView(_ view: NSView) -> WKWebView? {
      if let web = view as? WKWebView { return web }
      return view.subviews.lazy.compactMap { webView($0) }.first
    }
    do {
      let port = try String(contentsOf: root.appendingPathComponent("port"), encoding: .utf8)
      let base = "http://127.0.0.1:\(port)"
      let html = """
      <html><head><style>@import url('\(base)/external.css');
      .receipt{width:440px}.receipt td{padding:12px;color:rgb(18,52,86)}
      .background{background-image:url('\(base)/background.png')}</style></head>
      <body><img id='inline-image' src='cid:fixture-logo' width='12' height='12'><table class='receipt'><tr><td class='amount'>20.00</td></tr></table>
      <div class='background' style='width:20px;height:20px'>Image</div>
      <a href='https://example.com/verify?token=a%2Bb%3D'><img src='\(base)/button.png' alt='Verify email' width='140' height='40'></a>
      <script src='\(base)/script.js'></script><iframe src='\(base)/frame'></iframe></body></html>
      """
      let eml = root.appendingPathComponent("render.eml")
      let pixel = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/l9sAAAAASUVORK5CYII="
      let messageSource = "From: fixture@example.com\r\nSubject: Render fixture\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=reader-fixture\r\n\r\n--reader-fixture\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\(html)\r\n--reader-fixture\r\nContent-Type: image/png\r\nContent-ID: <fixture-logo>\r\nContent-Transfer-Encoding: base64\r\n\r\n\(pixel)\r\n--reader-fixture--\r\n"
      try Data(messageSource.utf8).write(to: eml)
      let engine = try MailEngine(directory: root.appendingPathComponent("db").path)
      try engine.seedDemo()
      let account = try engine.accounts()[0]
      let message = try engine.importEml(path: eml.path, accountId: account.id)
      let body = try engine.cachedBody(messageId: message.id)!
      check(body.html.contains("data:image/png;base64,"), "MIME CID resolves into a bounded inline raster image")
      let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 720, height: 500),
                            styleMask: [.titled], backing: .buffered, defer: false)
      let host = NSHostingView(rootView: SafeHTMLView(html: body.html))
      window.contentView = host
      host.layoutSubtreeIfNeeded()
      var web: WKWebView?
      let deadline = Date().addingTimeInterval(15)
      while Date() < deadline {
        pump(0.05)
        web = webView(host)
        if let web, web.url != nil, !web.isLoading { break }
      }
      guard let web else { throw MailAppError.message("No web reader") }
      func evaluate(_ script: String) -> Any? {
        var result: Any?, completed = false
        web.evaluateJavaScript(script) { value, _ in result = value; completed = true }
        let limit = Date().addingTimeInterval(5)
        while !completed && Date() < limit { pump(0.02) }
        return result
      }
      check(evaluate("getComputedStyle(document.querySelector('.amount')).color") as? String == "rgb(18, 52, 86)", "mail stylesheet and classes render")
      check(evaluate("document.querySelector('.receipt').getBoundingClientRect().width") as? Double == 440, "mail table geometry is retained")
      check(evaluate("document.querySelector('.lightmail-image-link-label').getBoundingClientRect().width > 0") as? Bool == true, "image-only action remains visible with images blocked")
      check(evaluate("document.querySelector('a').getAttribute('href')") as? String == "https://example.com/verify?token=a%2Bb%3D", "signed link destination is unchanged")
      pump(0.5)
      let requests = root.appendingPathComponent("requests.txt")
      check(try String(contentsOf: requests, encoding: .utf8).isEmpty, "default reader makes zero image CSS script or frame requests")
      check(evaluate("document.getElementById('inline-image').naturalWidth") as? Int == 0, "inline images remain blocked until explicit image choice")
      host.rootView = SafeHTMLView(html: body.html, loadImages: true)
      host.layoutSubtreeIfNeeded()
      let imageDeadline = Date().addingTimeInterval(10)
      var paths = ""
      while Date() < imageDeadline {
        pump(0.05)
        paths = try String(contentsOf: requests, encoding: .utf8)
        if paths.contains("/button.png") && paths.contains("/background.png") && evaluate("document.getElementById('inline-image').naturalWidth") as? Int == 1 { break }
      }
      check(paths.contains("/button.png") && paths.contains("/background.png"), "explicit image choice loads mail images")
      check(!paths.contains("external.css") && !paths.contains("script.js") && !paths.contains("/frame"), "image choice keeps CSS imports scripts and frames blocked")
      check(evaluate("document.getElementById('inline-image').naturalWidth") as? Int == 1, "explicit image choice renders the CID raster in WKWebView")
      window.contentView = nil
      print("Reader rendering: 10 passed (isolated loopback fixture)")
    } catch { print("FAIL reader: \(userError(error))"); exit(1) }
  }
}

extension ReaderRenderingTests {
  // Uses an isolated database copy prepared by the developer, never the live store.
  // Only aggregate results are printed; no subjects, addresses, body text or links.
  @MainActor static func runCached(directory: String) {
    do {
      let root = URL(fileURLWithPath: directory)
      guard FileManager.default.fileExists(atPath: root.appendingPathComponent("ISOLATED_COPY").path) else {
        throw MailAppError.message("Expected an explicitly isolated cache copy")
      }
      let engine = try MailEngine(directory: root.path)
      guard try engine.accounts().allSatisfy({ !$0.enabled && $0.provider == "local" }) else {
        throw MailAppError.message("Cache acceptance must have all network accounts disabled")
      }
      let rows = try engine.listMessages(query: MessageQuery(accountId: "", folderId: "", scope: "all", search: "", unreadOnly: false, limit: 500, offset: 0))
      let bodies = try rows.compactMap { try engine.cachedBody(messageId: $0.id) }.filter { !$0.html.isEmpty }.prefix(6)
      guard !bodies.isEmpty else { throw MailAppError.message("No cached HTML samples") }
      func findWeb(_ view: NSView) -> WKWebView? {
        if let web = view as? WKWebView { return web }
        return view.subviews.lazy.compactMap { findWeb($0) }.first
      }
      var count = 0
      for body in bodies {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 760, height: 540), styleMask: [.titled], backing: .buffered, defer: false)
        let host = NSHostingView(rootView: SafeHTMLView(html: body.html))
        window.contentView = host; host.layoutSubtreeIfNeeded()
        var web: WKWebView?
        let deadline = Date().addingTimeInterval(15)
        while Date() < deadline {
          RunLoop.current.run(until: Date().addingTimeInterval(0.05))
          web = findWeb(host)
          if let web, web.url != nil, !web.isLoading { break }
        }
        guard let web else { throw MailAppError.message("Cached reader did not start") }
        var done = false, valid = false
        web.evaluateJavaScript("document.body.innerText.trim().length > 0 && document.body.scrollHeight > 0 && document.querySelector('meta[http-equiv]').content.includes(\"img-src 'none'\")") { result, error in
          valid = error == nil && result as? Bool == true; done = true
        }
        let finish = Date().addingTimeInterval(5)
        while !done && Date() < finish { RunLoop.current.run(until: Date().addingTimeInterval(0.02)) }
        guard valid else { throw MailAppError.message("Cached reader sample failed") }
        web.stopLoading(); window.contentView = nil; window.close(); count += 1
      }
      print("Private cached reader: \(count) HTML samples passed; isolated copy, images blocked, no mailbox network access")
    } catch { print("Private cached reader failed (details omitted)"); exit(1) }
  }
}
