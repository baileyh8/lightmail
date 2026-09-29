import Foundation
import Security

struct KeychainStatusError: LocalizedError {
  let status: OSStatus
  var needsAuthorization: Bool {
    [errSecInteractionNotAllowed, errSecAuthFailed, errSecUserCanceled].contains(status)
  }
  var errorDescription: String? {
    needsAuthorization ? "凭证需要授权，请点击轻邮顶部的「授权访问」" : "Keychain 操作失败（\(status)）"
  }
}

protocol SecretBackend {
  func read(_ key: String, interactive: Bool) throws -> String?
  func save(_ value: String, for key: String) throws
  func remove(_ key: String) throws
}

struct MacKeychainBackend: SecretBackend {
  private func query(_ key: String) -> [String: Any] {
    [kSecClass as String: kSecClassGenericPassword,
     kSecAttrService as String: "com.bailey.lightmail.credentials", kSecAttrAccount as String: key]
  }
  // This app uses the file-based login keychain. AuthenticationUIFail alone is
  // insufficient for legacy ACL prompts. All our SecItem operations are serial
  // synchronous calls inside SessionSecrets, so the process setting is scoped
  // and restored before another operation can run, with no await in this scope.
  private func interaction<T>(_ allowed: Bool, _ operation: () throws -> T) throws -> T {
    var previous = DarwinBoolean(true)
    let getStatus = SecKeychainGetUserInteractionAllowed(&previous)
    guard getStatus == errSecSuccess else { throw KeychainStatusError(status: getStatus) }
    let setStatus = SecKeychainSetUserInteractionAllowed(allowed)
    guard setStatus == errSecSuccess else { throw KeychainStatusError(status: setStatus) }
    defer { SecKeychainSetUserInteractionAllowed(previous.boolValue) }
    return try operation()
  }
  func read(_ key: String, interactive: Bool) throws -> String? {
    try interaction(interactive) {
      var request = query(key)
      request[kSecReturnData as String] = true
      request[kSecMatchLimit as String] = kSecMatchLimitOne
      var result: CFTypeRef?
      let status = SecItemCopyMatching(request as CFDictionary, &result)
      if status == errSecItemNotFound { return nil }
      guard status == errSecSuccess else { throw KeychainStatusError(status: status) }
      guard let data = result as? Data, let value = String(data: data, encoding: .utf8) else {
        throw MailAppError.message("Keychain 凭证格式无效，请重新保存")
      }
      return value
    }
  }
  func save(_ value: String, for key: String) throws {
    try interaction(false) {
      let request = query(key), data = Data(value.utf8)
      let status = SecItemUpdate(request as CFDictionary, [kSecValueData as String: data] as CFDictionary)
      if status == errSecItemNotFound {
        var item = request
        item[kSecValueData as String] = data
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let added = SecItemAdd(item as CFDictionary, nil)
        guard added == errSecSuccess else { throw KeychainStatusError(status: added) }
      } else if status != errSecSuccess { throw KeychainStatusError(status: status) }
    }
  }
  func remove(_ key: String) throws {
    try interaction(false) {
      let status = SecItemDelete(query(key) as CFDictionary)
      guard status == errSecSuccess || status == errSecItemNotFound else {
        throw KeychainStatusError(status: status)
      }
    }
  }
}

final class SessionSecrets: @unchecked Sendable {
  private let lock = NSRecursiveLock()
  private enum Cached { case value(String), missing }
  private var values: [String: Cached] = [:]
  private var blocked: Set<String> = []
  private let backend: SecretBackend
  private let changed: () -> Void
  init(backend: SecretBackend, changed: @escaping () -> Void = {}) {
    self.backend = backend; self.changed = changed
  }
  func pendingKeys() -> [String] { lock.lock(); defer { lock.unlock() }; return blocked.sorted() }
  func read(_ key: String, interactive: Bool = false) throws -> String? {
    lock.lock(); defer { lock.unlock() }
    if !interactive {
      if blocked.contains(key) { throw KeychainStatusError(status: errSecInteractionNotAllowed) }
      if let cached = values[key] {
        switch cached { case .value(let value): return value; case .missing: return nil }
      }
    }
    do {
      let value = try backend.read(key, interactive: interactive)
      values[key] = value.map(Cached.value) ?? .missing
      if blocked.remove(key) != nil { changed() }
      return value
    } catch { record(error, key: key); throw error }
  }
  func save(_ value: String, for key: String) throws {
    lock.lock(); defer { lock.unlock() }
    do {
      try backend.save(value, for: key)
      values[key] = .value(value)
      if blocked.remove(key) != nil { changed() }
    } catch { record(error, key: key); throw error }
  }
  func remove(_ key: String) throws {
    lock.lock(); defer { lock.unlock() }
    do {
      try backend.remove(key)
      values[key] = nil
      if blocked.remove(key) != nil { changed() }
    } catch { record(error, key: key); throw error }
  }
  private func record(_ error: Error, key: String) {
    if let error = error as? KeychainStatusError, error.needsAuthorization {
      values[key] = nil
      if blocked.insert(key).inserted { changed() }
    }
  }
}

enum SecretStore {
  static let authorizationChanged = Notification.Name("LightmailCredentialAuthorizationChanged")
  private static let vault = SessionSecrets(backend: MacKeychainBackend()) {
    NotificationCenter.default.post(name: authorizationChanged, object: nil)
  }
  static func readSync(_ key: String) throws -> String? { try vault.read(key) }
  static func saveSync(_ value: String, for key: String) throws { try vault.save(value, for: key) }
  static func removeSync(_ key: String) throws { try vault.remove(key) }
  static func pendingKeys() async -> [String] { vault.pendingKeys() }
  static func read(_ key: String) async throws -> String? { try await Task.detached { try readSync(key) }.value }
  // Called only from the user's explicit authorization button.
  static func authorize(_ key: String) async throws { _ = try await Task.detached { try vault.read(key, interactive: true) }.value }
  static func save(_ value: String, for key: String) async throws { try await Task.detached { try saveSync(value, for: key) }.value }
  static func remove(_ key: String) async throws { try await Task.detached { try removeSync(key) }.value }
}
