import SwiftUI
import Translation

struct SystemLanguagesView: View {
  @Environment(\.dismiss) private var dismiss
  let targetCode: String
  @State private var source = "en"
  @State private var target = "zh-Hans"
  @State private var languages: [String] = ["en", "zh-Hans", "zh-Hant", "ja"]
  @State private var configuration = TranslationSession.Configuration()
  @State private var requested = false
  @State private var generation = UUID()
  @State private var status = "选择需要翻译的两种语言。"
  @State private var busy = false

  var body: some View {
    VStack(alignment: .leading, spacing: 20) {
      HStack {
        Text("系统翻译语言包").font(.system(size: 23, weight: .semibold))
        Spacer()
        Button("完成") { dismiss() }.keyboardShortcut(.cancelAction)
      }
      Text("下载后可离线翻译。关闭系统提示后，可以随时从这里重新打开。")
        .foregroundStyle(Theme.muted)
      Picker("原文语言", selection: $source) {
        ForEach(languages, id: \.self) { Text(name($0)).tag($0) }
      }
      Picker("目标语言", selection: $target) {
        ForEach(languages, id: \.self) { Text(name($0)).tag($0) }
      }
      HStack {
        Button(busy ? "重新打开下载提示" : "下载语言包") {
          generation = UUID()
          requested = true
          busy = true
          status = "等待系统语言下载提示…"
          configuration.source = Locale.Language(identifier: source)
          configuration.target = Locale.Language(identifier: target)
          configuration.invalidate()
        }.buttonStyle(OutlineButtonStyle(prominent: true)).disabled(source == target)
        Button("检查下载状态") { Task { await checkStatus() } }.buttonStyle(OutlineButtonStyle())
      }
      Text(status).foregroundStyle(Theme.muted).frame(minHeight: 36, alignment: .topLeading)
      Text("也可在 macOS「系统设置 → 通用 → 语言与地区 → 翻译语言」管理已下载语言。")
        .font(.system(size: 11)).foregroundStyle(Theme.muted)
    }.padding(28).frame(width: 500)
      .task {
        target = targetCode
        if source == target { source = target == "en" ? "zh-Hans" : "en" }
        let supported = await LanguageAvailability().supportedLanguages.map { $0.minimalIdentifier }
        languages = Array(Set(supported + [source, target])).sorted { name($0) < name($1) }
        await checkStatus()
      }
      .translationTask(configuration) { session in
        guard requested else { return }
        let request = generation
        do {
          try await session.prepareTranslation()
          guard request == generation else { return }
          await checkStatus()
        } catch {
          guard request == generation else { return }
          status = "语言包尚未就绪。可再次点击下载：\(userError(error))"
        }
        guard request == generation else { return }
        busy = false
      }
  }
  func name(_ code: String) -> String { Locale.current.localizedString(forIdentifier: code) ?? code }
  func checkStatus() async {
    let from = source, to = target
    let value = await LanguageAvailability().status(from: .init(identifier: from), to: .init(identifier: to))
    guard from == source, to == target else { return }
    switch value {
    case .installed: status = "语言包已就绪，可以返回邮件点击「重译」。"
    case .supported: status = "语言包尚未安装完成。点击下载；已开始下载时可稍后检查状态。"
    case .unsupported: status = "系统暂不支持这组语言，请选择其他语言或使用 Chat 格式 LLM。"
    @unknown default: status = "暂时无法确认语言包状态，请重试。"
    }
  }
}
