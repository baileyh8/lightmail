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
  private let credentials = CredentialProvider()
  private var monitorTasks: [String: Task<Void, Never>] = [:]
  private var periodicTask: Task<Void, Never>?
  private var preloadTasks: [String: Task<Void, Never>] = [:]
  private var preloadRequested: Set<String> = []
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
    if startAutomatically { Task { await start() } }
  }
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
      if let json = try await call({ try $0.setting(key: "translation-configs") }),
        let data = json.data(using: .utf8)
      {
        translationConfigs =
          (try? JSONDecoder().decode([TranslationConfiguration].self, from: data)) ?? []
      }
      selectedTranslationID = (try await call { try $0.setting(key: "translation-default") }) ?? ""
      googleClientID = (try await call { try $0.setting(key: "google-client-id") }) ?? ""
      await reload()
      if demoMode {
        selectedID = messages.first?.id
        await loadSelectedBody()
      }
      await refreshCredentialRequests()
      beginMonitoring()
      for account in accounts where account.enabled { schedulePreload(account) }
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
      if let cached = try await call({ try $0.cachedBody(messageId: m.id) }) {
        if selectionGeneration == request { body = cached }
      } else {
        guard let account = accounts.first(where: { $0.id == m.accountId }) else { return }
        let token = try await credential(account)
        guard !Task.isCancelled, selectionGeneration == request else { return }
        let fetched = try await call { try $0.fetchBody(messageId: m.id, credential: token) }
        guard selectionGeneration == request else { return }
        body = fetched
      }
      guard !Task.isCancelled, selectionGeneration == request else { return }
      // Receiving the body ends the loading state. Flag sync is independent.
      bodyLoading = false
      await reloadMessages()
      guard !Task.isCancelled, selectionGeneration == request else { return }
      if m.unread { Task { await mark(m, flag: "seen", value: true, quiet: true) } }
      if let config = translationConfig, let body {
        let key = TranslationService.cacheKey(
          accountID: m.accountId, body: body, configuration: config, subject: m.subject)
        if let data = try await call({ try $0.setting(key: key) })?.data(using: .utf8),
          selectionGeneration == request
        {
          translation = try? JSONDecoder().decode(TranslationResult.self, from: data)
        }
      }
    } catch { if selectionGeneration == request { bodyError = userError(error) } }
  }
  func credential(_ account: Account) async throws -> String {
    if account.provider == "demo" || account.provider == "local" { return "" }
    for host in Set([account.imapHost, account.smtpHost]) where !host.isEmpty {
      let route = try await Task.detached { try MailProxyRoute.resolve(host: host) }.value
      try await call { try $0.configureProxy(destination: host, kind: route.kind, host: route.host, port: route.port) }
    }
    return try await credentials.credential(
      account: account, clientID: googleClientID)
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
      for account in accounts where account.enabled { schedulePreload(account) }
      if bodyError != nil { await loadSelectedBody() }
    } catch {
      credentialAuthorizationError = "授权尚未完成，可再次点击；后台不会重复弹窗。"
      await refreshCredentialRequests()
    }
  }
  func refresh() {
    Task {
      await withTaskGroup(of: Void.self) { group in
        for account in accounts where account.enabled { group.addTask { await self.sync(account) } }
      }
      await reload()
    }
  }
  func sync(_ account: Account, folderID: String = "", older: Bool = false) async {
    guard account.enabled, !syncing.contains(account.id) else { return }
    let generation = engineGeneration
    syncing.insert(account.id)
    accountStatus[account.id] = "同步中…"
    defer { if engineGeneration == generation { syncing.remove(account.id) } }
    do {
      let token = try await credential(account)
      let result: SyncResult
      if !folderID.isEmpty, let folder = folders.first(where: { $0.id == folderID }) {
        result = try await call {
          try $0.syncFolder(
            accountId: account.id, credential: token, path: folder.path, older: older)
        }
      } else {
        // Publish the inbox and start warming recent bodies before scanning other folders.
        _ = try await call { try $0.syncFolder(accountId: account.id, credential: token, path: "INBOX", older: false) }
        guard engineGeneration == generation else { return }
        _ = try await call { try $0.pruneBodyCache(accountId: account.id) }
        await reload()
        schedulePreload(account)
        result = try await call { try $0.syncAccount(accountId: account.id, credential: token) }
      }
      guard engineGeneration == generation else { return }
      accountStatus[account.id] = result.message
      syncErrors[account.id] = nil
      _ = try await call { try $0.pruneBodyCache(accountId: account.id) }
      await reload()
      schedulePreload(account)
    } catch {
      if engineGeneration == generation {
        accountStatus[account.id] = userError(error)
        syncErrors[account.id] = userError(error)
      }
    }
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
    periodicTask?.cancel()
    monitorTasks.values.forEach { $0.cancel() }
    monitorTasks = [:]
    preloadTasks.values.forEach { $0.cancel() }
    preloadTasks = [:]
    preloadRequested = []
    guard !demoMode else { return }
    for account in accounts where account.enabled {
      monitorTasks[account.id] = Task { [weak self] in
        var failures = 0
        while !Task.isCancelled {
          guard let self else { return }
          do {
            let token = try await self.credential(account)
            let changed = try await self.call {
              try $0.waitForChange(accountId: account.id, credential: token)
            }
            if Task.isCancelled { return }
            failures = 0
            if changed { await self.sync(account) } else { try await Task.sleep(for: .seconds(15)) }
          } catch {
            failures += 1
            if !Task.isCancelled { self.accountStatus[account.id] = userError(error) }
            try? await Task.sleep(for: .seconds(min(300, 15 * (1 << min(failures, 4)))))
          }
        }
      }
    }
    periodicTask = Task { [weak self] in
      while !Task.isCancelled {
        guard let self else { return }
        self.refresh()
        try? await Task.sleep(for: .seconds(NSApp.isActive ? 30 : 120))
      }
    }
  }
  func schedulePreload(_ account: Account) {
    guard account.enabled, !["demo", "local"].contains(account.provider), !demoMode else { return }
    preloadRequested.insert(account.id)
    guard preloadTasks[account.id] == nil else { return }
    let generation = engineGeneration
    preloadTasks[account.id] = Task { [weak self] in
      guard let self else { return }
      defer { if self.engineGeneration == generation { self.preloadTasks[account.id] = nil } }
      while self.preloadRequested.remove(account.id) != nil && !Task.isCancelled {
        do {
          let token = try await self.credential(account)
          let recent = try await self.call { try $0.recentMessages(accountId: account.id) }
          for message in recent {
            guard !Task.isCancelled, self.engineGeneration == generation else { return }
            if self.preloadRequested.contains(account.id) { break }
            // A failed message must not prevent the rest of the window warming.
            try? await self.call { try $0.preloadBody(messageId: message.id, credential: token) }
            guard !Task.isCancelled, self.engineGeneration == generation else { return }
            await self.reloadMessages()
            if self.selectedID == message.id,
              let refreshed = try? await self.call({ try $0.cachedBody(messageId: message.id) }),
              self.selectedID == message.id {
              self.body = refreshed
            }
          }
          _ = try await self.call { try $0.pruneBodyCache(accountId: account.id) }
          self.storage = try await self.call { try $0.storageInfo() }
        } catch { break }
      }
    }
  }
  func mark(_ message: MessageSummary, flag: String, value: Bool, quiet: Bool = false) async {
    do {
      guard let account = accounts.first(where: { $0.id == message.accountId }) else { return }
      let token = try await credential(account)
      try await call {
        try $0.changeFlag(messageId: message.id, credential: token, flag: flag, value: value)
      }
      await reloadMessages()
      folders = try await call { try $0.folders(accountId: "") }
    } catch { if !quiet { errorMessage = userError(error) } }
  }
  func moveSelected(to role: String) {
    guard let m = selectedMessage, let a = selectedAccount else { return }
    Task {
      do {
        let token = try await credential(a)
        try await call { try $0.moveMessage(messageId: m.id, credential: token, role: role) }
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
    let timestamp = Int64(Date().timeIntervalSince1970)
    var draft = Draft(
      id: UUID().uuidString, accountId: account.id, to: "", cc: "", bcc: "", subject: "", body: "",
      attachmentPaths: [], replyToMessageId: "", references: "", status: "draft", lastError: "",
      createdAt: timestamp, updatedAt: timestamp, sendAfter: 0)
    if let mode = reply, let message = selectedMessage {
      if mode != "forward" {
        draft.to = message.replyToAddress.isEmpty ? message.fromAddress : message.replyToAddress
        if mode == "all" {
          let others = (message.toAddresses + "," + message.ccAddresses).split(separator: ",").map {
            $0.trimmingCharacters(in: .whitespaces)
          }.filter {
            !$0.isEmpty && $0.caseInsensitiveCompare(account.address) != .orderedSame
              && $0.caseInsensitiveCompare(message.fromAddress) != .orderedSame
          }
          draft.cc = Array(Set(others)).sorted().joined(separator: ", ")
        }
        draft.subject =
          message.subject.lowercased().hasPrefix("re:") ? message.subject : "Re: " + message.subject
        draft.replyToMessageId = message.messageId
        draft.references = message.messageId
      } else {
        draft.subject = "Fwd: " + message.subject
      }
      let quote = (body?.text ?? "").components(separatedBy: "\n").map { "> " + $0 }.joined(
        separator: "\n")
      draft.body = "\n\n\(message.date.formatted())，\(message.sender) 写道：\n\(quote)"
    }
    compose = draft
  }
  func saveDraft(_ draft: Draft) async {
    do {
      _ = try await call { try $0.saveDraft(draft: draft) }
      drafts = try await call { try $0.drafts() }
    } catch { errorMessage = userError(error) }
  }
  func queue(_ draft: Draft) async -> Bool {
    guard let account = accounts.first(where: { $0.id == draft.accountId }),
      account.provider != "demo", account.provider != "local"
    else {
      errorMessage = "示例或本地导入账号不能发送邮件，请添加真实邮箱"
      return false
    }
    guard
      !draft.to.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !draft.cc.isEmpty
        || !draft.bcc.isEmpty
    else {
      errorMessage = "请填写收件人"
      return false
    }
    var queued = draft
    queued.status = "queued"
    queued.sendAfter = Int64(Date().timeIntervalSince1970) + 5
    do {
      _ = try await call { try $0.saveDraft(draft: queued) }
      drafts = try await call { try $0.drafts() }
    } catch {
      errorMessage = userError(error)
      return false
    }
    compose = nil
    notify("邮件将在 5 秒后发送，可在待发送中撤销")
    Task {
      try? await Task.sleep(for: .seconds(5))
      await submitDraft(queued.id)
    }
    return true
  }
  func submitDraft(_ id: String) async {
    do {
      let rows = try await call { try $0.drafts() }
      guard let d = rows.first(where: { $0.id == id && ["queued", "failed"].contains($0.status) }),
        let account = accounts.first(where: { $0.id == d.accountId })
      else { return }
      let token = try await credential(account)
      if let i = drafts.firstIndex(where: { $0.id == id }) { drafts[i].status = "sending" }
      let result = try await call { try $0.sendDraft(id: id, credential: token) }
      drafts = try await call { try $0.drafts() }
      if result.status == "accepted" {
        notify(result.lastError.isEmpty ? "已提交到发件服务器" : result.lastError)
        await sync(account)
      } else {
        errorMessage = result.lastError
      }
    } catch {
      errorMessage = userError(error)
      let reason = userError(error)
      try? await call { try $0.failUnsubmitted(id: id, message: reason) }
      if let rows = try? await call({ try $0.drafts() }) { drafts = rows }
    }
  }
  func cancelQueued(_ draft: Draft) {
    Task {
      do {
        try await call { try $0.cancelQueued(id: draft.id) }
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
        let key = try await SecretStore.read("translation:\(config.id)") ?? ""
        let result = try await TranslationService.translate(
          subject: message.subject, markdown: body.markdown, hash: body.contentHash,
          configuration: config, key: key
        ) { [weak self] done, total, blocks in
          await self?.setTranslationProgress(
            selected: selected, generation: generation, done: done, total: total, blocks: blocks)
        }
        try Task.checkCancellation()
        guard selectedID == selected, translationGeneration == generation else { return }
        translation = result
        translating = false
        translationProgress = "全文已翻译"
        let cache = TranslationService.cacheKey(
          accountID: message.accountId, body: body, configuration: config, subject: message.subject)
        let json = String(decoding: try JSONEncoder().encode(result), as: UTF8.self)
        try await call { try $0.setSetting(key: cache, value: json) }
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
        } else if let id = Int(item.clientIdentifier ?? "") {
          translated.append(.init(id: id, text: item.targetText))
          partialTranslation = translated.sorted { $0.id < $1.id }
        }
      }
      guard selectedID == selected, translationGeneration == generation, translating else { return }
      guard translated.count == blocks.count else { throw MailAppError.message("系统翻译尚未处理完整正文") }
      let result = TranslationResult(
        subject: title, blocks: translated.sorted { $0.id < $1.id }, sourceHash: body.contentHash,
        model: "macOS 系统翻译")
      translation = result
      translating = false
      translationProgress = "全文已翻译"
      let cache = TranslationService.cacheKey(
        accountID: message.accountId, body: body, configuration: config, subject: message.subject)
      let json = String(decoding: try JSONEncoder().encode(result), as: UTF8.self)
      try await call { try $0.setSetting(key: cache, value: json) }
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
    guard selectedID == selected, translationGeneration == generation else { return }
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
    var output =
      "# \(mode == .original ? message.subject : translation?.subject ?? message.subject)\n\n- 发件人：\(message.sender) <\(message.fromAddress)>\n- 收件人：\(message.toAddresses)\n- 日期：\(message.date.formatted(.iso8601))\n\n---\n\n"
    switch mode {
    case .original: output += body.markdown
    case .translated: output += translation!.markdown
    case .bilingual:
      let originals = TranslationService.blocks(body.markdown)
      for block in originals {
        output +=
          block.text + "\n\n" + (translation!.blocks.first { $0.id == block.id }?.text ?? "（此段未翻译）")
          + "\n\n---\n\n"
      }
    }
    if !body.attachments.isEmpty {
      output +=
        "\n\n## 附件\n"
        + body.attachments.map { "- \($0.filename)（\(humanSize($0.size))）" }.joined(separator: "\n")
    }
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(output, forType: .string)
    notify("已复制\(mode == .bilingual ? "双语" : mode.rawValue) Markdown")
  }
  func download(_ attachment: AttachmentInfo) {
    guard let message = selectedMessage, let account = selectedAccount else { return }
    let panel = NSSavePanel()
    panel.nameFieldStringValue = attachment.filename
    guard panel.runModal() == .OK, let url = panel.url else { return }
    Task {
      do {
        let token = try await credential(account)
        try await call {
          try $0.downloadAttachment(
            messageId: message.id, partId: attachment.partId, credential: token,
            destination: url.path)
        }
        notify("附件已保存")
      } catch { errorMessage = userError(error) }
    }
  }
  func persistTranslationSettings() async throws {
    let json = String(decoding: try JSONEncoder().encode(translationConfigs), as: UTF8.self)
    let id = selectedTranslationID
    try await call {
      try $0.setSetting(key: "translation-configs", value: json)
      try $0.setSetting(key: "translation-default", value: id)
    }
    cancelTranslation()
    translation = nil
  }
  func saveAccount(_ account: Account, password: String) async throws {
    if !password.isEmpty { try await SecretStore.save(password, for: "account:\(account.id)") }
    try await call { try $0.saveAccount(account: account) }
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
    monitorTasks[account.id]?.cancel()
    monitorTasks[account.id] = nil
    preloadTasks[account.id]?.cancel()
    preloadTasks[account.id] = nil
    preloadRequested.remove(account.id)
    try await call { try $0.removeAccount(accountId: account.id) }
    try await SecretStore.remove("account:\(account.id)")
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
    periodicTask?.cancel()
    monitorTasks.values.forEach { $0.cancel() }
    monitorTasks = [:]
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
