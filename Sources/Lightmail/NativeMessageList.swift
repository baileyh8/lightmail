import AppKit
import SwiftUI

struct NativeMessageList: NSViewRepresentable {
  var messages: [MessageSummary]
  var accounts: [Account]
  var selection: String?
  var onSelect: (String?) -> Void
  var onOpen: () -> Void
  func makeCoordinator() -> Coordinator { Coordinator(self) }
  func makeNSView(context: Context) -> NSScrollView {
    let scroll = NSScrollView()
    scroll.hasVerticalScroller = true
    scroll.autohidesScrollers = true
    scroll.drawsBackground = false
    let table = NSTableView()
    let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("message"))
    table.addTableColumn(column)
    table.headerView = nil
    table.rowHeight = 94
    table.intercellSpacing = .zero
    table.style = .plain
    table.selectionHighlightStyle = .regular
    table.backgroundColor = NSColor(Color(hex: "FCFCFB"))
    table.delegate = context.coordinator
    table.dataSource = context.coordinator
    table.target = context.coordinator
    table.doubleAction = #selector(Coordinator.open)
    table.allowsEmptySelection = true
    table.allowsMultipleSelection = false
    table.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
    scroll.documentView = table
    context.coordinator.table = table
    return scroll
  }
  func updateNSView(_ nsView: NSScrollView, context: Context) {
    let c = context.coordinator
    let old = c.parent
    c.parent = self
    guard let table = c.table else { return }
    c.updating = true
    defer { c.updating = false }
    if old.messages != messages || old.accounts != accounts {
      let origin = nsView.contentView.bounds.origin
      table.reloadData()
      nsView.contentView.scroll(to: origin)
      nsView.reflectScrolledClipView(nsView.contentView)
    }
    if let selection, let row = messages.firstIndex(where: { $0.id == selection }) {
      if table.selectedRow != row {
        table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
      }
    } else {
      table.deselectAll(nil)
    }
  }
  final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
    var parent: NativeMessageList
    weak var table: NSTableView?
    var updating = false
    init(_ parent: NativeMessageList) { self.parent = parent }
    func numberOfRows(in tableView: NSTableView) -> Int { parent.messages.count }
    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int)
      -> NSView?
    {
      let id = NSUserInterfaceItemIdentifier("MailRow")
      let cell = (tableView.makeView(withIdentifier: id, owner: self) as? MailCell) ?? MailCell()
      cell.identifier = id
      let m = parent.messages[row]
      cell.configure(m, account: parent.accounts.first { $0.id == m.accountId })
      return cell
    }
    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
      MailRowBackground()
    }
    func tableViewSelectionDidChange(_ notification: Notification) {
      guard !updating, let table else { return }
      let index = table.selectedRow
      parent.onSelect(index >= 0 && index < parent.messages.count ? parent.messages[index].id : nil)
    }
    @objc func open() { parent.onOpen() }
  }
}
final class MailRowBackground: NSTableRowView {
  override func drawSelection(in dirtyRect: NSRect) {
    NSColor(Color(hex: "EAF2ED")).setFill()
    NSBezierPath(roundedRect: bounds.insetBy(dx: 5, dy: 3), xRadius: 7, yRadius: 7).fill()
  }
  override var interiorBackgroundStyle: NSView.BackgroundStyle { .normal }
}
final class MailCell: NSTableCellView {
  let sender = NSTextField(labelWithString: "")
  let subject = NSTextField(labelWithString: "")
  let preview = NSTextField(labelWithString: "")
  let time = NSTextField(labelWithString: "")
  let account = NSTextField(labelWithString: "")
  let dot = NSView()
  let line = NSView()
  override init(frame: NSRect) {
    super.init(frame: frame)
    for v in [sender, subject, preview, time, account, dot, line] {
      addSubview(v)
      v.translatesAutoresizingMaskIntoConstraints = false
    }
    sender.lineBreakMode = .byTruncatingTail
    subject.lineBreakMode = .byTruncatingTail
    preview.lineBreakMode = .byTruncatingTail
    account.lineBreakMode = .byTruncatingTail
    sender.maximumNumberOfLines = 1
    subject.maximumNumberOfLines = 1
    preview.maximumNumberOfLines = 1
    time.font = .systemFont(ofSize: 11)
    time.textColor = .secondaryLabelColor
    preview.font = .systemFont(ofSize: 12)
    preview.textColor = .secondaryLabelColor
    account.font = .systemFont(ofSize: 10)
    account.textColor = NSColor(Color(hex: "69736E"))
    account.alignment = .right
    dot.wantsLayer = true
    dot.layer?.cornerRadius = 3
    line.wantsLayer = true
    line.layer?.backgroundColor = NSColor(Color(hex: "E7EBE8")).cgColor
    NSLayoutConstraint.activate([
      dot.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 17),
      dot.topAnchor.constraint(equalTo: topAnchor, constant: 21),
      dot.widthAnchor.constraint(equalToConstant: 6),
      dot.heightAnchor.constraint(equalToConstant: 6),
      sender.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 32),
      sender.topAnchor.constraint(equalTo: topAnchor, constant: 13),
      sender.trailingAnchor.constraint(lessThanOrEqualTo: time.leadingAnchor, constant: -10),
      time.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -17),
      time.topAnchor.constraint(equalTo: topAnchor, constant: 15),
      time.widthAnchor.constraint(equalToConstant: 48),
      subject.leadingAnchor.constraint(equalTo: sender.leadingAnchor),
      subject.topAnchor.constraint(equalTo: sender.bottomAnchor, constant: 6),
      subject.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -18),
      preview.leadingAnchor.constraint(equalTo: sender.leadingAnchor),
      preview.topAnchor.constraint(equalTo: subject.bottomAnchor, constant: 5),
      preview.trailingAnchor.constraint(lessThanOrEqualTo: account.leadingAnchor, constant: -8),
      account.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -17),
      account.centerYAnchor.constraint(equalTo: preview.centerYAnchor),
      account.widthAnchor.constraint(equalToConstant: 70),
      line.leadingAnchor.constraint(equalTo: sender.leadingAnchor),
      line.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
      line.bottomAnchor.constraint(equalTo: bottomAnchor),
      line.heightAnchor.constraint(equalToConstant: 0.5),
    ])
  }
  required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
  func configure(_ m: MessageSummary, account a: Account?) {
    sender.stringValue = m.sender
    sender.font = .systemFont(ofSize: 13, weight: m.unread ? .semibold : .medium)
    subject.stringValue = m.subject
    subject.font = .systemFont(ofSize: 12, weight: m.unread ? .medium : .regular)
    preview.stringValue = m.snippet.isEmpty ? "正文将在打开时加载" : m.snippet
    time.stringValue = m.shortDate
    time.alignment = .right
    account.stringValue = a?.name ?? ""
    dot.layer?.backgroundColor = NSColor(Color(hex: a?.color ?? "226451")).cgColor
    dot.isHidden = !m.unread
    setAccessibilityLabel("\(m.unread ? "未读，":"")\(m.sender)，\(m.subject)，\(a?.name ?? "")")
  }
}
