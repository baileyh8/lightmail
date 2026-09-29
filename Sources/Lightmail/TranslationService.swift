import Foundation

// Swift convenience adapter for settings and platform rendering. All parsing,
// batching, protection, HTTP/SSE, validation and caching belong to the Rust core.
enum TranslationService {
  static func endpoint(_ base: String) throws -> URL {
    URL(string: try translationEndpoint(base: base))!
  }
  static func blocks(_ markdown: String) -> [TranslationBlock] { translationBlocks(markdown: markdown) }
  static func translate(subject: String, markdown: String, hash: String,
    configuration: TranslationConfiguration, key: String,
    progress: @escaping @Sendable (Int, Int, [TranslationBlock]) async -> Void
  ) async throws -> TranslationResult {
    try await TranslationClient(platform: MacPlatformServices()).translate(subject: subject,
      markdown: markdown, hash: hash, configuration: configuration, key: key,
      observer: CoreTranslationObserver(progress))
  }
  static func test(configuration: TranslationConfiguration, key: String) async throws -> String {
    try await TranslationClient(platform: MacPlatformServices()).test(configuration: configuration, key: key)
  }
}
