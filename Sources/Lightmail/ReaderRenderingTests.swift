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
      <body><table class='receipt'><tr><td class='amount'>20.00</td></tr></table>
      <div class='background' style='width:20px;height:20px'>Image</div>
      <a href='https://example.com/verify?token=a%2Bb%3D'><img src='\(base)/button.png' alt='Verify email' width='140' height='40'></a>
      <script src='\(base)/script.js'></script><iframe src='\(base)/frame'></iframe></body></html>
      """
      let eml = root.appendingPathComponent("render.eml")
      try Data("From: fixture@example.com\r\nSubject: Render fixture\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\(html)".utf8).write(to: eml)
      let engine = try MailEngine(directory: root.appendingPathComponent("db").path)
      try engine.seedDemo()
      let account = try engine.accounts()[0]
      let message = try engine.importEml(path: eml.path, accountId: account.id)
      let body = try engine.cachedBody(messageId: message.id)!
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
      host.rootView = SafeHTMLView(html: body.html, loadImages: true)
      host.layoutSubtreeIfNeeded()
      let imageDeadline = Date().addingTimeInterval(10)
      var paths = ""
      while Date() < imageDeadline {
        pump(0.05)
        paths = try String(contentsOf: requests, encoding: .utf8)
        if paths.contains("/button.png") && paths.contains("/background.png") { break }
      }
      check(paths.contains("/button.png") && paths.contains("/background.png"), "explicit image choice loads mail images")
      check(!paths.contains("external.css") && !paths.contains("script.js") && !paths.contains("/frame"), "image choice keeps CSS imports scripts and frames blocked")
      window.contentView = nil
      print("Reader rendering: 7 passed (isolated loopback fixture)")
    } catch { print("FAIL reader: \(userError(error))"); exit(1) }
  }
}
