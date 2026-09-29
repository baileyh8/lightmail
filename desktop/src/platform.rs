//! Windows-only capabilities; application policy stays in lightmail_core.
#[cfg(any(windows, test))]
use lightmail_core::fail;
use lightmail_core::{PlatformServices, ProxyRoute, Result};
#[cfg(not(windows))]
use std::{collections::HashMap, sync::Mutex};

#[derive(Default)]
pub struct DesktopPlatform {
    // Development previews on macOS never touch the native Swift app's vault.
    #[cfg(not(windows))]
    preview_secrets: Mutex<HashMap<String, String>>,
}
impl PlatformServices for DesktopPlatform {
    fn read_secret(&self, key: String) -> Result<Option<String>> {
        #[cfg(windows)]
        {
            let entry = keyring::Entry::new("com.bailey.lightmail.credentials", &key)
                .map_err(|_| fail("无法访问 Windows 凭据管理器"))?;
            match entry.get_password() {
                Ok(value) => Ok(Some(value)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(_) => Err(fail("Windows 凭据读取失败，请重新保存账号授权")),
            }
        }
        #[cfg(not(windows))]
        {
            Ok(self.preview_secrets.lock().unwrap().get(&key).cloned())
        }
    }
    fn write_secret(&self, key: String, value: String) -> Result<()> {
        #[cfg(windows)]
        {
            keyring::Entry::new("com.bailey.lightmail.credentials", &key)
                .and_then(|e| e.set_password(&value))
                .map_err(|_| fail("Windows 凭据保存失败"))?;
        }
        #[cfg(not(windows))]
        {
            self.preview_secrets.lock().unwrap().insert(key, value);
        }
        Ok(())
    }
    fn remove_secret(&self, key: String) -> Result<()> {
        #[cfg(windows)]
        {
            match keyring::Entry::new("com.bailey.lightmail.credentials", &key)
                .and_then(|e| e.delete_credential())
            {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(_) => return Err(fail("Windows 凭据移除失败")),
            }
        }
        #[cfg(not(windows))]
        {
            self.preview_secrets.lock().unwrap().remove(&key);
        }
        Ok(())
    }
    fn proxy_for(&self, host: String) -> Result<ProxyRoute> {
        if matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1" | "[::1]") {
            return Ok(direct());
        }
        #[cfg(windows)]
        {
            use winreg::{enums::HKEY_CURRENT_USER, RegKey};
            let root = RegKey::predef(HKEY_CURRENT_USER);
            let Ok(settings) =
                root.open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings")
            else {
                return Ok(direct());
            };
            let enabled = settings.get_value::<u32, _>("ProxyEnable").unwrap_or(0) != 0;
            if !enabled {
                if settings
                    .get_value::<String, _>("AutoConfigURL")
                    .is_ok_and(|s| !s.is_empty())
                {
                    return Err(fail(
                        "当前系统使用 PAC 自动代理，请改用系统固定 HTTP 或 SOCKS 代理后重试",
                    ));
                }
                return Ok(direct());
            }
            let bypass = settings
                .get_value::<String, _>("ProxyOverride")
                .unwrap_or_default();
            let server = settings
                .get_value::<String, _>("ProxyServer")
                .unwrap_or_default();
            return resolve_proxy(&host, &server, &bypass);
        }
        #[cfg(not(windows))]
        {
            let _ = host;
            Ok(direct())
        }
    }
}
fn direct() -> ProxyRoute {
    ProxyRoute {
        kind: "direct".into(),
        host: String::new(),
        port: 0,
    }
}
#[cfg(any(windows, test))]
fn wildcard(pattern: &str, value: &str) -> bool {
    let mut remainder = value;
    let mut first = true;
    for part in pattern.split('*') {
        if part.is_empty() {
            first = false;
            continue;
        }
        let Some(index) = remainder.find(part) else {
            return false;
        };
        if first && index != 0 {
            return false;
        }
        remainder = &remainder[index + part.len()..];
        first = false;
    }
    pattern.ends_with('*') || remainder.is_empty()
}
#[cfg(any(windows, test))]
fn resolve_proxy(host: &str, server: &str, bypass: &str) -> Result<ProxyRoute> {
    let host = host.to_lowercase();
    if bypass
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|p| {
            if p == "<local>" {
                !host.contains('.')
            } else {
                wildcard(&p.to_lowercase(), &host)
            }
        })
    {
        return Ok(direct());
    }
    let routes: Vec<_> = server
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    let selected = routes
        .iter()
        .find(|s| s.starts_with("socks="))
        .or_else(|| routes.iter().find(|s| s.starts_with("https=")))
        .or_else(|| routes.iter().find(|s| s.starts_with("http=")))
        .or_else(|| routes.iter().find(|s| !s.contains('=')))
        .ok_or_else(|| fail("系统代理配置无效"))?;
    let (kind, address) = if let Some(s) = selected.strip_prefix("socks=") {
        ("socks5", s)
    } else {
        (
            "http",
            selected.split_once('=').map(|(_, s)| s).unwrap_or(selected),
        )
    };
    let address = if address.contains("://") {
        address.to_string()
    } else {
        format!("http://{address}")
    };
    let url = url::Url::parse(&address).map_err(|_| fail("系统代理地址无效"))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(fail("暂不支持需要单独凭证的系统代理"));
    }
    Ok(ProxyRoute {
        kind: kind.into(),
        host: url
            .host_str()
            .ok_or_else(|| fail("系统代理缺少地址"))?
            .trim_matches(['[', ']'])
            .into(),
        port: url
            .port()
            .unwrap_or(if kind == "socks5" { 1080 } else { 80 }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_proxy_routes_and_bypass() {
        let route = resolve_proxy(
            "imap.gmail.com",
            "http=127.0.0.1:7897;https=127.0.0.1:7897;socks=127.0.0.1:7898",
            "<local>;*.example.com",
        )
        .unwrap();
        assert_eq!(route.kind, "socks5");
        assert_eq!(route.port, 7898);
        assert_eq!(
            resolve_proxy("mail.example.com", "127.0.0.1:7897", "*.example.com")
                .unwrap()
                .kind,
            "direct"
        );
        assert_eq!(
            resolve_proxy("imap.gmail.com", "[::1]:7897", "")
                .unwrap()
                .host,
            "::1"
        );
    }
    #[test]
    fn credential_roundtrip_uses_unique_synthetic_entry() {
        let platform = DesktopPlatform::default();
        let key = format!("acceptance:{}", uuid::Uuid::new_v4());
        platform
            .write_secret(key.clone(), "synthetic-only".into())
            .unwrap();
        assert_eq!(
            platform.read_secret(key.clone()).unwrap().as_deref(),
            Some("synthetic-only")
        );
        platform.remove_secret(key.clone()).unwrap();
        assert!(platform.read_secret(key).unwrap().is_none());
    }
}
