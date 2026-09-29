import SwiftUI

struct SettingsView: View {
  @EnvironmentObject var store: MailStore
  @Environment(\.dismiss) private var dismiss
  @State private var selectedAccount: Account?
  @State private var editingNew = false
  @State private var selectedConfigID = ""
  @State private var clientSecret = ""
  @State private var status = ""
  var body: some View {
    VStack(spacing: 0) {
      HStack {
        Text("设置").font(.system(size: 22, weight: .semibold))
        Spacer()
        Picker("设置分类", selection: $store.settingsTab) {
          Text("邮箱账号").tag("accounts")
          Text("翻译").tag("translation")
          Text("存储").tag("storage")
        }.pickerStyle(.segmented).frame(width: 260)
        Spacer()
        Button("完成") { dismiss() }.buttonStyle(OutlineButtonStyle())
      }.padding(24)
      Divider()
      if !store.credentialRequests.isEmpty { CredentialAuthorizationBanner().environmentObject(store) }
      if store.settingsTab == "accounts" {
        accountsPanel
      } else if store.settingsTab == "translation" {
        translationPanel
      } else {
        storagePanel
      }
      Divider()
      HStack(spacing: 16) {
        Text("轻邮 Lightmail").foregroundStyle(Theme.muted)
        Spacer()
        Link("隐私政策", destination: URL(string: "https://lightmail.sohym.com/privacy/")!)
        Link("使用条款", destination: URL(string: "https://lightmail.sohym.com/terms/")!)
      }.font(.system(size: 12)).padding(.horizontal, 24).padding(.vertical, 10)
    }.frame(width: 960, height: 700).background(Color.white)
      .disclosureGroupStyle(AreaDisclosureStyle())
  }
  var accountsPanel: some View {
    HSplitView {
      VStack(alignment: .leading, spacing: 8) {
        HStack {
          Text("我的邮箱").font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.muted)
          Spacer()
          Button {
            selectedAccount = nil
            editingNew = true
          } label: {
            Image(systemName: "plus")
          }.buttonStyle(AreaButtonStyle())
        }.padding(.bottom, 12)
        ForEach(store.accounts) { a in
          Button {
            selectedAccount = a
            editingNew = false
          } label: {
            HStack(spacing: 9) {
              Circle().fill(Color(hex: a.color)).frame(width: 8, height: 8)
              VStack(alignment: .leading, spacing: 4) {
                Text(a.name).font(.system(size: 13, weight: .medium))
                Text(a.address).font(.system(size: 10)).foregroundStyle(Theme.muted).lineLimit(1)
              }
              Spacer()
            }.padding(10).background(
              selectedAccount?.id == a.id ? Theme.selected : Color.clear,
              in: RoundedRectangle(cornerRadius: 6)).contentShape(Rectangle())
          }.buttonStyle(AreaButtonStyle())
        }
        Spacer()
        Text("凭证保存在 macOS Keychain。\n账号数据保存在本机。").font(.system(size: 11)).foregroundStyle(
          Theme.muted
        ).lineSpacing(5)
      }.padding(18).frame(minWidth: 210, maxWidth: 230).background(Theme.sidebar)
      ScrollView {
        VStack(alignment: .leading, spacing: 22) {
          if editingNew || selectedAccount != nil || store.accounts.isEmpty {
            AccountEditor(account: selectedAccount).id(selectedAccount?.id ?? "new")
              .environmentObject(store)
          } else {
            VStack(alignment: .leading, spacing: 16) {
              Text("连接你的邮箱").font(.system(size: 24, weight: .semibold))
              Text("每个账号独立同步，聚合查看时仍清楚标明来源。").foregroundStyle(Theme.muted)
              Button("添加邮箱") { editingNew = true }.buttonStyle(OutlineButtonStyle(prominent: true))
            }.padding(.vertical, 20)
          }
          Divider()
          DisclosureGroup("Google 登录配置") {
            VStack(alignment: .leading, spacing: 12) {
              Text("使用你自己的 Google OAuth 桌面应用客户端。首次接入需要配置 Client ID；不会共用其他项目中的凭证。").font(
                .system(size: 11)
              ).foregroundStyle(Theme.muted)
              TextField("Desktop Client ID", text: $store.googleClientID).textFieldStyle(
                .roundedBorder)
              SecureField("Client Secret（客户端提供时填写）", text: $clientSecret).textFieldStyle(
                .roundedBorder)
              HStack {
                Button("保存 Google 配置") {
                  Task {
                    do {
                      try await store.saveGoogle(clientSecret: clientSecret)
                      clientSecret = ""
                      status = "Google 配置已保存"
                    } catch { status = userError(error) }
                  }
                }.buttonStyle(OutlineButtonStyle())
                Text(status).font(.system(size: 11)).foregroundStyle(Theme.muted)
              }
            }.padding(.top, 12)
          }.font(.system(size: 13))
        }.padding(28)
      }.frame(minWidth: 550, maxWidth: .infinity)
    }
  }
  var translationPanel: some View {
    HSplitView {
      VStack(alignment: .leading, spacing: 10) {
        HStack {
          Text("翻译服务").font(.system(size: 12, weight: .medium)).foregroundStyle(Theme.muted)
          Spacer()
          Button {
            let c = TranslationConfiguration()
            store.translationConfigs.append(c)
            selectedConfigID = c.id
            if store.selectedTranslationID.isEmpty { store.selectedTranslationID = c.id }
          } label: {
            Image(systemName: "plus")
          }.buttonStyle(AreaButtonStyle())
        }.padding(.bottom, 10)
        ForEach(store.translationConfigs) { c in
          Button {
            selectedConfigID = c.id
          } label: {
            HStack {
              VStack(alignment: .leading, spacing: 4) {
                Text(c.name).font(.system(size: 13, weight: .medium))
                Text(c.engine == "system" ? "设备端翻译" : c.model.isEmpty ? "尚未配置模型" : c.model).font(
                  .system(size: 10)
                ).foregroundStyle(Theme.muted).lineLimit(1)
              }
              Spacer()
              if c.id == store.selectedTranslationID {
                Image(systemName: "checkmark.circle.fill").font(.system(size: 12)).foregroundStyle(
                  Theme.accent)
              }
            }.padding(10).background(
              (selectedConfigID.isEmpty ? store.translationConfig?.id : selectedConfigID) == c.id
                ? Theme.selected : Color.clear, in: RoundedRectangle(cornerRadius: 6)).contentShape(Rectangle())
          }.buttonStyle(AreaButtonStyle())
        }
        Spacer()
        Text("只在你点击翻译时请求。\n译文缓存到本机。").font(.system(size: 11)).foregroundStyle(Theme.muted)
          .lineSpacing(5)
      }.padding(18).frame(minWidth: 210, maxWidth: 230).background(Theme.sidebar)
      if let config = store.translationConfigs.first(where: {
        $0.id == (selectedConfigID.isEmpty ? store.translationConfig?.id : selectedConfigID)
      }) {
        TranslationEditor(configuration: config).id(config.id).environmentObject(store)
      } else {
        VStack(alignment: .leading, spacing: 18) {
          Image(systemName: "translate").font(.system(size: 36, weight: .light)).foregroundStyle(
            Theme.accent)
          Text("让译文更准确达意").font(.system(size: 25, weight: .semibold))
          Text("连接 OpenAI Chat Completions 兼容服务，保留整封邮件的语境、语气与专业术语。").font(.system(size: 14))
            .foregroundStyle(Theme.muted).lineSpacing(6)
          Button("添加翻译服务") {
            let c = TranslationConfiguration()
            store.translationConfigs.append(c)
            store.selectedTranslationID = c.id
            selectedConfigID = c.id
          }.buttonStyle(OutlineButtonStyle(prominent: true))
        }.padding(45).frame(maxWidth: .infinity, maxHeight: .infinity)
      }
    }
  }
  var storagePanel: some View {
    VStack(alignment: .leading, spacing: 25) {
      Text("保持轻盈").font(.system(size: 26, weight: .semibold))
      Text("每个邮箱分别预加载并保留最新 20 封正文；新邮件到达后释放第 21 封及更早的正文缓存。摘要保留，旧信打开时临时读取，附件按需下载。").font(.system(size: 13)).foregroundStyle(
        Theme.muted
      ).lineSpacing(6)
      if let storage = store.storage {
        HStack(spacing: 50) {
          metric("已同步邮件", String(storage.messageCount))
          metric("正文缓存", humanSize(storage.cacheBytes))
          metric("数据库", humanSize(storage.databaseBytes))
        }
        Divider()
        Button("清理正文与翻译缓存") {
          Task {
            do {
              try await store.call { try $0.clearBodyCache() }
              store.body = nil
              store.translation = nil
              await store.reload()
              store.notify("可重新下载的缓存已清理")
            } catch { store.errorMessage = userError(error) }
          }
        }.buttonStyle(OutlineButtonStyle())
      }
      Text("受保护内容：本地草稿、待发送记录和你下载到其他文件夹的附件。").font(.system(size: 12)).foregroundStyle(Theme.muted)
      Divider()
      HStack {
        Text("轻邮 \(Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "0.0.3") · macOS 原生应用").font(.system(size: 12)).foregroundStyle(Theme.muted)
        Spacer()
        Button(store.demoMode ? "返回真实邮箱" : "打开示例预览") {
          Task {
            await store.toggleDemo()
            dismiss()
          }
        }.buttonStyle(OutlineButtonStyle())
      }
      Spacer()
    }.padding(36).frame(maxWidth: .infinity, alignment: .leading)
  }
  func metric(_ title: String, _ value: String) -> some View {
    VStack(alignment: .leading, spacing: 10) {
      Text(title).font(.system(size: 12)).foregroundStyle(Theme.muted)
      Text(value).font(.system(size: 25, weight: .medium))
    }
  }
}

struct AccountEditor: View {
  @EnvironmentObject var store: MailStore
  let account: Account?
  @State private var id = UUID().uuidString
  @State private var name = ""
  @State private var address = ""
  @State private var provider = "gmail"
  @State private var authKind = "oauth"
  @State private var password = ""
  @State private var imapHost = "imap.gmail.com"
  @State private var smtpHost = "smtp.gmail.com"
  @State private var smtpPort = 465
  @State private var imapPort = 993
  @State private var enabled = true
  @State private var sentMode = "server"
  @State private var color = "#226451"
  @State private var busy = false
  @State private var showLanguages = false
  @State private var status = ""
  @State private var confirmRemoval = false
  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      Text(account == nil ? "添加邮箱" : "邮箱账号").font(.system(size: 23, weight: .semibold))
      if account?.provider == "demo" || account?.provider == "local" {
        Text("此账号仅用于示例或本地导入，不会连接收发服务器。").font(.system(size: 13)).foregroundStyle(Theme.muted)
      } else {
        Picker("邮箱类型", selection: $provider) {
          Text("Gmail").tag("gmail")
          Text("163 邮箱").tag("163")
          Text("QQ 邮箱").tag("qq")
          Text("其他 IMAP").tag("custom")
        }.pickerStyle(.segmented).labelsHidden().onChange(of: provider) { _, new in applyPreset(new)
        }
        field("显示名称", text: $name, prompt: "例如：Gmail 工作")
        field("邮箱地址", text: $address, prompt: "完整邮箱地址")
        if provider == "gmail" {
          Picker("登录方式", selection: $authKind) {
            Text("Google 登录（推荐）").tag("oauth")
            Text("应用专用密码").tag("password")
          }.font(.system(size: 12))
        }
        if authKind != "oauth" {
          VStack(alignment: .leading, spacing: 6) {
            Text(provider == "gmail" ? "应用专用密码" : "客户端授权码").font(.system(size: 12)).foregroundStyle(
              Theme.muted)
            SecureField(account == nil ? "在此安全输入" : "已保存则留空", text: $password).textFieldStyle(
              .roundedBorder)
            Text("163 / QQ 请先在邮箱网页开启 IMAP/SMTP，并生成客户端授权码。").font(.system(size: 10)).foregroundStyle(
              Theme.muted)
          }
        }
        DisclosureGroup("连接与发送设置") {
          VStack(alignment: .leading, spacing: 12) {
            HStack {
              TextField("IMAP 服务器", text: $imapHost)
              TextField("端口", value: $imapPort, format: .number.grouping(.never)).frame(width: 70)
            }
            HStack {
              TextField("SMTP 服务器", text: $smtpHost)
              TextField("端口", value: $smtpPort, format: .number.grouping(.never)).frame(width: 70)
            }
            Picker("已发送副本", selection: $sentMode) {
              Text("服务器自动保存").tag("server")
              Text("由轻邮保存到已发送").tag("append")
            }
            Text("Gmail 通常由服务器保存。QQ / 163 请根据网页版的实际行为选择，避免重复副本。").font(.system(size: 10))
              .foregroundStyle(Theme.muted)
            Toggle("启用此账号的自动同步", isOn: $enabled)
          }.textFieldStyle(.roundedBorder).font(.system(size: 12)).padding(.top, 10)
        }.font(.system(size: 12))
        HStack(spacing: 14) {
          Button {
            Task { await save() }
          } label: {
            HStack(spacing: 8) {
              if busy { ProgressView().controlSize(.small) }
              Text(authKind == "oauth" && account == nil ? "使用 Google 登录" : "保存并连接")
            }
          }.buttonStyle(OutlineButtonStyle(prominent: true)).disabled(busy || address.isEmpty)
          if authKind == "oauth" && account != nil {
            Button("重新授权") { Task { await save(forceLogin: true) } }.buttonStyle(
              OutlineButtonStyle()
            ).disabled(busy)
          }
        }
      }
      if !status.isEmpty {
        Text(status).font(.system(size: 12)).foregroundStyle(Theme.muted).textSelection(.enabled)
      }
      if let account {
        if let current = store.accountStatus[account.id] {
          Text(current).font(.system(size: 12)).foregroundStyle(Theme.muted)
        }
        Button("移除此邮箱", role: .destructive) { confirmRemoval = true }.buttonStyle(AreaButtonStyle()).font(
          .system(size: 12)
        ).padding(.top, 10)
      }
    }.onAppear {
      if let a = account {
        id = a.id
        name = a.name
        address = a.address
        provider = a.provider
        authKind = a.authKind
        imapHost = a.imapHost
        imapPort = Int(a.imapPort)
        smtpHost = a.smtpHost
        smtpPort = Int(a.smtpPort)
        enabled = a.enabled
        sentMode = a.sentMode
        color = a.color
      }
    }
    .confirmationDialog(
      "移除此邮箱及本地缓存？服务器上的邮件不会删除。", isPresented: $confirmRemoval, titleVisibility: .visible
    ) {
      Button("移除本地账号", role: .destructive) {
        if let account {
          Task {
            do {
              try await store.removeAccount(account)
              status = "账号已移除"
            } catch { status = userError(error) }
          }
        }
      }
    }
  }
  func field(_ label: String, text: Binding<String>, prompt: String) -> some View {
    VStack(alignment: .leading, spacing: 6) {
      Text(label).font(.system(size: 12)).foregroundStyle(Theme.muted)
      TextField(prompt, text: text).textFieldStyle(.roundedBorder)
    }
  }
  func applyPreset(_ p: String) {
    switch p {
    case "gmail":
      imapHost = "imap.gmail.com"
      smtpHost = "smtp.gmail.com"
      authKind = "oauth"
      color = "#226451"
    case "163":
      imapHost = "imap.163.com"
      smtpHost = "smtp.163.com"
      authKind = "password"
      color = "#BC795F"
    case "qq":
      imapHost = "imap.qq.com"
      smtpHost = "smtp.qq.com"
      authKind = "password"
      color = "#C19944"
    default:
      authKind = "password"
      color = "#6687B7"
    }
    imapPort = 993
    smtpPort = 465
  }
  func save(forceLogin: Bool = false) async {
    busy = true
    defer { busy = false }
    status = ""
    do {
      guard let imapPort = UInt16(exactly: imapPort), let smtpPort = UInt16(exactly: smtpPort),
        imapPort > 0, smtpPort > 0
      else { throw MailAppError.message("端口应在 1–65535 之间") }
      let a = Account(
        id: id, name: name.isEmpty ? address : name,
        address: address.trimmingCharacters(in: .whitespacesAndNewlines), provider: provider,
        imapHost: imapHost.trimmingCharacters(in: .whitespaces), imapPort: imapPort,
        smtpHost: smtpHost.trimmingCharacters(in: .whitespaces), smtpPort: smtpPort,
        authKind: authKind, color: color, enabled: enabled, sentMode: sentMode)
      if authKind == "oauth" && (account == nil || forceLogin) {
        status = "请在浏览器中完成 Google 授权…"
        try await GoogleOAuth.signIn(
          account: a, clientID: store.googleClientID,
          clientSecret: try await SecretStore.read("google-client-secret") ?? "")
      }
      if authKind != "oauth" && password.isEmpty && account == nil {
        throw MailAppError.message("请填写客户端授权码")
      }
      try await store.saveAccount(a, password: password)
      password = ""
      store.finishAccountSetup(a)
    } catch { status = userError(error) }
  }
}

struct TranslationEditor: View {
  @EnvironmentObject var store: MailStore
  @State var configuration: TranslationConfiguration
  @State private var apiKey = ""
  @State private var status = ""
  @State private var busy = false
  @State private var showLanguages = false
  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 18) {
        Text("翻译服务").font(.system(size: 24, weight: .semibold))
        Picker("翻译方式", selection: $configuration.engine) {
          Text("Chat 格式 LLM").tag("llm")
          Text("macOS 离线翻译").tag("system")
        }.pickerStyle(.segmented).labelsHidden()
        setting("服务名称", text: $configuration.name)
        if configuration.engine == "llm" {
          setting("Base URL", text: $configuration.baseURL)
          VStack(alignment: .leading, spacing: 6) {
            Text("API Key").font(.system(size: 12)).foregroundStyle(Theme.muted)
            SecureField("保存在 Keychain；已保存可留空", text: $apiKey).textFieldStyle(.roundedBorder)
          }
          setting("Model", text: $configuration.model)
          Text("可以填写兼容服务提供的模型 ID，无需依赖模型列表接口。").font(.system(size: 10)).foregroundStyle(Theme.muted)
        }
        Picker("目标语言", selection: $configuration.targetLanguage) {
          Text("简体中文").tag("简体中文")
          Text("繁體中文").tag("繁體中文")
          Text("English").tag("English")
          Text("日本語").tag("日本語")
        }.font(.system(size: 12))
        if configuration.engine == "llm" {
          VStack(alignment: .leading, spacing: 7) {
            Text("术语表").font(.system(size: 12)).foregroundStyle(Theme.muted)
            TextEditor(text: $configuration.glossary).font(.system(size: 12)).frame(height: 80)
              .padding(6).overlay(RoundedRectangle(cornerRadius: 6).stroke(Theme.line))
            Text("每行一条，例如：onboarding = 首次使用流程；Lightmail = 轻邮").font(.system(size: 10))
              .foregroundStyle(Theme.muted)
          }
          DisclosureGroup("兼容选项") {
            VStack(alignment: .leading, spacing: 12) {
              Toggle("使用流式响应", isOn: $configuration.stream)
              Picker("输出格式", selection: $configuration.outputFormat) {
                Text("提示词 JSON（兼容优先）").tag("prompt")
                Text("JSON mode").tag("json")
                Text("JSON Schema").tag("schema")
              }
              Stepper(
                "每批约 \(configuration.inputCharacters) 字符", value: $configuration.inputCharacters,
                in: 6000...30000, step: 3000)
            }.font(.system(size: 12)).padding(.top, 10)
          }.font(.system(size: 12))
          Text("点击翻译时，本封邮件的主题和正文（包含引用）将发送到此服务；附件和无关邮件不会发送。API Key 不会写入日志。").font(.system(size: 11))
            .foregroundStyle(Theme.muted).lineSpacing(4)
        } else {
          Button("下载 / 管理系统语言包…") { showLanguages = true }.buttonStyle(OutlineButtonStyle())
          Text("使用系统已支持的语言。首次使用可能需要下载语言包，翻译在设备上处理。").font(.system(size: 12)).foregroundStyle(
            Theme.muted)
        }
        HStack(spacing: 12) {
          Button("保存并设为默认") { Task { await save() } }.buttonStyle(
            OutlineButtonStyle(prominent: true))
          if configuration.engine == "llm" {
            Button {
              Task { await test() }
            } label: {
              HStack {
                if busy { ProgressView().controlSize(.small) }
                Text("测试连接")
              }
            }.buttonStyle(OutlineButtonStyle()).disabled(busy)
          }
        }
        if !status.isEmpty {
          Text(status).font(.system(size: 12)).foregroundStyle(Theme.muted).textSelection(.enabled)
        }
      }.padding(28)
    }.sheet(isPresented: $showLanguages) {
      SystemLanguagesView(targetCode: store.languageCode(configuration.targetLanguage))
    }
  }
  func setting(_ name: String, text: Binding<String>) -> some View {
    VStack(alignment: .leading, spacing: 6) {
      Text(name).font(.system(size: 12)).foregroundStyle(Theme.muted)
      TextField(name, text: text).textFieldStyle(.roundedBorder)
    }
  }
  func save() async {
    do {
      try validateTranslationConfiguration(configuration: configuration)
      if !apiKey.isEmpty { try await SecretStore.save(apiKey, for: "translation:\(configuration.id)") }
      if let index = store.translationConfigs.firstIndex(where: { $0.id == configuration.id }) {
        store.translationConfigs[index] = configuration
      } else {
        store.translationConfigs.append(configuration)
      }
      store.selectedTranslationID = configuration.id
      try await store.persistTranslationSettings()
      apiKey = ""
      status = "已保存为默认翻译服务"
    } catch { status = userError(error) }
  }
  func test() async {
    busy = true
    defer { busy = false }
    status = "正在用合成文本测试，不读取邮件…"
    do {
      let key =
        apiKey.isEmpty ? try await SecretStore.read("translation:\(configuration.id)") ?? "" : apiKey
      status = try await TranslationService.test(configuration: configuration, key: key)
    } catch { status = userError(error) }
  }
}
