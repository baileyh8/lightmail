import AppKit
import CryptoKit
import Security
import SwiftUI

enum MailAppError: LocalizedError {
  case message(String)
  var errorDescription: String? {
    if case .message(let message) = self { return message }
    return nil
  }
}
enum Theme {
  static let accent = Color(hex: "226451")
  static let selected = Color(hex: "EAF2ED")
  static let sidebar = Color(hex: "F5F6F5")
  static let ink = Color(hex: "202724")
  static let muted = Color(hex: "69736E")
  static let line = Color(hex: "E7EBE8")
}
extension Color {
  init(hex: String) {
    let v = UInt64(hex.replacingOccurrences(of: "#", with: ""), radix: 16) ?? 0x226451
    self.init(
      red: Double((v >> 16) & 255) / 255, green: Double((v >> 8) & 255) / 255,
      blue: Double(v & 255) / 255)
  }
}
extension Account: Identifiable {}
extension Folder: Identifiable {}
extension MessageSummary: Identifiable {}
extension Draft: Identifiable {}
extension AttachmentInfo: Identifiable { public var id: String { partId } }
extension MessageSummary {
  var sender: String { fromName.isEmpty ? fromAddress : fromName }
  var date: Date { Date(timeIntervalSince1970: Double(timestamp)) }
  var shortDate: String {
    Calendar.current.isDateInToday(date)
      ? date.formatted(date: .omitted, time: .shortened) : date.formatted(.dateTime.month().day())
  }
}
func digest(_ text: String) -> String {
  SHA256.hash(data: Data(text.utf8)).map { String(format: "%02x", $0) }.joined()
}
func humanSize(_ bytes: UInt64) -> String {
  ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
}
func userError(_ error: Error) -> String {
  if let e = error as? MailError, case .Failure(let message) = e { return message }
  if error is CancellationError { return "已取消" }
  return error.localizedDescription
}


struct MailScope: Equatable {
  var title = "全部收件箱"
  var role = "inbox"
  var accountID = ""
  var folderID = ""
  static let inbox = MailScope()
  static let sent = MailScope(title: "已发送", role: "sent")
  static let outbox = MailScope(title: "待发送", role: "outbox")
  static let drafts = MailScope(title: "草稿", role: "drafts")
  static let starred = MailScope(title: "星标", role: "starred")
  var isDraftList: Bool { role == "drafts" || role == "outbox" }
}
enum ReadingMode: String, CaseIterable {
  case original = "原文"
  case translated = "译文"
  case bilingual = "双语"
}
extension TranslationConfiguration: Identifiable {
  init() {
    self.init(id: UUID().uuidString, name: "我的翻译服务", baseUrl: "https://api.openai.com/v1",
      model: "", targetLanguage: "简体中文", stream: true, outputFormat: "prompt",
      inputCharacters: 12000, glossary: "", engine: "llm")
  }
  var baseURL: String { get { baseUrl } set { baseUrl = newValue } }
}
extension TranslationResult {
  init(subject: String, blocks: [TranslationBlock], sourceHash: String, model: String) {
    self.init(subject: subject, blocks: blocks, sourceHash: sourceHash, model: model,
      inputTokens: nil, outputTokens: nil)
  }
  var markdown: String { translationMarkdown(result: self) }
}
struct OAuthSettings: Codable { var clientID = "" }

// Plain buttons otherwise hit-test only their visible text/image on macOS.
// Keep the existing appearance while including the label's padding and gaps.
struct AreaButtonStyle: ButtonStyle {
  @Environment(\.isEnabled) private var isEnabled

  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .frame(minWidth: 28, minHeight: 28)
      .contentShape(Rectangle())
      .opacity(isEnabled ? (configuration.isPressed ? 0.65 : 1) : 0.4)
  }
}

struct AreaDisclosureStyle: DisclosureGroupStyle {
  func makeBody(configuration: Configuration) -> some View {
    VStack(alignment: .leading, spacing: 0) {
      Button { configuration.isExpanded.toggle() } label: {
        HStack(spacing: 6) {
          Image(systemName: configuration.isExpanded ? "chevron.down" : "chevron.right")
            .font(.system(size: 10, weight: .medium)).foregroundStyle(.secondary).frame(width: 12)
          configuration.label
          Spacer(minLength: 0)
        }.frame(maxWidth: .infinity, minHeight: 28, alignment: .leading)
          .contentShape(Rectangle())
      }.buttonStyle(AreaButtonStyle())
        .accessibilityValue(configuration.isExpanded ? "已展开" : "已收起")
      if configuration.isExpanded { configuration.content }
    }
  }
}

struct OutlineButtonStyle: ButtonStyle {
  var prominent = false
  func makeBody(configuration: Configuration) -> some View {
    configuration.label.font(.system(size: 13, weight: .medium)).padding(.horizontal, 12).padding(
      .vertical, 8
    )
    .foregroundStyle(prominent ? Color.white : Theme.ink)
    .background(
      prominent
        ? Theme.accent.opacity(configuration.isPressed ? 0.75 : 1)
        : Color.white.opacity(configuration.isPressed ? 0.55 : 0.85)
    )
    .clipShape(RoundedRectangle(cornerRadius: 7))
    .overlay(
      RoundedRectangle(cornerRadius: 7).stroke(prominent ? Color.clear : Theme.line, lineWidth: 1))
    .contentShape(Rectangle())
  }
}
struct IconButton: View {
  let symbol: String
  let help: String
  var action: () -> Void
  var body: some View {
    Button(action: action) {
      Image(systemName: symbol).font(.system(size: 16)).frame(width: 32, height: 32).contentShape(
        Rectangle())
    }.buttonStyle(AreaButtonStyle()).foregroundStyle(Theme.muted).help(help).accessibilityLabel(help)
  }
}
