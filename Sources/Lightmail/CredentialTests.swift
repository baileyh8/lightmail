import Foundation
import Security

// Uses only synthetic values and an injected backend: never the user's keychain.
enum CredentialTests {
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
          for _ in 0..<100 { group.addTask { _ = try await vault.read("account:fixture") } }
          try await group.waitForAll()
        }
        check(backend.reads == 1 && backend.interactiveReads == 0, "100 concurrent requests reuse one silent read")
        try await vault.save("fixture-updated", for: "account:fixture")
        let updated = try await vault.read("account:fixture")
        check(updated == "fixture-updated" && backend.reads == 1, "save refreshes the in-memory value")
        try await vault.remove("account:fixture")
        let removed = try await vault.read("account:fixture")
        check(removed == nil && backend.reads == 2, "remove invalidates the session value")

        let locked = Backend()
        locked.protected = true
        let gated = SessionSecrets(backend: locked)
        for _ in 0..<100 { _ = try? await gated.read("account:fixture") }
        let pending = await gated.pendingKeys()
        check(locked.reads == 1 && locked.interactiveReads == 0 && pending.count == 1,
              "blocked background retries never present authentication UI")
        locked.rejectAuthorization = true
        _ = try? await gated.read("account:fixture", interactive: true)
        for _ in 0..<100 { _ = try? await gated.read("account:fixture") }
        check(locked.reads == 2 && locked.interactiveReads == 1,
              "user cancellation does not trigger another automatic prompt")
        locked.rejectAuthorization = false
        _ = try await gated.read("account:fixture", interactive: true)
        for _ in 0..<100 { _ = try await gated.read("account:fixture") }
        let unblocked = await gated.pendingKeys()
        check(locked.reads == 3 && locked.interactiveReads == 2 && unblocked.isEmpty,
              "explicit retry authorizes once then reuses the session")
        _ = try? await gated.read("translation:fixture")
        let separate = await gated.pendingKeys()
        check(separate == ["translation:fixture"], "different credentials retain separate authorization")
      } catch { print("FAIL credential fixtures: \(userError(error))"); exit(1) }
      finished.signal()
    }
    guard finished.wait(timeout: .now() + 10) == .success else { print("FAIL credential fixtures timed out"); exit(1) }
    return 7
  }
}
