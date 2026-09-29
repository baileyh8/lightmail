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
struct TranslationConfiguration: Codable, Identifiable, Equatable {
  var id = UUID().uuidString
  var name = "我的翻译服务"
  var baseURL = "https://api.openai.com/v1"
  var model = ""
  var targetLanguage = "简体中文"
  var stream = true
  var outputFormat = "prompt"
  var inputCharacters = 12000
  var glossary = ""
  var engine = "llm"
}
struct TranslationBlock: Codable, Equatable {
  var id: Int
  var text: String
}
struct TranslationResult: Codable, Equatable {
  var subject: String
  var blocks: [TranslationBlock]
  var sourceHash: String
  var model: String
  var inputTokens: Int?
  var outputTokens: Int?
  var markdown: String { blocks.map(\.text).joined(separator: "\n\n") }
}
struct OAuthSettings: Codable { var clientID = "" }

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
  }
}
struct IconButton: View {
  let symbol: String
  let help: String
  var action: () -> Void
  var body: some View {
    Button(action: action) {
      Image(systemName: symbol).font(.system(size: 16)).frame(width: 28, height: 28).contentShape(
        Rectangle())
    }.buttonStyle(.plain).foregroundStyle(Theme.muted).help(help).accessibilityLabel(help)
  }
}
