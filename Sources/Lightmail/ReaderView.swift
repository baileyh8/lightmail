import SwiftUI
import WebKit

struct ReaderView: View {
  @EnvironmentObject var store: MailStore
  @State private var htmlLayout = true
  @State private var showLanguages = false
  @State private var loadImages = false
  var body: some View {
    VStack(spacing: 0) {
      toolbar.frame(height: 55).padding(.horizontal, 24)
      Divider().overlay(Theme.line)
      if let message = store.selectedMessage {
        if htmlLayout && store.readingMode == .original, let body = store.body, !body.html.isEmpty {
          // WebKit owns this viewport. Do not place a scrollable web view inside
          // another vertical ScrollView: it consumes wheel events at its edges.
          VStack(alignment: .leading, spacing: 0) {
            messageHeader(message).padding(.horizontal, 36).padding(.top, 24)
            if body.html.localizedCaseInsensitiveContains("<img") {
              HStack {
                Text(loadImages ? "已显示本封邮件的远程图片" : "远程图片未加载").foregroundStyle(Theme.muted)
                Button(loadImages ? "隐藏图片" : "显示图片") { loadImages.toggle() }
                  .buttonStyle(.plain).foregroundStyle(Theme.accent)
              }.font(.system(size: 11)).padding(.horizontal, 36).padding(.bottom, 12)
            }
            SafeHTMLView(html: body.html, loadImages: loadImages).frame(maxWidth: .infinity, maxHeight: .infinity)
              .padding(.horizontal, 36)
            HStack(spacing: 12) {
              replyActions
              if !body.attachments.isEmpty {
                Menu("附件（\(body.attachments.count)）") {
                  ForEach(body.attachments) { attachment in
                    Button(attachment.filename) { store.download(attachment) }
                  }
                }.menuStyle(.borderlessButton).fixedSize()
              }
              Spacer()
            }.padding(.horizontal, 36).padding(.vertical, 18)
          }.id(message.id)
        } else {
          ScrollView {
            VStack(alignment: .leading, spacing: 0) {
              messageHeader(message)
            if store.bodyLoading {
              HStack {
                ProgressView().controlSize(.small)
                Text("正在读取正文…").font(.system(size: 12)).foregroundStyle(Theme.muted)
              }.padding(.vertical, 18)
            }
            if let error = store.bodyError {
              VStack(alignment: .leading, spacing: 12) {
                Text(error).foregroundStyle(Theme.muted)
                Button("重新读取") { Task { await store.loadSelectedBody() } }.buttonStyle(
                  OutlineButtonStyle())
              }.padding(.vertical, 20)
            }
              if let body = store.body {
                if store.readingMode != .original { translationStatus.padding(.bottom, 22) }
                if store.readingMode == .original { MarkdownBody(markdown: body.markdown) }
                else if store.readingMode == .bilingual { bilingual(body) }
                else if let translation = store.translation { MarkdownBody(markdown: translation.markdown) }
                else if !store.partialTranslation.isEmpty {
                  MarkdownBody(markdown: store.partialTranslation.map(\.text).joined(separator: "\n\n"))
                }
                messageFooter(body)
              }
              Spacer(minLength: 32)
            }.padding(.horizontal, store.focusReading ? 32 : 36).padding(.top, 29)
              .frame(maxWidth: .infinity, alignment: .leading)
              .background(VerticalScrollBridge())
          }.id(message.id)
        }
      } else {
        VStack(spacing: 14) {
          Image(systemName: "envelope.open").font(.system(size: 45, weight: .ultraLight))
            .foregroundStyle(Theme.accent.opacity(0.65))
          Text("选一封邮件，慢慢读。").font(.system(size: 18, weight: .medium))
          Text("全文翻译与 Markdown 复制，随时在手边。").font(.system(size: 12)).foregroundStyle(Theme.muted)
        }.frame(maxWidth: .infinity, maxHeight: .infinity)
      }
    }.background(Color.white).onChange(of: store.selectedID) { _, _ in htmlLayout = true; loadImages = false }
      .sheet(isPresented: $showLanguages) {
        SystemLanguagesView(targetCode: store.languageCode(store.translationConfig?.targetLanguage ?? "简体中文"))
      }
  }
  func messageHeader(_ message: MessageSummary) -> some View {
    VStack(alignment: .leading, spacing: 0) {
            Text(message.subject).font(.system(size: 27, weight: .semibold)).foregroundStyle(
              Theme.ink
            ).textSelection(.enabled).padding(.bottom, 8)
            if store.readingMode != .original, let translation = store.translation {
              Text(translation.subject).font(.system(size: 21, weight: .medium)).foregroundStyle(
                Theme.accent
              ).textSelection(.enabled).padding(.bottom, 9)
            }
            if let account = store.selectedAccount {
              Text(account.name).font(.system(size: 11)).foregroundStyle(Theme.accent).padding(
                .horizontal, 7
              ).padding(.vertical, 3).background(
                Theme.selected, in: RoundedRectangle(cornerRadius: 4)
              ).padding(.bottom, 18)
            }
            HStack(alignment: .top) {
              VStack(alignment: .leading, spacing: 7) {
                HStack(spacing: 7) {
                  Text(message.sender).font(.system(size: 14, weight: .semibold))
                  Text("<\(message.fromAddress)>").font(.system(size: 12)).foregroundStyle(
                    Theme.muted)
                }
                Text("收件人：\(message.toAddresses)").font(.system(size: 12)).foregroundStyle(
                  Theme.muted)
                if !message.ccAddresses.isEmpty {
                  Text("抄送：\(message.ccAddresses)").font(.system(size: 11)).foregroundStyle(
                    Theme.muted)
                }
              }
              Spacer()
              Text(message.date.formatted(date: .abbreviated, time: .shortened)).font(
                .system(size: 11)
              ).foregroundStyle(Theme.muted)
            }.textSelection(.enabled)
            Divider().padding(.vertical, 24)
    }
  }
  func messageFooter(_ body: MailBody) -> some View {
    VStack(alignment: .leading, spacing: 0) {
              if !body.attachments.isEmpty {
                FlowAttachments(attachments: body.attachments, download: store.download).padding(
                  .top, 30)
              }
      replyActions.padding(.top, 30)
    }
  }
  var replyActions: some View {
              HStack(spacing: 10) {
                Button {
                  store.newDraft(reply: "reply")
                } label: {
                  Label("回复", systemImage: "arrowshape.turn.up.left")
                }.buttonStyle(OutlineButtonStyle())
                Menu {
                  Button("回复全部") { store.newDraft(reply: "all") }
                  Button("转发") { store.newDraft(reply: "forward") }
                } label: {
                  Label("更多回复", systemImage: "arrowshape.turn.up.right")
                }.menuStyle(.borderlessButton).fixedSize().padding(8)
              }
  }
  var toolbar: some View {
    HStack(spacing: 14) {
      if store.focusReading {
        Button {
          store.focusReading = false
        } label: {
          Label("返回列表", systemImage: "chevron.left").font(.system(size: 12))
        }.buttonStyle(.plain).foregroundStyle(Theme.ink)
        Divider().frame(height: 20)
      }
      IconButton(symbol: "archivebox", help: "归档") { store.moveSelected(to: "archive") }.disabled(
        store.selectedMessage == nil)
      IconButton(symbol: "trash", help: "移到垃圾箱") { store.moveSelected(to: "trash") }.disabled(
        store.selectedMessage == nil)
      Menu {
        if let m = store.selectedMessage {
          Button(m.starred ? "取消星标" : "加星标") {
            Task { await store.mark(m, flag: "starred", value: !m.starred) }
          }
          Button(m.unread ? "标记已读" : "标记未读") {
            Task { await store.mark(m, flag: "seen", value: m.unread) }
          }
        }
        if !(store.body?.html.isEmpty ?? true) {
          Button(htmlLayout ? "清爽阅读" : "查看原始排版") {
            htmlLayout.toggle()
            store.readingMode = .original
          }
        }
        Button("系统翻译语言包…") { showLanguages = true }
        Button("回复全部") { store.newDraft(reply: "all") }
        Button("转发") { store.newDraft(reply: "forward") }
      } label: {
        Image(systemName: "ellipsis").font(.system(size: 19)).foregroundStyle(Theme.muted)
      }.menuStyle(.borderlessButton).fixedSize().disabled(store.selectedMessage == nil)
      Spacer(minLength: 12)
      if store.focusReading {
        Picker("阅读模式", selection: $store.readingMode) {
          ForEach(ReadingMode.allCases, id: \.self) { Text($0.rawValue).tag($0) }
        }.pickerStyle(.segmented).labelsHidden().frame(width: 164).onChange(of: store.readingMode) {
          _, mode in
          if mode != .original && store.translation == nil && !store.translating {
            store.beginTranslation()
          }
        }
      } else {
        Button {
          store.beginTranslation()
        } label: {
          Label("全文翻译", systemImage: "translate")
        }.buttonStyle(OutlineButtonStyle()).disabled(store.body == nil)
      }
      Menu {
        Button("复制当前视图") { store.copyMarkdown() }
        Divider()
        Button("原文 Markdown") { store.copyMarkdown(mode: .original) }
        Button("译文 Markdown") { store.copyMarkdown(mode: .translated) }.disabled(
          store.translation == nil)
        Button("双语 Markdown") { store.copyMarkdown(mode: .bilingual) }.disabled(
          store.translation == nil)
      } label: {
        Label("复制 Markdown", systemImage: "doc.on.doc").font(.system(size: 12))
      } primaryAction: {
        store.copyMarkdown()
      }.menuStyle(.borderlessButton).fixedSize().disabled(store.body == nil)
    }
  }
  var translationStatus: some View {
    VStack(alignment: .leading, spacing: 12) {
      HStack(spacing: 9) {
        if store.translating {
          ProgressView().controlSize(.small)
        } else {
          Image(
            systemName: store.translation != nil
              ? "checkmark.circle.fill" : "exclamationmark.circle"
          ).foregroundStyle(store.translation != nil ? Theme.accent : .orange)
        }
        Text(store.translation != nil ? "全文已翻译" : store.translationProgress).font(
          .system(size: 12, weight: .medium)
        ).foregroundStyle(Theme.accent)
        if let config = store.translationConfig {
          Text("→ \(config.targetLanguage)").font(.system(size: 11)).foregroundStyle(Theme.muted)
          Spacer()
          Menu {
            ForEach(store.translationConfigs) { c in
              Button("\(c.name) · \(c.model)") {
                store.selectedTranslationID = c.id
                store.translation = nil
                Task {
                  do {
                    try await store.persistTranslationSettings()
                    store.beginTranslation(force: true)
                  } catch { store.errorMessage = userError(error) }
                }
              }
            }
            Divider()
            Button("翻译设置…") {
              store.settingsTab = "translation"
              store.showSettings = true
            }
          } label: {
            Text(config.engine == "system" ? "系统翻译" : config.model).font(.system(size: 10))
              .lineLimit(1)
          }.menuStyle(.borderlessButton).fixedSize()
        }
        if store.translationConfig?.engine == "system" {
          Button("语言包…") { store.cancelTranslation(); showLanguages = true }
            .buttonStyle(.plain).font(.system(size: 11)).foregroundStyle(Theme.accent)
        }
        if store.translating {
          Button("取消") {
            store.cancelTranslation()
            store.translationProgress = "翻译已取消"
          }.buttonStyle(.plain).font(.system(size: 11))
        } else {
          Button("重译") { store.beginTranslation(force: true) }.buttonStyle(.plain).font(
            .system(size: 11)
          ).foregroundStyle(Theme.muted)
        }
      }
      if let error = store.translationError {
        Text(error).font(.system(size: 12)).foregroundStyle(.orange)
      }
      Divider()
    }
  }
  func bilingual(_ body: MailBody) -> some View {
    let source = TranslationService.blocks(body.markdown)
    let translated = store.translation?.blocks ?? store.partialTranslation
    return VStack(alignment: .leading, spacing: 0) {
      HStack {
        Text("原文").frame(maxWidth: .infinity, alignment: .leading)
        Text(store.translationConfig?.targetLanguage ?? "简体中文").frame(
          maxWidth: .infinity, alignment: .leading)
      }.font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.muted).padding(.bottom, 18)
      ForEach(source, id: \.id) { block in
        HStack(alignment: .top, spacing: 24) {
          MarkdownBody(markdown: block.text).frame(maxWidth: .infinity, alignment: .topLeading)
          Rectangle().fill(Theme.line).frame(width: 1)
          if let target = translated.first(where: { $0.id == block.id }) {
            MarkdownBody(markdown: target.text).frame(maxWidth: .infinity, alignment: .topLeading)
          } else {
            Color.clear.frame(maxWidth: .infinity, minHeight: 1)
              .accessibilityHidden(true)
          }
        }.fixedSize(horizontal: false, vertical: true).padding(.bottom, 18)
      }
    }
  }
}

struct FlowAttachments: View {
  let attachments: [AttachmentInfo]
  var download: (AttachmentInfo) -> Void
  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      ForEach(attachments) { attachment in
        HStack(spacing: 10) {
          Image(systemName: "doc").font(.system(size: 22, weight: .light))
          VStack(alignment: .leading, spacing: 4) {
            Text(attachment.filename).font(.system(size: 12, weight: .medium)).lineLimit(1)
            Text(humanSize(attachment.size)).font(.system(size: 10)).foregroundStyle(Theme.muted)
          }
          Spacer()
          Button {
            download(attachment)
          } label: {
            Image(systemName: "arrow.down.to.line").font(.system(size: 16))
          }.buttonStyle(.plain).help("下载附件")
        }.padding(12).frame(maxWidth: 340).background(
          Theme.sidebar, in: RoundedRectangle(cornerRadius: 7)
        ).overlay(RoundedRectangle(cornerRadius: 7).stroke(Theme.line))
      }
    }
  }
}
struct SafeHTMLView: NSViewRepresentable {
  let html: String
  var loadImages = false
  func makeCoordinator() -> Coordinator { Coordinator() }
  func makeNSView(context: Context) -> WKWebView {
    let config = WKWebViewConfiguration()
    config.websiteDataStore = .nonPersistent()
    config.defaultWebpagePreferences.allowsContentJavaScript = false
    let view = WKWebView(frame: .zero, configuration: config)
    view.setValue(false, forKey: "drawsBackground")
    view.navigationDelegate = context.coordinator
    return view
  }
  func updateNSView(_ view: WKWebView, context: Context) {
    guard context.coordinator.last != html || context.coordinator.images != loadImages else { return }
    context.coordinator.last = html
    context.coordinator.images = loadImages
    let imageSources = loadImages ? "https: http:" : "'none'"
    let imageStyle = loadImages
      ? ".lightmail-image-link-label{display:none!important}"
      : "img{display:none!important}.lightmail-image-link-label{display:inline-block!important;padding:10px 16px!important;border:1px solid currentColor!important;border-radius:5px!important;font:14px -apple-system!important;color:#226451!important;background:#f2f7f5!important}"

    let page =
      "<!doctype html><html><head><meta charset='utf-8'><meta http-equiv='Content-Security-Policy' content=\"default-src 'none'; style-src 'unsafe-inline'; img-src \(imageSources); connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'\"><style>:root{color-scheme:light}body{font:15px -apple-system;line-height:1.65;color:#202724;margin:0;overflow-wrap:anywhere}table{max-width:100%}pre{white-space:pre-wrap}a{color:#226451}blockquote{border-left:2px solid #e7ebe8;margin-left:0;padding-left:16px}.lightmail-empty-link-label{display:inline-block!important;padding:10px 16px!important;border:1px solid currentColor!important;border-radius:5px!important;color:#226451!important;background:#f2f7f5!important}\(imageStyle)</style></head><body>\(html)</body></html>"
    view.loadHTMLString(page, baseURL: nil)
  }
  final class Coordinator: NSObject, WKNavigationDelegate {
    var last = ""
    var images = false
    func webView(
      _ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
      decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
    ) {
      if navigationAction.navigationType == .linkActivated {
        if let url = navigationAction.request.url,
          ["https", "http", "mailto"].contains(url.scheme ?? "")
        {
          NSWorkspace.shared.open(url)
        }
        decisionHandler(.cancel)
      } else if navigationAction.request.url?.scheme == "about" {
        decisionHandler(.allow)
      } else {
        decisionHandler(.cancel)
      }
    }
  }
  static func dismantleNSView(_ view: WKWebView, coordinator: Coordinator) {
    view.stopLoading()
    view.navigationDelegate = nil
  }
}
