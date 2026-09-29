import SwiftUI
import UniformTypeIdentifiers

struct ComposeView: View {
  @EnvironmentObject var store: MailStore
  @Environment(\.dismiss) private var dismiss
  @State var draft: Draft
  @State private var saving: Task<Void, Never>?
  @State private var submitting = false
  @State private var extraRecipients = false
  @State private var saved = false
  private var changeKey: String {
    [
      draft.accountId, draft.to, draft.cc, draft.bcc, draft.subject, draft.body,
      draft.attachmentPaths.joined(separator: "\n"),
    ].joined(separator: "\u{0}")
  }
  var body: some View {
    VStack(spacing: 0) {
      HStack {
        Text(draft.replyToMessageId.isEmpty ? "写邮件" : "回复邮件").font(
          .system(size: 21, weight: .semibold))
        Spacer()
        Text(saved ? "草稿已保存" : "编辑中").font(.system(size: 11)).foregroundStyle(Theme.muted)
        Button("关闭") {
          Task {
            await store.saveDraft(draft)
            dismiss()
          }
        }.buttonStyle(OutlineButtonStyle())
      }.padding(24)
      Divider()
      VStack(spacing: 0) {
        HStack {
          Text("发件人").frame(width: 60, alignment: .leading).foregroundStyle(Theme.muted)
          Picker("发件账号", selection: $draft.accountId) {
            ForEach(store.accounts) { a in Text("\(a.name) <\(a.address)>").tag(a.id) }
          }.labelsHidden()
          Spacer()
        }.font(.system(size: 12)).padding(.vertical, 12)
        Divider()
        recipient("收件人", text: $draft.to) {
          Button(extraRecipients ? "收起" : "抄送 / 密送") { extraRecipients.toggle() }.buttonStyle(
            .plain
          ).font(.system(size: 11)).foregroundStyle(Theme.muted)
        }
        if extraRecipients || !draft.cc.isEmpty || !draft.bcc.isEmpty {
          recipient("抄送", text: $draft.cc) { EmptyView() }
          recipient("密送", text: $draft.bcc) { EmptyView() }
        }
        HStack {
          Text("主题").frame(width: 60, alignment: .leading).foregroundStyle(Theme.muted)
          TextField("邮件主题", text: $draft.subject).textFieldStyle(.plain)
        }.font(.system(size: 13)).padding(.vertical, 14)
        Divider()
      }.padding(.horizontal, 26)
      TextEditor(text: $draft.body).font(.system(size: 15)).lineSpacing(7).scrollContentBackground(
        .hidden
      ).padding(.horizontal, 22).padding(.vertical, 18).frame(minHeight: 300)
      if !draft.attachmentPaths.isEmpty {
        ScrollView(.horizontal) {
          HStack {
            ForEach(draft.attachmentPaths, id: \.self) { path in
              HStack(spacing: 7) {
                Image(systemName: "paperclip")
                Text(URL(fileURLWithPath: path).lastPathComponent).lineLimit(1)
                Button {
                  draft.attachmentPaths.removeAll { $0 == path }
                } label: {
                  Image(systemName: "xmark.circle.fill")
                }.buttonStyle(.plain)
              }.font(.system(size: 11)).padding(9).background(
                Theme.sidebar, in: RoundedRectangle(cornerRadius: 6))
            }
          }
        }.padding(.horizontal, 26).padding(.bottom, 12)
      }
      Divider()
      HStack(spacing: 16) {
        Button {
          Task {
            submitting = true
            saving?.cancel()
            if await store.queue(draft) { dismiss() } else { submitting = false }
          }
        } label: {
          Label("发送", systemImage: "paperplane.fill").padding(.horizontal, 10)
        }.buttonStyle(OutlineButtonStyle(prominent: true)).disabled(submitting)
        Button {
          attach()
        } label: {
          Label("添加附件", systemImage: "paperclip")
        }.buttonStyle(.plain).font(.system(size: 12))
        Spacer()
        Text("发送后有 5 秒撤销时间").font(.system(size: 11)).foregroundStyle(Theme.muted)
      }.padding(23)
    }.frame(width: 780, height: 670).background(Color.white).onChange(of: changeKey) { _, _ in
      scheduleSave()
    }.onAppear {
      if draft.status != "draft" {
        draft.status = "draft"
        draft.lastError = ""
      }
      Task {
        await store.saveDraft(draft)
        saved = true
      }
    }.onDisappear {
      saving?.cancel()
      if !submitting { Task { await store.saveDraft(draft) } }
    }
  }
  func recipient<T: View>(_ name: String, text: Binding<String>, @ViewBuilder trailing: () -> T)
    -> some View
  {
    VStack(spacing: 0) {
      HStack {
        Text(name).frame(width: 60, alignment: .leading).foregroundStyle(Theme.muted)
        TextField("多个地址用逗号分隔", text: text).textFieldStyle(.plain)
        trailing()
      }.font(.system(size: 12)).padding(.vertical, 13)
      Divider()
    }
  }
  func scheduleSave() {
    saved = false
    saving?.cancel()
    let snapshot = draft
    saving = Task {
      try? await Task.sleep(for: .milliseconds(600))
      guard !Task.isCancelled, !submitting else { return }
      await store.saveDraft(snapshot)
      saved = true
    }
  }
  func attach() {
    let panel = NSOpenPanel()
    panel.allowsMultipleSelection = true
    panel.canChooseDirectories = false
    if panel.runModal() == .OK {
      for url in panel.urls where !draft.attachmentPaths.contains(url.path) {
        draft.attachmentPaths.append(url.path)
      }
    }
  }
}
