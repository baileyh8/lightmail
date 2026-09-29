import AppKit

// Synchronous foreign callbacks run on Rust workers, never prompt for Keychain access.
final class MacPlatformServices: PlatformServices, @unchecked Sendable {
  private func bridge<T>(_ operation: () throws -> T) throws -> T {
    do { return try operation() } catch { throw MailError.Failure(message: userError(error)) }
  }
  func readSecret(key: String) throws -> String? { try bridge { try SecretStore.readSync(key) } }
  func writeSecret(key: String, value: String) throws { try bridge { try SecretStore.saveSync(value, for: key) } }
  func removeSecret(key: String) throws { try bridge { try SecretStore.removeSync(key) } }
  func proxyFor(host: String) throws -> ProxyRoute {
    try bridge {
      let route = try MailProxyRoute.resolve(host: host)
      return ProxyRoute(kind: route.kind, host: route.host, port: route.port)
    }
  }
}

final class CoreApplicationObserver: ApplicationObserver, @unchecked Sendable {
  weak var store: MailStore?
  let generation: UUID
  init(generation: UUID) { self.generation = generation }
  func changed(event: ApplicationEvent) {
    Task { @MainActor [weak self] in
      guard let self else { return }
      await store?.coreChanged(event, generation: generation)
    }
  }
}

final class CoreTranslationObserver: TranslationObserver, @unchecked Sendable {
  private let update: @Sendable (Int, Int, [TranslationBlock]) async -> Void
  init(_ update: @escaping @Sendable (Int, Int, [TranslationBlock]) async -> Void) { self.update = update }
  func progress(done: UInt32, total: UInt32, blocks: [TranslationBlock]) {
    Task { await update(Int(done), Int(total), blocks) }
  }
}
