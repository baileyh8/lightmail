import AppKit
import SwiftUI
import Translation
import NaturalLanguage

@MainActor final class MailStore: ObservableObject {
  @Published var accounts: [Account] = []
  @Published var folders: [Folder] = []
  @Published var messages: [MessageSummary] = []
  @Published var drafts: [Draft] = []
  @Published var scope = MailScope.inbox
  @Published var selectedID: String?
  @Published private var openedMessage: MessageSummary?
  @Published var body: MailBody?
  @Published var bodyLoading = false
  @Published var bodyError: String?
  @Published var credentialRequests: [String] = []
  @Published var authorizingCredentials = false
  @Published var credentialAuthorizationError: String?
  @Published var search = ""
  @Published var unreadOnly = false
  @Published var syncing: Set<String> = []
  @Published var accountStatus: [String: String] = [:]
  @Published var syncErrors: [String: String] = [:]
  @Published var errorMessage: String?
  @Published var toast: String?
  @Published var showSettings = false
  @Published var settingsTab = "accounts"
  @Published var compose: Draft?
  @Published var readingMode: ReadingMode = .original
  @Published var translation: TranslationResult?
  @Published var translationProgress = ""
  @Published var partialTranslation: [TranslationBlock] = []
  @Published var translating = false
  @Published var translationError: String?
  @Published var translationConfigs: [TranslationConfiguration] = []
  @Published var selectedTranslationID = ""
  @Published var googleClientID = ""
  @Published var focusReading = false
  @Published var demoMode: Bool
  @Published var storage: StorageInfo?
  @Published var systemTranslationConfiguration: TranslationSession.Configuration?
  private(set) var engine: MailEngine
  private(set) var application: MailApplication!
  private var coreObserver: CoreApplicationObserver?
  private var coreReloadTask: Task<Void, Never>?
  private var bodyTask: Task<Void, Never>?
  private var systemConfigurationSeed = TranslationSession.Configuration()
  private var translationTask: Task<Void, Never>?
  private var searchTask: Task<Void, Never>?
  private var selectionGeneration = UUID()
  private var queryGeneration = UUID()
  private var engineGeneration = UUID()
  private var translationGeneration = UUID()
  private let root: URL

  init(directory: URL? = nil, startAutomatically: Bool = true) {
    let initialDemo = CommandLine.arguments.contains("--demo")
    demoMode = initialDemo
    let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
      .first!.appendingPathComponent("Lightmail", isDirectory: true)
    if let diagnostic = MemoryTests.diagnosticDirectory {
      root = diagnostic
    } else if let directory {
      root = directory
    } else if let index = CommandLine.arguments.firstIndex(of: "--data-dir"),
      CommandLine.arguments.count > index + 1
    {
      root = URL(fileURLWithPath: CommandLine.arguments[index + 1], isDirectory: true)
    } else {
      root = base
    }
    do {
      engine = try MailEngine(
        directory: root.appendingPathComponent(initialDemo ? "Preview" : "Mail").path)
    } catch { fatalError("无法打开轻邮本地数据库：\(error.localizedDescription)") }
    connectApplication()
    if startAutomatically { Task { await start() } }
  }
  deinit { application?.stop() }
  private func connectApplication() {
    let observer = CoreApplicationObserver(generation: engineGeneration)
    observer.store = self
    coreObserver = observer
    application = MailApplication(engine: engine, platform: MacPlatformServices(), observer: observer)
  }
  func coreChanged(_ event: ApplicationEvent, generation: UUID) async {
    guard generation == engineGeneration else { return }
    switch event.kind {
    case .syncStarted:
      syncing.insert(event.accountId)
      accountStatus[event.accountId] = "同步中…"
    case .syncFinished: syncing.remove(event.accountId)
    case .problem:
      accountStatus[event.accountId] = event.message
      syncErrors[event.accountId] = event.failed ? event.message : nil
    case .draftChanged:
      if !event.message.isEmpty {
        if event.failed { errorMessage = event.message } else { notify(event.message) }
      }
      fallthrough
    case .dataChanged:
      coreReloadTask?.cancel()
      coreReloadTask = Task { [weak self] in
        try? await Task.sleep(for: .milliseconds(40))
        guard !Task.isCancelled, let self, self.engineGeneration == generation else { return }
        await self.reload()
        // Background revalidation can replace an older HTML cache while this
        // message remains selected. Updating the reader is presentation work.
        if !self.bodyLoading, let selected = self.selectedID,
          let cached = try? await self.call({ try $0.cachedBody(messageId: selected) }),
          !Task.isCancelled, self.engineGeneration == generation, self.selectedID == selected {
          self.body = cached
          self.bodyError = nil
        }
      }
    }
  }
  func setActive(_ active: Bool) { application.setActive(active: active) }
  var selectedMessage: MessageSummary? {
    messages.first { $0.id == selectedID }
      ?? (openedMessage?.id == selectedID ? openedMessage : nil)
  }
  var selectedAccount: Account? {
    selectedMessage.flatMap { m in accounts.first { $0.id == m.accountId } }
  }
  var translationConfig: TranslationConfiguration? {
    translationConfigs.first { $0.id == selectedTranslationID } ?? translationConfigs.first
  }
  var visibleSyncError: String? {
    accounts.filter { scope.accountID.isEmpty || $0.id == scope.accountID }
      .compactMap { syncErrors[$0.id] }.first
  }
  var visibleDrafts: [Draft] {
    drafts.filter { d in
      (scope.accountID.isEmpty || d.accountId == scope.accountID)
        && (scope.role == "drafts"
          ? d.status == "draft"
          : ["queued", "sending", "failed", "delivery_unknown"].contains(d.status))
        && (search.isEmpty || d.subject.localizedCaseInsensitiveContains(search)
          || d.to.localizedCaseInsensitiveContains(search))
    }
  }
  func call<T>(_ operation: @escaping (MailEngine) throws -> T) async throws -> T {
    let core = engine
    return try await Task.detached(priority: .userInitiated) { try operation(core) }.value
  }
  func start() async {
    do {
      if demoMode { try await call { try $0.seedDemo() } }
      translationConfigs = try application.translationConfigurations()
      selectedTranslationID = (try await call { try $0.setting(key: "translation-default") }) ?? ""
      googleClientID = (try await call { try $0.setting(key: "google-client-id") }) ?? ""
      await reload()
      if demoMode {
        selectedID = messages.first?.id
        await loadSelectedBody()
      }
      await refreshCredentialRequests()
      beginMonitoring()
    } catch { errorMessage = userError(error) }
  }
  func reload() async {
    do {
      accounts = try await call { try $0.accounts() }
      folders = try await call { try $0.folders(accountId: "") }
      drafts = try await call { try $0.drafts() }
      await reloadMessages()
      storage = try await call { try $0.storageInfo() }
    } catch { errorMessage = userError(error) }
  }
  func reloadMessages(loadMore: Bool = false) async {
    guard !scope.isDraftList else {
      messages = []
      return
    }
    let request = UUID()
    queryGeneration = request
    let q = MessageQuery(
      accountId: scope.accountID, folderId: scope.folderID, scope: scope.role, search: search,
      unreadOnly: unreadOnly, limit: 100, offset: loadMore ? UInt32(messages.count) : 0)
    do {
      let rows = try await call { try $0.listMessages(query: q) }
      guard queryGeneration == request else { return }
      if loadMore {
        let present = Set(messages.map(\.id))
        messages.append(contentsOf: rows.filter { !present.contains($0.id) })
      } else {
        messages = rows
      }
      if let current = messages.first(where: { $0.id == selectedID }) { openedMessage = current }
    } catch { errorMessage = userError(error) }
  }
  func setScope(_ next: MailScope) {
    bodyTask?.cancel()
    selectionGeneration = UUID()
    bodyLoading = false
    scope = next
    selectedID = nil
    body = nil
    bodyError = nil
    translation = nil
    focusReading = false
    readingMode = .original
    search = ""
    unreadOnly = false
    cancelTranslation()
    Task {
      await reloadMessages()
      if !next.accountID.isEmpty, let account = accounts.first(where: { $0.id == next.accountID }) {
        await sync(account, folderID: next.folderID)
      }
    }
  }
  func searchChanged() {
    searchTask?.cancel()
    searchTask = Task {
      try? await Task.sleep(for: .milliseconds(180))
      if !Task.isCancelled { await reloadMessages() }
    }
  }
  func select(_ id: String?) {
    guard selectedID != id else { return }
    selectedID = id
    openedMessage = messages.first { $0.id == id }
    cancelTranslation()
    translation = nil
    partialTranslation = []
    readingMode = .original
    body = nil
    bodyError = nil
    bodyTask?.cancel()
    selectionGeneration = UUID()
    bodyTask = Task { await loadSelectedBody() }
  }
  func loadSelectedBody() async {
    guard let m = selectedMessage else { return }
    let request = UUID()
    selectionGeneration = request
    bodyLoading = true
    bodyError = nil
    defer { if selectionGeneration == request { bodyLoading = false } }
    do {
      let fetched = try await application.body(messageId: m.id)
      guard !Task.isCancelled, selectionGeneration == request else { return }
      body = fetched
      guard !Task.isCancelled, selectionGeneration == request else { return }
      // Receiving the body ends the loading state. Flag sync is independent.
      bodyLoading = false
      await reloadMessages()
      guard !Task.isCancelled, selectionGeneration == request else { return }
      if m.unread { Task { await mark(m, flag: "seen", value: true, quiet: true) } }
      if let config = translationConfig, let body {
        translation = try application.cachedTranslation(messageId: m.id, body: body, configuration: config)
      }
    } catch { if selectionGeneration == request { bodyError = userError(error) } }
  }
  func refreshCredentialRequests() async {
    credentialRequests = await SecretStore.pendingKeys()
  }
  func authorizeCredentials() async {
    guard !authorizingCredentials else { return }
    authorizingCredentials = true
    credentialAuthorizationError = nil
    defer { authorizingCredentials = false }
    do {
      for key in credentialRequests { try await SecretStore.authorize(key) }
      await refreshCredentialRequests()
      refresh()

      if bodyError != nil { await loadSelectedBody() }
    } catch {
      credentialAuthorizationError = "授权尚未完成，可再次点击；后台不会重复弹窗。"
      await refreshCredentialRequests()
    }
  }
  func refresh() {
    Task {
      do { try await application.refresh() }
      catch { errorMessage = userError(error) }
    }
  }
  func sync(_ account: Account, folderID: String = "", older: Bool = false) async {
    let path = folders.first { $0.id == folderID }?.path
    do { try await application.sync(accountId: account.id, path: path, older: older) }
    catch { /* The service publishes account-specific failure state. */ }
  }
  func loadOlder() {
    Task {
      await reloadMessages(loadMore: true)
      let current = scope
      let count = messages.count
      let targets = folders.filter { folder in
        (current.accountID.isEmpty || folder.accountId == current.accountID)
          && (current.folderID.isEmpty
            ? folder.role == current.role : folder.id == current.folderID)
      }
      for folder in targets {
        guard scope == current else { return }
        if let account = accounts.first(where: { $0.id == folder.accountId && $0.enabled }) {
          await sync(account, folderID: folder.id, older: true)
        }
      }
      guard scope == current else { return }
      while messages.count < count + 100 {
        let before = messages.count
        await reloadMessages(loadMore: true)
        if messages.count == before { break }
      }
    }
  }
  func beginMonitoring() {
    guard !demoMode else { return }
    application.setActive(active: NSApp.isActive)
    do { try application.start() } catch { errorMessage = userError(error) }
  }
  func mark(_ message: MessageSummary, flag: String, value: Bool, quiet: Bool = false) async {
    do {
      try await application.mark(messageId: message.id, flag: flag, value: value)
      await reloadMessages()
      folders = try await call { try $0.folders(accountId: "") }
    } catch { if !quiet { errorMessage = userError(error) } }
  }
  func moveSelected(to role: String) {
    guard let m = selectedMessage else { return }
    Task {
      do {
        try await application.moveMessage(messageId: m.id, role: role)
        selectedID = nil
        body = nil
        await reload()
        notify(role == "archive" ? "已归档" : "已移到垃圾箱")
      } catch { errorMessage = userError(error) }
    }
  }
  func newDraft(reply: String? = nil) {
    guard
      let account = selectedAccount ?? accounts.first(where: { $0.id == scope.accountID })
        ?? accounts.first
    else {
      settingsTab = "accounts"
      showSettings = true
      return
    }
    let mode: ComposeMode = reply == "forward" ? .forward : reply == "all" ? .replyAll : reply == nil ? .new : .reply
    compose = composeDraft(account: account, message: selectedMessage, body: body, mode: mode,
      dateLabel: selectedMessage?.date.formatted() ?? "")
  }

  func saveDraft(_ draft: Draft) async {
    do {
      _ = try await call { try $0.saveDraft(draft: draft) }
      drafts = try await call { try $0.drafts() }
    } catch { errorMessage = userError(error) }
  }
  func queue(_ draft: Draft) async -> Bool {
    do {
      _ = try await application.queue(draft: draft)
      drafts = try await call { try $0.drafts() }
      compose = nil
      notify("邮件将在 5 秒后发送，可在待发送中撤销")
      return true
    } catch { errorMessage = userError(error); return false }
  }
  func submitDraft(_ id: String) async {
    do { _ = try await application.submit(id: id) }
    catch { errorMessage = userError(error) }
    drafts = (try? await call { try $0.drafts() }) ?? drafts
  }
  func cancelQueued(_ draft: Draft) {
    Task {
      do {
        try application.cancelQueued(id: draft.id)
        drafts = try await call { try $0.drafts() }
        notify("已撤销发送，邮件保留在草稿")
      } catch { errorMessage = userError(error) }
    }
  }
  func beginTranslation(force: Bool = false) {
    guard let message = selectedMessage, let body else { return }
    guard let config = translationConfig else {
      settingsTab = "translation"
      showSettings = true
      return
    }
    focusReading = true
    readingMode = .bilingual
    if translation != nil && !force { return }
    cancelTranslation()
    translation = nil
    let generation = translationGeneration
    translating = true
    translationError = nil
    partialTranslation = []
    translationProgress = "正在理解整封邮件…"
    if config.engine == "system" {
      let language = NLLanguageRecognizer.dominantLanguage(for: String(body.text.prefix(20_000)))
      systemConfigurationSeed.source = language.map { Locale.Language(identifier: $0.rawValue) }
      systemConfigurationSeed.target = Locale.Language(identifier: languageCode(config.targetLanguage))
      // A fresh revision is essential: nil -> the same configuration within one
      // SwiftUI update does not restart a dismissed or cancelled system session.
      systemConfigurationSeed.invalidate()
      translationProgress = "正在检查系统翻译语言…"
      systemTranslationConfiguration = systemConfigurationSeed
      return
    }
    let selected = message.id
    translationTask = Task {
      do {
        let observer = CoreTranslationObserver { [weak self] done, total, blocks in
          await self?.setTranslationProgress(selected: selected, generation: generation,
            done: done, total: total, blocks: blocks)
        }
        let result = try await application.translate(messageId: message.id, body: body,
          configuration: config, force: force, observer: observer)
        try Task.checkCancellation()
        guard selectedID == selected, translationGeneration == generation else { return }
        translation = result
        translating = false
        translationProgress = "全文已翻译"

      } catch {
        if selectedID == selected, translationGeneration == generation {
          translating = false
          translationError = userError(error)
          translationProgress = partialTranslation.isEmpty ? "翻译未完成" : "部分完成，尚有内容未翻译"
        }
      }
    }
  }
  func languageCode(_ name: String) -> String {
    ["简体中文": "zh-Hans", "English": "en", "日本語": "ja", "繁體中文": "zh-Hant"][name] ?? "zh-Hans"
  }
  func performSystemTranslation(_ session: TranslationSession) async {
    guard translating, let message = selectedMessage, let body, let config = translationConfig,
      config.engine == "system" else { return }
    let selected = message.id
    let generation = translationGeneration
    do {
      if let source = session.sourceLanguage {
        let status = await LanguageAvailability().status(from: source, to: session.targetLanguage)
        guard selectedID == selected, translationGeneration == generation else { return }
        if status == .unsupported { throw MailAppError.message("系统不支持这组语言，请改用 Chat 格式 LLM") }
        if status != .installed {
          translationProgress = "等待下载系统语言包；关闭提示后可点「语言包…」重新打开"
          try await session.prepareTranslation()
        }
      }
      try Task.checkCancellation()
      guard selectedID == selected, translationGeneration == generation else { return }
      translationProgress = "正在使用系统翻译…"
      let blocks = TranslationService.blocks(body.markdown)
      var translated: [TranslationBlock] = []
      var title = message.subject
      let requests =
        [TranslationSession.Request(sourceText: message.subject, clientIdentifier: "subject")]
        + blocks.map {
          TranslationSession.Request(sourceText: $0.text, clientIdentifier: String($0.id))
        }
      for try await item in session.translate(batch: requests) {
        guard selectedID == selected, translationGeneration == generation, translating else {
          return
        }
        if item.clientIdentifier == "subject" {
          title = item.targetText
        } else if let id = UInt32(item.clientIdentifier ?? "") {
          translated.append(.init(id: id, text: item.targetText))
          partialTranslation = translated.sorted { $0.id < $1.id }
        }
      }
      guard selectedID == selected, translationGeneration == generation, translating else { return }
      guard translated.count == blocks.count else { throw MailAppError.message("系统翻译尚未处理完整正文") }
      let result = TranslationResult(
        subject: title, blocks: translated.sorted { $0.id < $1.id }, sourceHash: body.contentHash,
        model: "macOS 系统翻译")
      try application.saveSystemTranslation(messageId: message.id, body: body,
        configuration: config, result: result)
      translation = result
      translating = false
      translationProgress = "全文已翻译"
    } catch {
      if selectedID == selected, translationGeneration == generation {
        translating = false
        translationProgress = "系统翻译未完成"
        translationError = "\(userError(error))。可点「语言包…」检查下载，然后重译。"
      }
    }
  }
  func setTranslationProgress(
    selected: String, generation: UUID, done: Int, total: Int, blocks: [TranslationBlock]
  ) {
    guard translating, selectedID == selected, translationGeneration == generation,
      blocks.count >= partialTranslation.count else { return }
    translationProgress = "已翻译 \(done) / \(total) 段"
    partialTranslation = blocks
  }
  func cancelTranslation() {
    translationGeneration = UUID()
    translationTask?.cancel()
    translationTask = nil
    translating = false
    systemTranslationConfiguration = nil
  }
  func copyMarkdown(mode: ReadingMode? = nil) {
    guard let message = selectedMessage, let body else { return }
    let mode = mode ?? readingMode
    if mode != .original && translation == nil {
      notify("译文尚未完成，暂不能复制完整译文")
      return
    }
    let output: String
    do {
      let exportMode: ExportMode = mode == .original ? .original : mode == .translated ? .translated : .bilingual
      output = try exportMarkdown(message: message, body: body, translation: translation,
        mode: exportMode, dateLabel: message.date.formatted(.iso8601))
    } catch { errorMessage = userError(error); return }
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(output, forType: .string)
    notify("已复制\(mode == .bilingual ? "双语" : mode.rawValue) Markdown")
  }
  func download(_ attachment: AttachmentInfo) {
    guard let message = selectedMessage else { return }
    let panel = NSSavePanel()
    panel.nameFieldStringValue = attachment.filename
    guard panel.runModal() == .OK, let url = panel.url else { return }
    Task {
      do {
        try await application.download(messageId: message.id, partId: attachment.partId, destination: url.path)
        notify("附件已保存")
      } catch { errorMessage = userError(error) }
    }
  }
  func persistTranslationSettings() async throws {
    try application.saveTranslationConfigurations(configurations: translationConfigs, selectedId: selectedTranslationID)
    cancelTranslation()
    translation = nil
  }
  func saveAccount(_ account: Account, password: String) async throws {
    try await application.saveAccount(account: account, password: password)
    await reload()
    beginMonitoring()
  }
  func finishAccountSetup(_ account: Account) {
    // Account persistence completes login. Folder sync must not keep the
    // setup sheet open; setScope schedules it independently in the background.
    showSettings = false
    setScope(MailScope(title: "\(account.name) · 收件箱", role: "inbox", accountID: account.id))
    notify("邮箱已连接，正在同步邮件")
    NSApp.windows.first { $0.canBecomeMain && $0.sheetParent == nil }?.makeKeyAndOrderFront(nil)
    NSApp.activate(ignoringOtherApps: true)
  }
  func removeAccount(_ account: Account) async throws {
    try await application.removeAccount(accountId: account.id)
    syncErrors[account.id] = nil
    selectedID = nil
    body = nil
    await reload()
  }
  func saveGoogle(clientSecret: String) async throws {
    let value = googleClientID
    try await call { try $0.setSetting(key: "google-client-id", value: value) }
    if !clientSecret.isEmpty { try await SecretStore.save(clientSecret, for: "google-client-secret") }
  }
  func toggleDemo() async {
    application.stop()
    bodyTask?.cancel()
    coreReloadTask?.cancel()
    cancelTranslation()
    engineGeneration = UUID()
    selectionGeneration = UUID()
    queryGeneration = UUID()
    syncing = []
    syncErrors = [:]
    accountStatus = [:]
    do {
      demoMode.toggle()
      engine = try MailEngine(
        directory: root.appendingPathComponent(demoMode ? "Preview" : "Mail").path)
      connectApplication()
      accounts = []
      folders = []
      messages = []
      selectedID = nil
      body = nil
      translation = nil
      translationConfigs = []
      scope = .inbox
      await start()
    } catch { errorMessage = userError(error) }
  }
  func importMessage() {
    let panel = NSOpenPanel()
    panel.allowedContentTypes = [.emailMessage]
    panel.allowsMultipleSelection = false
    guard panel.runModal() == .OK, let url = panel.url else { return }
    Task {
      do {
        let account = Account(
          id: "local-import", name: "本地邮件", address: "local@example.com", provider: "local",
          imapHost: "localhost", imapPort: 993, smtpHost: "localhost", smtpPort: 465,
          authKind: "none", color: "69736E", enabled: false, sentMode: "server")
        try await call {
          try $0.saveAccount(account: account)
          _ = try $0.importEml(path: url.path, accountId: account.id)
        }
        await reload()
        notify("已导入邮件，可在本地查看和翻译")
      } catch { errorMessage = userError(error) }
    }
  }
  func notify(_ message: String) {
    toast = message
    Task {
      try? await Task.sleep(for: .seconds(3))
      if toast == message { toast = nil }
    }
  }
}
