import CFNetwork
import Foundation

struct MailProxyRoute: Sendable {
  let kind: String
  let host: String
  let port: UInt16

  static func resolve(host: String, settings supplied: [String: Any]? = nil) throws -> MailProxyRoute {
    let direct = MailProxyRoute(kind: "direct", host: "", port: 0)
    guard let settings = supplied ?? (CFNetworkCopySystemProxySettings()?.takeRetainedValue() as? [String: Any]) else {
      return direct
    }
    var components = URLComponents()
    components.scheme = "https"
    components.host = host
    guard let url = components.url else { throw MailAppError.message("邮件服务器地址无效") }
    let routes = CFNetworkCopyProxiesForURL(url as CFURL, settings as CFDictionary).takeRetainedValue() as NSArray
    guard let first = routes.firstObject as? [String: Any], let firstType = first[kCFProxyTypeKey as String] as? String else {
      return direct
    }
    if firstType == kCFProxyTypeNone as String { return direct }
    // These are raw mail sockets, not web requests. Prefer the system SOCKS route
    // when both exist, after honoring the system's destination bypass decision.
    let available = routes.compactMap { $0 as? [String: Any] }
    let route = available.first { ($0[kCFProxyTypeKey as String] as? String) == kCFProxyTypeSOCKS as String } ?? first
    let type = route[kCFProxyTypeKey as String] as? String ?? ""
    let kind: String
    if type == kCFProxyTypeSOCKS as String { kind = "socks5" }
    else if type == kCFProxyTypeHTTP as String || type == kCFProxyTypeHTTPS as String { kind = "http" }
    else { throw MailAppError.message("当前系统代理类型暂不支持邮件连接，请使用系统 HTTP 或 SOCKS 代理") }
    guard let proxyHost = route[kCFProxyHostNameKey as String] as? String,
      let number = route[kCFProxyPortNumberKey as String] as? NSNumber,
      let proxyPort = UInt16(exactly: number.intValue), proxyPort > 0,
      !proxyHost.isEmpty else { throw MailAppError.message("系统代理地址或端口无效") }
    return MailProxyRoute(kind: kind, host: proxyHost, port: proxyPort)
  }
}
