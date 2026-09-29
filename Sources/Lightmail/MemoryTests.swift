import AppKit
import SwiftUI

// Offscreen regression harness: isolated database, no credentials or network calls.
enum MemoryTests {
  // Opt-in for a developer's temporary app bundle only. Never enabled by build.sh.
  static var diagnosticDirectory: URL? {
    (Bundle.main.object(forInfoDictionaryKey: "LightmailDiagnosticDirectory") as? String)
      .map { URL(fileURLWithPath: $0) }
  }
  private static var diagnosticTimer: DispatchSourceTimer?
  static func startDiagnosticGuard() {
    guard let directory = diagnosticDirectory else { return }
    let started = Date()
    let log = directory.appendingPathComponent("footprint.log")
    FileManager.default.createFile(atPath: log.path, contents: nil)
    let handle = try? FileHandle(forWritingTo: log)
    let timer = DispatchSource.makeTimerSource(queue: DispatchQueue.global(qos: .userInitiated))
    timer.schedule(deadline: .now(), repeating: .milliseconds(100))
    timer.setEventHandler {
      var info = task_vm_info_data_t()
      var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
      let status = withUnsafeMutablePointer(to: &info) {
        $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
          task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
        }
      }
      let age = Date().timeIntervalSince(started)
      if status == KERN_SUCCESS {
        try? handle?.write(contentsOf: Data("\(age),\(info.phys_footprint)\n".utf8))
        if info.phys_footprint > 384 * 1024 * 1024 { _exit(70) }
      }
      if age > 150 { _exit(0) }
    }
    diagnosticTimer = timer
    timer.resume()
  }
  @MainActor static func run(directory: String) {
    let app = NSApplication.shared
    app.setActivationPolicy(.prohibited)
    let store = MailStore(directory: URL(fileURLWithPath: directory), startAutomatically: false)
    do {
      let readerStress = CommandLine.arguments.contains("--reader-stress")
      let fixture = URL(fileURLWithPath: directory).appendingPathComponent("nested-tables.eml")
      if readerStress {
        try store.engine.seedDemo()
        let html = String(repeating: "<table><tr><td>", count: 12)
          + "<h2>Nested mail fixture</h2><p>UniqueBodyToken</p><p><a href='https://example.com'>Reference</a></p><blockquote>Earlier message</blockquote><ul><li>One</li><li>Two</li></ul>"
          + String(repeating: "</td></tr></table>", count: 12)
        try Data("From: Fixture <fixture@example.com>\r\nTo: Reader <reader@example.com>\r\nSubject: Nested table memory regression\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\(html)".utf8).write(to: fixture)
      }
      store.accounts = try store.engine.accounts()
      store.folders = try store.engine.folders(accountId: "")
      let query = MessageQuery(accountId: "", folderId: "", scope: "inbox", search: "",
                               unreadOnly: false, limit: 100, offset: 0)
      store.messages = try store.engine.listMessages(query: query)
      let rows = store.messages
      let accounts = store.accounts
      let transition = CommandLine.arguments.contains("--transitions")
      if transition { store.accounts = []; store.messages = [] }
      print("Memory test: accounts=\(store.accounts.count), rows=\(store.messages.count)")
      fflush(stdout)
      let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1440, height: 920),
                            styleMask: [.titled, .resizable], backing: .buffered, defer: false)
      let host = NSHostingView(rootView: RootView().environmentObject(store))
      window.contentView = host
      var baseline: UInt64 = 0
      for iteration in 0..<(readerStress ? 500 : 300) {
        if readerStress {
          let message = try store.engine.importEml(path: fixture.path, accountId: accounts[0].id)
          store.messages = [message]
          store.selectedID = message.id
          store.body = try store.engine.cachedBody(messageId: message.id)
          guard store.body?.markdown.components(separatedBy: "UniqueBodyToken").count == 2 else {
            throw MailAppError.message("Body was duplicated or truncated")
          }
        }
        if transition {
          if iteration == 10 { store.accounts = accounts }
          if iteration == 30 { store.messages = rows }
          if iteration == 80 { store.showSettings = true }
          if iteration == 130 { store.showSettings = false }
          if iteration > 150 && iteration % 10 == 0 {
            store.messages = iteration % 20 == 0 ? [] : rows
          }
        }
        autoreleasepool {
          host.layoutSubtreeIfNeeded()
          RunLoop.current.run(until: Date().addingTimeInterval(0.02))
        }
        if iteration % 50 == 0 {
          print("Memory test: layout iteration=\(iteration), syncing=\(store.syncing.count), errors=\(store.syncErrors.count)")
          print("Memory test: footprint_mib=\(Double(footprint()) / 1048576)")
          fflush(stdout)
        }
        if iteration == 50 { baseline = footprint() }
      }
      let final = footprint()
      print("Memory test: final_footprint_mib=\(Double(final)/1048576), growth_after_warmup_mib=\(Double(Int64(final)-Int64(baseline))/1048576)")
      guard final < 192 * 1048576, final < baseline + 32 * 1048576 else {
        throw MailAppError.message("Memory regression exceeded budget")
      }
      print("Memory test: offscreen layout completed")
      func countCells(_ view: NSView) -> Int {
        (view is MailCell ? 1 : 0) + view.subviews.reduce(0) { $0 + countCells($1) }
      }
      print("Memory test: instantiated mail cells=\(countCells(host))")
      window.contentView = nil
    } catch {
      print("Memory test failed: \(userError(error))")
      exit(1)
    }
  }
  static func footprint() -> UInt64 {
    var info = task_vm_info_data_t()
    var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
    let status = withUnsafeMutablePointer(to: &info) {
      $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
        task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
      }
    }
    return status == KERN_SUCCESS ? info.phys_footprint : 0
  }
}
