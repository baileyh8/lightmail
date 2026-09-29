import AppKit
import SwiftUI

final class ApplicationDelegate: NSObject, NSApplicationDelegate {
  func applicationDidFinishLaunching(_ notification: Notification) {
    NSApp.setActivationPolicy(.regular)
    NSApp.activate(ignoringOtherApps: true)
  }
  func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { false }
  func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool
  {
    if !flag { sender.windows.first?.makeKeyAndOrderFront(nil) }
    return true
  }
}

@main struct LightmailApp: App {
  @NSApplicationDelegateAdaptor(ApplicationDelegate.self) var delegate
  @StateObject private var store = MailStore()
  init() {
    MemoryTests.startDiagnosticGuard()
    if let i = CommandLine.arguments.firstIndex(of: "--render-test"), CommandLine.arguments.count > i + 1 {
      ReaderRenderingTests.run(directory: CommandLine.arguments[i + 1])
      exit(0)
    }
    if let i = CommandLine.arguments.firstIndex(of: "--memory-test"),
      CommandLine.arguments.count > i + 1
    {
      MemoryTests.run(directory: CommandLine.arguments[i + 1])
      exit(0)
    }
    if let i = CommandLine.arguments.firstIndex(of: "--integration-test"),
      CommandLine.arguments.count > i + 1
    {
      SelfTests.runIntegration(baseURL: CommandLine.arguments[i + 1])
      exit(0)
    }
    if CommandLine.arguments.contains("--self-test") {
      SelfTests.run()
      exit(0)
    }
  }
  var body: some Scene {
    WindowGroup("轻邮") {
      RootView().environmentObject(store).preferredColorScheme(.light).tint(Theme.accent)
        .onReceive(
          NSWorkspace.shared.notificationCenter.publisher(for: NSWorkspace.didWakeNotification)
        ) { _ in store.refresh() }
    }
    .windowStyle(.hiddenTitleBar)
    .defaultSize(width: 1440, height: 920)
    .commands {
      CommandGroup(replacing: .newItem) {
        Button("写邮件") { store.newDraft() }.keyboardShortcut("n")
        Button("导入 .eml 邮件…") { store.importMessage() }.keyboardShortcut("o")
      }
      CommandGroup(replacing: .appSettings) {
        Button("设置…") { store.showSettings = true }.keyboardShortcut(",")
      }
      CommandMenu("邮件") {
        Button("刷新所有邮箱") { store.refresh() }.keyboardShortcut("r")
        Button("回复") { store.newDraft(reply: "reply") }.keyboardShortcut(
          "r", modifiers: [.command, .shift]
        ).disabled(store.selectedMessage == nil)
        Button("复制 Markdown") { store.copyMarkdown() }.keyboardShortcut(
          "c", modifiers: [.command, .shift]
        ).disabled(store.body == nil)
        Divider()
        Button("全文翻译") { store.beginTranslation() }.keyboardShortcut(
          "t", modifiers: [.command, .shift]
        ).disabled(store.body == nil)
        Button("归档") { store.moveSelected(to: "archive") }.keyboardShortcut(
          "e", modifiers: .command
        ).disabled(store.selectedMessage == nil)
      }
    }
  }
}

struct RootView: View {
  @EnvironmentObject var store: MailStore
  @State private var showSidebar = true
  var body: some View {
    GeometryReader { geometry in
      HStack(spacing: 0) {
        if showSidebar && geometry.size.width >= 1000 {
          SidebarView().frame(width: 218)
          Divider().overlay(Theme.line)
        }
        if store.scope.isDraftList {
          DraftListView().frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if store.accounts.isEmpty {
          WelcomeView().frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
          HSplitView {
            if !store.focusReading {
              MessageListPane().frame(minWidth: 300, idealWidth: 350, maxWidth: 430)
            }
            ReaderView().frame(minWidth: 400, maxWidth: .infinity, maxHeight: .infinity)
          }
        }
      }
      .background(Color.white)
      .overlay(alignment: .bottomTrailing) {
        if let toast = store.toast {
          Label(toast, systemImage: "checkmark.circle.fill").font(.system(size: 13))
            .foregroundStyle(Theme.accent).padding(.horizontal, 18).padding(.vertical, 13)
            .background(Theme.selected, in: RoundedRectangle(cornerRadius: 9)).padding(24)
            .transition(.opacity).allowsHitTesting(false)
        }
      }
      .overlay(alignment: .topLeading) {
        if geometry.size.width < 1000 {
          Menu {
            Button("全部收件箱") { store.setScope(.inbox) }
            Button("已发送") { store.setScope(.sent) }
            Button("草稿") { store.setScope(.drafts) }
            Button("设置…") { store.showSettings = true }
          } label: {
            Image(systemName: "sidebar.left").padding(12)
          }.menuStyle(.borderlessButton).fixedSize().offset(x: 6, y: 4)
        }
      }
    }
    .safeAreaInset(edge: .top, spacing: 0) {
      if !store.credentialRequests.isEmpty {
        CredentialAuthorizationBanner().environmentObject(store)
      }
    }
    .onReceive(NotificationCenter.default.publisher(for: SecretStore.authorizationChanged).receive(on: DispatchQueue.main)) { _ in
      Task { await store.refreshCredentialRequests() }
    }
    .frame(minWidth: 860, minHeight: 600)
    .sheet(isPresented: $store.showSettings) { SettingsView().environmentObject(store) }
    .sheet(item: $store.compose) { draft in ComposeView(draft: draft).environmentObject(store) }
    .alert(
      "轻邮",
      isPresented: Binding(
        get: { store.errorMessage != nil }, set: { if !$0 { store.errorMessage = nil } })
    ) {
      Button("好", role: .cancel) { store.errorMessage = nil }
    } message: {
      Text(store.errorMessage ?? "")
    }
    .environment(
      \.openURL,
      OpenURLAction { url in
        if ["https", "http", "mailto"].contains(url.scheme?.lowercased() ?? "") {
          NSWorkspace.shared.open(url)
        }
        return .handled
      }
    )
    .translationTask(store.systemTranslationConfiguration) { session in
      await store.performSystemTranslation(session)
    }
    .animation(.easeOut(duration: 0.14), value: store.toast)
  }
}

struct SidebarView: View {
  @EnvironmentObject var store: MailStore
  @State private var expanded: Set<String> = []
  private func count(_ account: String? = nil) -> Int {
    store.folders.filter { $0.role == "inbox" && (account == nil || $0.accountId == account) }
      .reduce(0) { $0 + Int($1.unreadCount) }
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack(spacing: 10) {
        Image(systemName: "paperplane").font(.system(size: 25, weight: .light)).foregroundStyle(
          Theme.accent)
        Text("轻邮").font(.system(size: 23, weight: .semibold))
        Spacer()
      }.padding(.horizontal, 22).padding(.top, 30).padding(.bottom, 24)
      Button {
        store.newDraft()
      } label: {
        Label("写邮件", systemImage: "square.and.pencil").frame(maxWidth: .infinity)
      }.buttonStyle(OutlineButtonStyle()).foregroundStyle(Theme.accent).padding(.horizontal, 16)
        .padding(.bottom, 20)
      VStack(spacing: 3) {
        nav("全部收件箱", "tray", .inbox, count: count())
        nav("已发送", "paperplane", .sent)
        nav(
          "待发送", "clock", .outbox,
          count: store.drafts.filter {
            ["queued", "sending", "failed", "delivery_unknown"].contains($0.status)
          }.count)
        nav("草稿", "doc", .drafts, count: store.drafts.filter { $0.status == "draft" }.count)
        nav("星标", "star", .starred)
      }.padding(.horizontal, 10)
      HStack {
        Text("邮箱").font(.system(size: 11, weight: .medium)).foregroundStyle(Theme.muted)
        Spacer()
        Button {
          store.settingsTab = "accounts"
          store.showSettings = true
        } label: {
          Image(systemName: "plus").font(.system(size: 11))
        }.buttonStyle(.plain).help("添加邮箱")
      }.padding(.horizontal, 21).padding(.top, 26).padding(.bottom, 9)
      ScrollView {
        VStack(spacing: 2) {
          ForEach(store.accounts) { account in
            VStack(spacing: 1) {
              HStack(spacing: 9) {
                Circle().fill(Color(hex: account.color)).frame(width: 8, height: 8)
                Button {
                  store.setScope(
                    MailScope(title: account.name, role: "inbox", accountID: account.id))
                } label: {
                  Text(account.name).font(.system(size: 13)).lineLimit(1).frame(
                    maxWidth: .infinity, alignment: .leading)
                }.buttonStyle(.plain)
                if store.syncing.contains(account.id) {
                  ProgressView().controlSize(.mini)
                } else if count(account.id) > 0 {
                  Text(String(count(account.id))).font(.system(size: 12)).foregroundStyle(
                    Theme.muted)
                }
                Button {
                  if expanded.contains(account.id) {
                    expanded.remove(account.id)
                  } else {
                    expanded.insert(account.id)
                  }
                } label: {
                  Image(
                    systemName: expanded.contains(account.id) ? "chevron.down" : "chevron.right"
                  ).font(.system(size: 9))
                }.buttonStyle(.plain).foregroundStyle(Theme.muted)
              }.padding(.horizontal, 12).padding(.vertical, 11).background(
                store.scope.accountID == account.id && store.scope.folderID.isEmpty
                  ? Theme.selected : Color.clear, in: RoundedRectangle(cornerRadius: 7))
              if expanded.contains(account.id) {
                ForEach(store.folders.filter { $0.accountId == account.id }) { folder in
                  Button {
                    store.setScope(
                      MailScope(
                        title: folderTitle(folder), role: "all", accountID: account.id,
                        folderID: folder.id))
                  } label: {
                    HStack(spacing: 9) {
                      Image(systemName: folderIcon(folder)).frame(width: 16)
                      Text(folderTitle(folder)).lineLimit(1)
                      Spacer()
                    }.font(.system(size: 12)).padding(.leading, 29).padding(.trailing, 8).padding(
                      .vertical, 8
                    ).background(
                      store.scope.folderID == folder.id ? Theme.selected : Color.clear,
                      in: RoundedRectangle(cornerRadius: 6))
                  }.buttonStyle(.plain).foregroundStyle(Theme.muted)
                }
              }
            }.help(store.accountStatus[account.id] ?? account.address)
          }
        }.padding(.horizontal, 10)
      }
      Spacer(minLength: 8)
      VStack(alignment: .leading, spacing: 14) {
        if store.demoMode {
          Button {
            Task { await store.toggleDemo() }
          } label: {
            Label("示例模式 · 返回真实邮箱", systemImage: "eye").font(.system(size: 11))
          }.buttonStyle(.plain).foregroundStyle(Theme.accent)
        }
        Button {
          store.showSettings = true
        } label: {
          Label("设置", systemImage: "gearshape").font(.system(size: 13))
        }.buttonStyle(.plain)
        HStack(spacing: 7) {
          Circle().fill(store.syncing.isEmpty ? Theme.accent : Color.orange).frame(
            width: 6, height: 6)
          Text(
            store.demoMode
              ? "仅展示示例数据"
              : (!store.syncing.isEmpty
                ? "正在同步…" : store.accounts.isEmpty ? "添加邮箱后开始收件" : "本地优先 · 按需同步")
          ).font(.system(size: 10)).foregroundStyle(Theme.muted)
        }
      }.padding(.horizontal, 22).padding(.bottom, 20)
    }.foregroundStyle(Theme.ink).background(Theme.sidebar)
      .onChange(of: store.accounts.count) { _, _ in
        if expanded.isEmpty, let first = store.accounts.first { expanded.insert(first.id) }
      }
  }
  func nav(_ title: String, _ icon: String, _ scope: MailScope, count: Int = 0) -> some View {
    Button {
      store.setScope(scope)
    } label: {
      HStack(spacing: 12) {
        Image(systemName: icon).font(.system(size: 17, weight: .regular)).frame(width: 20)
        Text(title).font(.system(size: 13, weight: store.scope == scope ? .medium : .regular))
        Spacer()
        if count > 0 { Text(String(count)).font(.system(size: 12)) }
      }.padding(.horizontal, 12).padding(.vertical, 11).background(
        store.scope == scope ? Theme.selected : Color.clear, in: RoundedRectangle(cornerRadius: 7))
    }.buttonStyle(.plain).foregroundStyle(store.scope == scope ? Theme.accent : Theme.ink)
  }
  func folderTitle(_ f: Folder) -> String {
    [
      "inbox": "收件箱", "sent": "已发送", "drafts": "草稿", "trash": "垃圾箱", "junk": "垃圾邮件",
      "archive": "归档", "allmail": "所有邮件",
    ][f.role] ?? f.name
  }
  func folderIcon(_ f: Folder) -> String {
    [
      "inbox": "tray", "sent": "paperplane", "drafts": "doc", "trash": "trash",
      "junk": "exclamationmark.shield", "archive": "archivebox",
    ][f.role] ?? "folder"
  }
}

struct WelcomeView: View {
  @EnvironmentObject var store: MailStore
  var body: some View {
    VStack(alignment: .leading, spacing: 20) {
      Image(systemName: "paperplane").font(.system(size: 48, weight: .ultraLight)).foregroundStyle(
        Theme.accent
      ).padding(.bottom, 10)
      Text("邮件归于一处，\n专注每一次沟通。").font(.system(size: 34, weight: .semibold)).lineSpacing(8)
      Text("连接 Gmail、163 和 QQ，在你的 Mac 上轻快地收发、阅读与翻译。").font(.system(size: 14)).foregroundStyle(
        Theme.muted
      ).frame(maxWidth: 430, alignment: .leading).lineSpacing(6)
      HStack(spacing: 12) {
        Button {
          store.settingsTab = "accounts"
          store.showSettings = true
        } label: {
          Label("添加第一个邮箱", systemImage: "plus").padding(.horizontal, 6)
        }.buttonStyle(OutlineButtonStyle(prominent: true))
        Button {
          Task { await store.toggleDemo() }
        } label: {
          Text("预览示例界面")
        }.buttonStyle(OutlineButtonStyle())
      }.padding(.top, 9)
      Button("或导入一封 .eml 邮件") { store.importMessage() }.buttonStyle(.plain).font(.system(size: 12))
        .foregroundStyle(Theme.muted)
    }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .center).padding(60)
  }
}

struct MessageListPane: View {
  @EnvironmentObject var store: MailStore
  @FocusState private var searchFocus: Bool
  var body: some View {
    VStack(spacing: 0) {
      HStack {
        Text(store.scope.title).font(.system(size: 21, weight: .semibold)).lineLimit(1)
        Spacer()
        IconButton(symbol: "arrow.clockwise", help: "刷新邮箱 ⌘R") { store.refresh() }
      }.padding(.horizontal, 20).padding(.top, 27).padding(.bottom, 18)
      HStack(spacing: 9) {
        Image(systemName: "magnifyingglass").foregroundStyle(Theme.muted)
        TextField("搜索已同步邮件", text: $store.search).textFieldStyle(.plain).font(.system(size: 12))
          .focused($searchFocus)
        if !store.search.isEmpty {
          Button {
            store.search = ""
          } label: {
            Image(systemName: "xmark.circle.fill").foregroundStyle(Theme.muted)
          }.buttonStyle(.plain)
        } else {
          Text("⌘ F").font(.system(size: 10)).foregroundStyle(Theme.muted)
        }
      }.padding(10).background(Color(hex: "F3F5F3"), in: RoundedRectangle(cornerRadius: 7)).overlay(
        RoundedRectangle(cornerRadius: 7).stroke(Theme.line)
      ).padding(.horizontal, 16)
      HStack(spacing: 24) {
        filter("全部", false)
        filter("未读", true)
        Spacer()
        Text("\(store.messages.count)").font(.system(size: 11)).foregroundStyle(Theme.muted)
      }.padding(.horizontal, 23).padding(.top, 15).padding(.bottom, 11)
      Divider().overlay(Theme.line)
      if store.messages.isEmpty {
        VStack(spacing: 10) {
          Image(systemName: store.search.isEmpty ? "tray" : "magnifyingglass").font(
            .system(size: 30, weight: .ultraLight))
          Text(store.search.isEmpty
            ? (store.visibleSyncError != nil ? "邮件尚未同步" : !store.syncing.isEmpty ? "正在同步邮件…" : "这里还没有邮件")
            : "没有找到匹配的邮件").font(.system(size: 13))
          if let error = store.visibleSyncError, store.search.isEmpty {
            Text(error).font(.system(size: 12)).multilineTextAlignment(.center).padding(.horizontal, 20)
            Button("重新同步") { store.refresh() }.buttonStyle(.plain).foregroundStyle(Theme.accent)
          } else {
            Text("仅搜索已同步的摘要与缓存正文").font(.system(size: 10)).foregroundStyle(Theme.muted)
          }
        }.foregroundStyle(Theme.muted).frame(maxWidth: .infinity, maxHeight: .infinity)
      } else {
        NativeMessageList(
          messages: store.messages, accounts: store.accounts, selection: store.selectedID,
          onSelect: store.select, onOpen: { store.focusReading = true }
        ).frame(maxWidth: .infinity, maxHeight: .infinity)
      }
      HStack {
        Text(store.search.isEmpty ? "最近邮件 · 正文按需读取" : "搜索范围：本地已同步内容").font(.system(size: 10))
          .foregroundStyle(Theme.muted)
        Spacer()
        Button("加载更多") { store.loadOlder() }.buttonStyle(.plain).font(.system(size: 10))
          .foregroundStyle(Theme.accent)
      }.padding(.horizontal, 18).padding(.vertical, 12).background(Color.white)
    }.background(Color(hex: "FCFCFB")).onChange(of: store.search) { _, _ in store.searchChanged() }
      .onChange(of: store.unreadOnly) { _, _ in Task { await store.reloadMessages() } }.background(
        Button("") { searchFocus = true }.keyboardShortcut("f").hidden())
  }
  func filter(_ name: String, _ unread: Bool) -> some View {
    Button {
      store.unreadOnly = unread
    } label: {
      Text(name).font(.system(size: 12, weight: store.unreadOnly == unread ? .semibold : .regular))
        .foregroundStyle(store.unreadOnly == unread ? Theme.accent : Theme.muted).padding(
          .vertical, 3
        ).overlay(alignment: .bottom) {
          if store.unreadOnly == unread {
            Rectangle().fill(Theme.accent).frame(height: 2).offset(y: 11)
          }
        }
    }.buttonStyle(.plain)
  }
}

struct DraftListView: View {
  @EnvironmentObject var store: MailStore
  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack {
        Text(store.scope.title).font(.system(size: 25, weight: .semibold))
        Spacer()
        Button("写邮件") { store.newDraft() }.buttonStyle(OutlineButtonStyle())
      }.padding(30)
      Divider()
      if store.visibleDrafts.isEmpty {
        ContentUnavailableView(
          store.scope.role == "drafts" ? "没有未完成的草稿" : "没有等待发送的邮件",
          systemImage: store.scope.role == "drafts" ? "doc" : "paperplane"
        ).frame(maxWidth: .infinity, maxHeight: .infinity)
      } else {
        ScrollView {
          LazyVStack(spacing: 0) {
            ForEach(store.visibleDrafts) { d in
              VStack(alignment: .leading, spacing: 10) {
                HStack {
                  Text(d.subject.isEmpty ? "（无主题）" : d.subject).font(
                    .system(size: 17, weight: .medium))
                  Spacer()
                  Text(status(d.status)).font(.system(size: 12)).foregroundStyle(
                    d.status == "failed" || d.status == "delivery_unknown"
                      ? Color.orange : Theme.muted)
                }
                Text("收件人：\(d.to)").font(.system(size: 12)).foregroundStyle(Theme.muted)
                if !d.lastError.isEmpty {
                  Text(d.lastError).font(.system(size: 12)).foregroundStyle(.orange)
                }
                HStack {
                  if d.status == "draft" || d.status == "failed" {
                    Button("继续编辑") { store.compose = d }.buttonStyle(OutlineButtonStyle())
                    if d.status == "failed" {
                      Button("重试发送") { Task { await store.submitDraft(d.id) } }.buttonStyle(
                        OutlineButtonStyle())
                    }
                  } else if d.status == "queued" {
                    Button("撤销发送") { store.cancelQueued(d) }.buttonStyle(OutlineButtonStyle())
                  } else if d.status == "delivery_unknown" {
                    Text("请先在邮箱网页确认结果，避免重复发送。").font(.system(size: 12)).foregroundStyle(Theme.muted)
                  }
                  Spacer()
                  if d.status == "draft" {
                    Button("删除草稿", role: .destructive) {
                      Task {
                        do {
                          try await store.call { try $0.deleteDraft(id: d.id) }
                          await store.reload()
                        } catch { store.errorMessage = userError(error) }
                      }
                    }.buttonStyle(.plain)
                  }
                }
              }.padding(24)
              Divider()
            }
          }
        }
      }
    }
  }
  func status(_ s: String) -> String {
    [
      "draft": "已保存到本机", "queued": "等待发送 · 可撤销", "sending": "正在提交…", "failed": "发送失败",
      "delivery_unknown": "发送结果待确认",
    ][s] ?? s
  }
}

struct CredentialAuthorizationBanner: View {
  @EnvironmentObject var store: MailStore
  var body: some View {
        HStack(spacing: 12) {
          Image(systemName: "lock.shield")
          VStack(alignment: .leading, spacing: 3) {
            Text("部分凭证需要钥匙串授权").font(.system(size: 12, weight: .medium))
            Text(store.credentialAuthorizationError ?? "点击授权后，在 macOS 提示中确认访问。后台不会主动弹窗。")
              .font(.system(size: 11)).foregroundStyle(Theme.muted)
          }
          Spacer()
          Button(store.authorizingCredentials ? "等待系统授权…" : "授权访问") {
            Task { await store.authorizeCredentials() }
          }.buttonStyle(OutlineButtonStyle()).disabled(store.authorizingCredentials)
        }.padding(.horizontal, 20).padding(.vertical, 10).background(Theme.selected)
  }
}
