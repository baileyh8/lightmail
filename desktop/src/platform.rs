//! Windows-only capabilities; application policy stays in lightmail_core.
use lightmail_core::fail;
use lightmail_core::{PlatformServices, ProxyRoute, Result};
#[cfg(any(not(windows), test))]
use std::{collections::HashMap, sync::Mutex};

pub struct DesktopPlatform {
    vault: Box<dyn Vault + Send + Sync>,
}

impl Default for DesktopPlatform {
    fn default() -> Self {
        #[cfg(windows)]
        let vault: Box<dyn Vault + Send + Sync> = Box::new(CredentialManager);
        // Development previews on macOS never touch the native Swift app's vault.
        #[cfg(not(windows))]
        let vault: Box<dyn Vault + Send + Sync> = Box::new(MemoryVault::default());
        Self { vault }
    }
}

/// Raw per-entry storage. Values larger than one Credential Manager entry are
/// split by `write_value`, so an implementation never receives an oversized value.
trait Vault {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

#[cfg(windows)]
const SERVICE: &str = "com.bailey.lightmail.credentials";

#[cfg(windows)]
struct CredentialManager;

#[cfg(windows)]
impl Vault for CredentialManager {
    fn get(&self, key: &str) -> Result<Option<String>> {
        let entry =
            keyring::Entry::new(SERVICE, key).map_err(|_| fail("无法访问 Windows 凭据管理器"))?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(fail("Windows 凭据读取失败，请重新保存账号授权")),
        }
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        keyring::Entry::new(SERVICE, key)
            .and_then(|e| e.set_password(value))
            .map_err(|_| fail("Windows 凭据保存失败"))
    }
    fn delete(&self, key: &str) -> Result<()> {
        match keyring::Entry::new(SERVICE, key).and_then(|e| e.delete_credential()) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(fail("Windows 凭据移除失败")),
        }
    }
}

#[cfg(any(not(windows), test))]
#[derive(Default)]
struct MemoryVault(Mutex<HashMap<String, String>>);

#[cfg(any(not(windows), test))]
impl Vault for MemoryVault {
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

// A Credential Manager entry holds at most 2560 bytes and keyring stores UTF-16,
// so one entry fits 1280 code units. Longer values, such as large OAuth tokens,
// are split: the main entry then holds a manifest naming one generation of parts.
// Any value starting with the manifest prefix is itself stored split, so reading
// the main entry is never ambiguous.
const INLINE_UNITS: usize = 1200;
const PART_UNITS: usize = 1000;
const CHUNKS: &str = "lightmail-chunks:v1:";

fn part_key(key: &str, generation: &str, index: usize) -> String {
    format!("{key}#{generation}.{index}")
}

fn manifest(value: &str) -> Option<(&str, usize)> {
    let (generation, count) = value.strip_prefix(CHUNKS)?.split_once(':')?;
    Some((generation, count.parse().ok()?))
}

fn split_utf16(value: &str, units: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut used = 0;
    for c in value.chars() {
        if used + c.len_utf16() > units {
            parts.push(std::mem::take(&mut current));
            used = 0;
        }
        current.push(c);
        used += c.len_utf16();
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

fn read_value(vault: &dyn Vault, key: &str) -> Result<Option<String>> {
    let Some(value) = vault.get(key)? else {
        return Ok(None);
    };
    let Some((generation, count)) = manifest(&value) else {
        return Ok(Some(value));
    };
    let mut joined = String::new();
    for index in 1..=count {
        let part = vault
            .get(&part_key(key, generation, index))?
            .ok_or_else(|| fail("Windows 凭据不完整，请重新保存账号授权"))?;
        joined.push_str(&part);
    }
    Ok(Some(joined))
}

fn remove_parts(vault: &dyn Vault, key: &str, previous: Option<&str>) {
    if let Some((generation, count)) = previous.and_then(manifest) {
        for index in 1..=count {
            let _ = vault.delete(&part_key(key, generation, index));
        }
    }
}

// New parts are written under a fresh generation before the manifest switches,
// so an interrupted write leaves only unreferenced parts, never a torn secret.
fn write_value(vault: &dyn Vault, key: &str, value: &str) -> Result<()> {
    let previous = vault.get(key).ok().flatten();
    if value.encode_utf16().count() <= INLINE_UNITS && !value.starts_with(CHUNKS) {
        vault.set(key, value)?;
    } else {
        let generation = uuid::Uuid::new_v4().simple().to_string();
        let parts = split_utf16(value, PART_UNITS);
        let discard = |written: usize| {
            for index in 1..=written {
                let _ = vault.delete(&part_key(key, &generation, index));
            }
        };
        for (index, part) in parts.iter().enumerate() {
            if let Err(error) = vault.set(&part_key(key, &generation, index + 1), part) {
                discard(index);
                return Err(error);
            }
        }
        if let Err(error) = vault.set(key, &format!("{CHUNKS}{generation}:{}", parts.len())) {
            discard(parts.len());
            return Err(error);
        }
    }
    remove_parts(vault, key, previous.as_deref());
    Ok(())
}

fn remove_value(vault: &dyn Vault, key: &str) -> Result<()> {
    let previous = vault.get(key).ok().flatten();
    vault.delete(key)?;
    remove_parts(vault, key, previous.as_deref());
    Ok(())
}

#[cfg(windows)]
pub fn initial_window_bounds() -> Option<gpui::Bounds<gpui::Pixels>> {
    #[repr(C)]
    #[derive(Default)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn SystemParametersInfoW(action: u32, param: u32, value: *mut Rect, flags: u32) -> i32;
        fn GetDpiForSystem() -> u32;
    }
    let mut area = Rect::default();
    if unsafe { SystemParametersInfoW(0x0030, 0, &mut area, 0) } == 0 {
        return None;
    }
    let scale = (unsafe { GetDpiForSystem() } as f32 / 96.).max(1.);
    let width = (area.right - area.left) as f32 / scale;
    let height = (area.bottom - area.top) as f32 / scale;
    let size = gpui::size(
        gpui::px(1280f32.min(width - 32.)),
        gpui::px(800f32.min(height - 64.)),
    );
    let origin = gpui::point(
        gpui::px(area.left as f32 / scale + (width - f32::from(size.width)) / 2.),
        gpui::px(area.top as f32 / scale + (height - f32::from(size.height) - 32.) / 2.),
    );
    Some(gpui::Bounds::new(origin, size))
}
impl PlatformServices for DesktopPlatform {
    fn read_secret(&self, key: String) -> Result<Option<String>> {
        read_value(self.vault.as_ref(), &key)
    }
    fn write_secret(&self, key: String, value: String) -> Result<()> {
        write_value(self.vault.as_ref(), &key, &value)
    }
    fn remove_secret(&self, key: String) -> Result<()> {
        remove_value(self.vault.as_ref(), &key)
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
        assert!(platform.read_secret(key.clone()).unwrap().is_none());
        // Longer than one Credential Manager entry, with surrogate pairs at part edges.
        let long = "令牌🙂token".repeat(400);
        platform.write_secret(key.clone(), long.clone()).unwrap();
        assert_eq!(platform.read_secret(key.clone()).unwrap(), Some(long));
        platform.remove_secret(key.clone()).unwrap();
        assert!(platform.read_secret(key).unwrap().is_none());
    }

    // Rejects what Credential Manager would reject: more than 2560 bytes of UTF-16.
    #[derive(Default)]
    struct LimitedVault(MemoryVault);
    impl Vault for LimitedVault {
        fn get(&self, key: &str) -> Result<Option<String>> {
            self.0.get(key)
        }
        fn set(&self, key: &str, value: &str) -> Result<()> {
            if value.encode_utf16().count() * 2 > 2560 {
                return Err(fail("entry too long"));
            }
            self.0.set(key, value)
        }
        fn delete(&self, key: &str) -> Result<()> {
            self.0.delete(key)
        }
    }

    #[test]
    fn long_secrets_are_split_and_every_generation_is_cleaned_up() {
        let vault = LimitedVault::default();
        let entries = |vault: &LimitedVault| vault.0 .0.lock().unwrap().len();
        write_value(&vault, "account:a", "short").unwrap();
        assert_eq!(entries(&vault), 1);
        let long = "🙂".repeat(3000) + "tail";
        write_value(&vault, "account:a", &long).unwrap();
        assert_eq!(
            read_value(&vault, "account:a").unwrap().as_deref(),
            Some(long.as_str())
        );
        let parts = entries(&vault) - 1;
        assert!(
            parts >= 7,
            "6004 UTF-16 units need several parts, got {parts}"
        );
        // Replacing a split value removes the previous generation of parts.
        let other = "x".repeat(5000);
        write_value(&vault, "account:a", &other).unwrap();
        assert_eq!(read_value(&vault, "account:a").unwrap(), Some(other));
        assert_eq!(entries(&vault), 1 + 5);
        write_value(&vault, "account:a", "short again").unwrap();
        assert_eq!(entries(&vault), 1);
        // A value that looks like a manifest is stored split, so it reads back verbatim.
        let lookalike = format!("{CHUNKS}abc:2");
        write_value(&vault, "account:b", &lookalike).unwrap();
        assert_eq!(read_value(&vault, "account:b").unwrap(), Some(lookalike));
        remove_value(&vault, "account:a").unwrap();
        remove_value(&vault, "account:b").unwrap();
        assert_eq!(entries(&vault), 0);
        assert_eq!(read_value(&vault, "account:a").unwrap(), None);
    }

    #[test]
    fn a_missing_part_is_an_error_not_a_truncated_secret() {
        let vault = MemoryVault::default();
        write_value(&vault, "account:c", &"y".repeat(3000)).unwrap();
        let (generation, _) = {
            let main = vault.get("account:c").unwrap().unwrap();
            let (generation, count) = manifest(&main).unwrap();
            (generation.to_string(), count)
        };
        vault
            .delete(&part_key("account:c", &generation, 2))
            .unwrap();
        assert!(read_value(&vault, "account:c").is_err());
    }

    #[test]
    fn a_failed_split_write_keeps_the_previous_value() {
        struct FailsAfter(MemoryVault, Mutex<u32>);
        impl Vault for FailsAfter {
            fn get(&self, key: &str) -> Result<Option<String>> {
                self.0.get(key)
            }
            fn set(&self, key: &str, value: &str) -> Result<()> {
                let mut left = self.1.lock().unwrap();
                if *left == 0 {
                    return Err(fail("vault unavailable"));
                }
                *left -= 1;
                self.0.set(key, value)
            }
            fn delete(&self, key: &str) -> Result<()> {
                self.0.delete(key)
            }
        }
        let vault = FailsAfter(MemoryVault::default(), Mutex::new(1));
        write_value(&vault, "account:d", "old").unwrap();
        *vault.1.lock().unwrap() = 2;
        assert!(write_value(&vault, "account:d", &"z".repeat(4000)).is_err());
        assert_eq!(
            read_value(&vault, "account:d").unwrap().as_deref(),
            Some("old")
        );
        assert_eq!(
            vault.0 .0.lock().unwrap().len(),
            1,
            "partial parts are discarded"
        );
    }
}
