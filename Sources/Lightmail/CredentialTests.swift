import Foundation
import Security

// Uses only synthetic values and an injected backend: never the user's keychain.
enum CredentialTests {
  @MainActor static func runAuthorizationPresentation() -> Int {
    let directory = FileManager.default.temporaryDirectory
      .appendingPathComponent("lightmail-credential-presentation-\(UUID().uuidString)")
    defer { try? FileManager.default.removeItem(at: directory) }
    let backend = Backend()
    backend.protected = true
    let vault = SessionSecrets(backend: backend) {
      NotificationCenter.default.post(name: SecretStore.authorizationChanged, object: nil)
    }
    var store: MailStore? = MailStore(directory: directory, startAutomatically: false,
                                    credentialRequestSource: { vault.pendingKeys() })
    func waitFor(_ name: String, _ condition: () -> Bool) {
      let deadline = Date().addingTimeInterval(3)
      while !condition() && Date() < deadline {
        RunLoop.current.run(until: Date().addingTimeInterval(0.01))
      }
      guard condition() else { print("FAIL credentials: \(name)"); exit(1) }
      print("PASS credentials: \(name)")
    }
    // Deliberately no RootView or SettingsView: reproduce a startup worker failure.
    DispatchQueue.global().async { _ = try? vault.read("account:fixture") }
    waitFor("startup credential failure exposes authorization without a mounted view") {
      store?.credentialRequests == ["account:fixture"] && backend.interactiveReads == 0
    }
    DispatchQueue.global().async { _ = try? vault.read("account:fixture", interactive: true) }
    waitFor("successful explicit authorization clears the presentation state") {
      store?.credentialRequests.isEmpty == true
    }
    weak var releasedStore = store
    store = nil
    waitFor("credential observer does not retain the mail store") { releasedStore == nil }
    return 3
  }

  final class Backend: SecretBackend {
    var value: String? = "fixture-secret"
    var reads = 0
    var interactiveReads = 0
    var protected = false
    var rejectAuthorization = false
    func read(_ key: String, interactive: Bool) throws -> String? {
      reads += 1
      if interactive { interactiveReads += 1 }
      if protected && (!interactive || rejectAuthorization) {
        throw KeychainStatusError(status: interactive ? errSecUserCanceled : errSecInteractionNotAllowed)
      }
      return value
    }
    func save(_ value: String, for key: String) throws { self.value = value }
    func remove(_ key: String) throws { value = nil }
  }
  static func run() -> Int {
    let finished = DispatchSemaphore(value: 0)
    Task.detached {
      func check(_ condition: Bool, _ name: String) {
        if !condition { print("FAIL credentials: \(name)"); exit(1) }
        print("PASS credentials: \(name)")
      }
      do {
        let backend = Backend()
        let vault = SessionSecrets(backend: backend)
        try await withThrowingTaskGroup(of: Void.self) { group in
          for _ in 0..<100 { group.addTask { _ = try vault.read("account:fixture") } }
          try await group.waitForAll()
        }
        check(backend.reads == 1 && backend.interactiveReads == 0, "100 concurrent requests reuse one silent read")
        try vault.save("fixture-updated", for: "account:fixture")
        let updated = try vault.read("account:fixture")
        check(updated == "fixture-updated" && backend.reads == 1, "save refreshes the in-memory value")
        try vault.remove("account:fixture")
        let removed = try vault.read("account:fixture")
        check(removed == nil && backend.reads == 2, "remove invalidates the session value")

        let locked = Backend()
        locked.protected = true
        let gated = SessionSecrets(backend: locked)
        for _ in 0..<100 { _ = try? gated.read("account:fixture") }
        let pending = gated.pendingKeys()
        check(locked.reads == 1 && locked.interactiveReads == 0 && pending.count == 1,
              "blocked background retries never present authentication UI")
        locked.rejectAuthorization = true
        _ = try? gated.read("account:fixture", interactive: true)
        for _ in 0..<100 { _ = try? gated.read("account:fixture") }
        check(locked.reads == 2 && locked.interactiveReads == 1,
              "user cancellation does not trigger another automatic prompt")
        locked.rejectAuthorization = false
        _ = try gated.read("account:fixture", interactive: true)
        for _ in 0..<100 { _ = try gated.read("account:fixture") }
        let unblocked = gated.pendingKeys()
        check(locked.reads == 3 && locked.interactiveReads == 2 && unblocked.isEmpty,
              "explicit retry authorizes once then reuses the session")
        _ = try? gated.read("translation:fixture")
        let separate = gated.pendingKeys()
        check(separate == ["translation:fixture"], "different credentials retain separate authorization")
      } catch { print("FAIL credential fixtures: \(userError(error))"); exit(1) }
      finished.signal()
    }
    guard finished.wait(timeout: .now() + 10) == .success else { print("FAIL credential fixtures timed out"); exit(1) }
    return 7
  }
}
