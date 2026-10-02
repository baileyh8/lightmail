//! Account-owned network policy. Rust-only until upstream exports the settings UI.
use crate::{models::*, platform::*, MailEngine};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountProxyMode {
    #[default]
    System,
    Direct,
    Http,
    Socks5,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountProxySettings {
    pub mode: AccountProxyMode,
    pub host: String,
    pub port: u16,
}

impl AccountProxySettings {
    pub fn validated(mut self) -> Result<Self> {
        if matches!(
            self.mode,
            AccountProxyMode::System | AccountProxyMode::Direct
        ) {
            self.host.clear();
            self.port = 0;
            return Ok(self);
        }
        let host = self.host.trim();
        if self.port == 0
            || host.is_empty()
            || host.chars().any(char::is_whitespace)
            || self.host.chars().any(char::is_control)
            || host.contains(['/', '@', '?', '#', '\\'])
        {
            return Err(fail(
                "请填写有效的代理主机和端口；主机中不包含协议、路径或凭证",
            ));
        }
        self.host = if let Ok(ip) = host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
            ip.to_string()
        } else {
            if host.contains(':') || host.contains(['[', ']']) {
                return Err(fail("代理主机格式无效，请将主机和端口分别填写"));
            }
            url::Host::parse(host)
                .map_err(|_| fail("代理主机格式无效"))?
                .to_string()
        };
        Ok(self)
    }

    pub fn route(&self, platform: &dyn PlatformServices, host: String) -> Result<ProxyRoute> {
        self.explicit_route()
            .map(Ok)
            .unwrap_or_else(|| platform.proxy_for(host))
    }

    fn explicit_route(&self) -> Option<ProxyRoute> {
        Some(match self.mode {
            AccountProxyMode::System => return None,
            AccountProxyMode::Direct => ProxyRoute {
                kind: "direct".into(),
                host: String::new(),
                port: 0,
            },
            AccountProxyMode::Http | AccountProxyMode::Socks5 => ProxyRoute {
                kind: if self.mode == AccountProxyMode::Http {
                    "http"
                } else {
                    "socks5"
                }
                .into(),
                host: self.host.clone(),
                port: self.port,
            },
        })
    }

    pub fn scoped_platform(
        self,
        platform: Arc<dyn PlatformServices>,
    ) -> Result<Arc<dyn PlatformServices>> {
        Ok(Arc::new(AccountPlatform {
            settings: self.validated()?,
            platform,
        }))
    }
}

struct AccountPlatform {
    settings: AccountProxySettings,
    platform: Arc<dyn PlatformServices>,
}
impl PlatformServices for AccountPlatform {
    fn read_secret(&self, key: String) -> Result<Option<String>> {
        self.platform.read_secret(key)
    }
    fn write_secret(&self, key: String, value: String) -> Result<()> {
        self.platform.write_secret(key, value)
    }
    fn remove_secret(&self, key: String) -> Result<()> {
        self.platform.remove_secret(key)
    }
    fn proxy_for(&self, host: String) -> Result<ProxyRoute> {
        self.settings.route(self.platform.as_ref(), host)
    }
}

pub(crate) fn setting_key(account: &str) -> String {
    format!("account-proxy:{account}")
}

impl MailEngine {
    pub fn account_proxy(&self, account: &str) -> Result<AccountProxySettings> {
        self.setting(setting_key(account))?
            .map(|value| {
                serde_json::from_str::<AccountProxySettings>(&value)
                    .map_err(|_| fail("此邮箱的代理配置损坏，请重新保存连接设置"))?
                    .validated()
            })
            .unwrap_or_else(|| Ok(AccountProxySettings::default()))
    }

    pub fn set_account_proxy(&self, account: &str, settings: AccountProxySettings) -> Result<()> {
        self.account(account)?;
        let settings = settings.validated()?;
        self.set_setting(
            setting_key(account),
            serde_json::to_string(&settings).map_err(fail)?,
        )?;
        self.pool.invalidate(account);
        Ok(())
    }

    pub(crate) fn save_account_record(
        &self,
        account: Account,
        proxy: Option<AccountProxySettings>,
    ) -> Result<()> {
        if account.id.is_empty()
            || !account.address.contains('@')
            || account.imap_host.contains(['\r', '\n', '/'])
            || account.smtp_host.contains(['\r', '\n', '/'])
        {
            return Err(fail("邮箱配置不完整"));
        }
        let proxy = proxy.map(AccountProxySettings::validated).transpose()?;
        let mut db = self.connection()?;
        let tx = db.transaction().map_err(fail)?;
        tx.execute(
            "INSERT INTO accounts VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",
            params![account.id, serde_json::to_string(&account).map_err(fail)?],
        )
        .map_err(fail)?;
        if let Some(proxy) = proxy {
            tx.execute("INSERT INTO settings VALUES(?1,?2,?3) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated=excluded.updated",
                params![setting_key(&account.id), serde_json::to_string(&proxy).map_err(fail)?, now()]).map_err(fail)?;
        }
        tx.commit().map_err(fail)?;
        self.pool.invalidate(&account.id);
        Ok(())
    }

    pub(crate) fn mail_proxy(
        &self,
        account: &str,
        destination: &str,
    ) -> Result<Option<crate::proxy::Proxy>> {
        let settings = self.account_proxy(account)?;
        let route = if settings.mode == AccountProxyMode::System {
            self.proxies.resolve(destination)?
        } else {
            // Explicit policies never consult or inherit the system proxy.
            settings.explicit_route()
        };
        route
            .map(crate::proxy::Proxy::from_route)
            .transpose()
            .map(Option::flatten)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application_tests::{Events, TestPlatform};

    #[test]
    #[ignore = "explicit live proxy check; public Google endpoints, no account credentials"]
    fn live_account_proxy_google_http() {
        let port: u16 = std::env::var("LIGHTMAIL_DIAGNOSTIC_PROXY_PORT")
            .expect("explicit proxy port")
            .parse()
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let engine = MailEngine::new(directory.path().to_string_lossy().into()).unwrap();
        let platform = Arc::new(TestPlatform::default());
        *platform.route.lock().unwrap() = Some(ProxyRoute {
            kind: "http".into(),
            host: "127.0.0.1".into(),
            port: 1,
        });
        let app = crate::MailApplication::new(
            engine.clone(),
            platform.clone(),
            Arc::new(Events::default()),
        );
        let records = crate::platform::runtime().block_on(async {
            let mut records = Vec::new();
            for (mode, kind) in [(AccountProxyMode::Http, "http"), (AccountProxyMode::Socks5, "socks5")] {
                let id = format!("live-probe-{kind}");
                let account = Account { id: id.clone(), name: "Proxy diagnostic".into(), address: "probe@example.test".into(), provider: "custom".into(), imap_host: "imap.gmail.com".into(), imap_port: 993, smtp_host: "smtp.gmail.com".into(), smtp_port: 465, auth_kind: "password".into(), color: "#226451".into(), enabled: false, sent_mode: "server".into() };
                engine.save_account_record(account, Some(settings(mode, "127.0.0.1", port))).unwrap();
                let scoped = app.account_platform(&id).unwrap();
                for (endpoint, expected, post) in [
                    ("https://accounts.google.com/.well-known/openid-configuration", 200, false),
                    ("https://oauth2.googleapis.com/token", 400, true),
                    ("https://openidconnect.googleapis.com/v1/userinfo", 401, false),
                ] {
                    let started = std::time::Instant::now();
                    let mut observed_status = None;
                    let check = async {
                        let client = crate::platform::http_client(scoped.as_ref(), endpoint, 15).map_err(|_| "client setup failed")?;
                        // Explicitly invalid grant, no client id, code, token or secret.
                        let request = if post { client.post(endpoint).form(&[("grant_type", "invalid_lightmail_diagnostic")]) } else { client.get(endpoint) };
                        let response = request.send().await.map_err(|_| "HTTPS request failed")?;
                        let status = response.status().as_u16();
                        observed_status = Some(status);
                        if status != expected { return Err("unexpected endpoint status"); }
                        let body = crate::platform::limited_body(response, 64 * 1024).await.map_err(|_| "response read failed")?;
                        let value: serde_json::Value = serde_json::from_slice(&body).map_err(|_| "expected JSON response")?;
                        if expected == 200 && value["authorization_endpoint"] != "https://accounts.google.com/o/oauth2/v2/auth" { return Err("unexpected Google discovery response"); }
                        if post && value["error"] != "unsupported_grant_type" { return Err("unexpected token diagnostic response"); }
                        Ok::<_, &str>(status)
                    };
                    let result = tokio::time::timeout(std::time::Duration::from_secs(18), check).await;
                    let (status, error) = match result { Ok(Ok(status)) => (Some(status), None), Ok(Err(error)) => (observed_status, Some(error)), Err(_) => (observed_status, Some("timeout")) };
                    let row = serde_json::json!({"kind":kind,"proxy":"127.0.0.1","proxyPort":port,"endpoint":endpoint,"expectedStatus":expected,"actualStatus":status,"passed":error.is_none(),"error":error,"elapsedMs":started.elapsed().as_millis()});
                    println!("{row}"); records.push(row);
                }
                // A neighboring direct account must override even a broken system route.
                let direct = AccountProxySettings { mode: AccountProxyMode::Direct, ..Default::default() }.scoped_platform(platform.clone()).unwrap();
                assert_eq!(direct.proxy_for("imap.gmail.com".into()).unwrap().kind, "direct");
            }
            records
        });
        if let Ok(directory) = std::env::var("LIGHTMAIL_DIAGNOSTIC_REPORT_DIR") {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                std::path::Path::new(&directory).join("google-http.json"),
                serde_json::to_vec_pretty(&records).unwrap(),
            )
            .unwrap();
        }
        assert!(
            records.iter().all(|row| row["passed"] == true),
            "live Google proxy diagnostic failed"
        );
    }

    fn settings(mode: AccountProxyMode, host: &str, port: u16) -> AccountProxySettings {
        AccountProxySettings {
            mode,
            host: host.into(),
            port,
        }
    }

    #[test]
    fn proxy_settings_validate_endpoints_and_normalize_ipv6() {
        assert_eq!(
            settings(AccountProxyMode::Socks5, "[::1]", 1080)
                .validated()
                .unwrap()
                .host,
            "::1"
        );
        assert_eq!(
            settings(AccountProxyMode::Http, "  Proxy.Example.Test  ", 7897)
                .validated()
                .unwrap()
                .host,
            "proxy.example.test"
        );
        for host in [
            "",
            "http://localhost",
            "user:pass@localhost",
            "localhost:7897",
            "localhost/path",
            "host\r\n",
            "a b",
            "host\0",
        ] {
            assert!(
                settings(AccountProxyMode::Http, host, 7897)
                    .validated()
                    .is_err(),
                "{host:?}"
            );
        }
        assert!(settings(AccountProxyMode::Http, "localhost", 0)
            .validated()
            .is_err());
        assert_eq!(
            settings(AccountProxyMode::Direct, "unused", 7897)
                .validated()
                .unwrap(),
            settings(AccountProxyMode::Direct, "", 0)
        );
    }

    #[test]
    fn accounts_on_the_same_server_keep_independent_routes_and_persist_them() {
        let dir = tempfile::tempdir().unwrap();
        let engine = MailEngine::new(dir.path().to_string_lossy().into()).unwrap();
        engine.seed_demo().unwrap();
        let a = engine.accounts().unwrap()[0].clone();
        let mut b = a.clone();
        b.id = "separate-account".into();
        engine.save_account(b.clone()).unwrap();
        let platform = Arc::new(TestPlatform::default());
        *platform.route.lock().unwrap() = Some(ProxyRoute {
            kind: "http".into(),
            host: "system.invalid".into(),
            port: 1111,
        });
        let app = crate::MailApplication::new(
            engine.clone(),
            platform.clone(),
            Arc::new(Events::default()),
        );
        engine
            .set_account_proxy(&a.id, settings(AccountProxyMode::Socks5, "127.0.0.1", 2222))
            .unwrap();
        engine
            .set_account_proxy(&b.id, settings(AccountProxyMode::Direct, "", 0))
            .unwrap();
        assert_eq!(
            engine
                .mail_proxy(&a.id, &a.imap_host)
                .unwrap()
                .unwrap()
                .port,
            2222
        );
        assert!(engine.mail_proxy(&b.id, &a.imap_host).unwrap().is_none());
        assert_eq!(
            app.account_platform(&a.id)
                .unwrap()
                .proxy_for("oauth2.googleapis.com".into())
                .unwrap()
                .kind,
            "socks5"
        );
        assert_eq!(
            app.account_platform(&b.id)
                .unwrap()
                .proxy_for("oauth2.googleapis.com".into())
                .unwrap()
                .kind,
            "direct"
        );
        // Saving through the older API must preserve an existing explicit policy.
        engine.save_account(a.clone()).unwrap();
        assert_eq!(
            engine.account_proxy(&a.id).unwrap().mode,
            AccountProxyMode::Socks5
        );
        drop(app);
        drop(engine);
        let engine = MailEngine::new(dir.path().to_string_lossy().into()).unwrap();
        assert_eq!(
            engine
                .mail_proxy(&a.id, &a.smtp_host)
                .unwrap()
                .unwrap()
                .port,
            2222
        );
        assert!(engine.mail_proxy(&b.id, &b.smtp_host).unwrap().is_none());
        engine.remove_account(a.id.clone()).unwrap();
        assert!(engine.setting(setting_key(&a.id)).unwrap().is_none());
        assert_eq!(
            engine.account_proxy(&b.id).unwrap().mode,
            AccountProxyMode::Direct
        );
    }

    #[test]
    fn system_route_updates_are_resolved_at_connection_time_and_corruption_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let engine = MailEngine::new(dir.path().to_string_lossy().into()).unwrap();
        engine.seed_demo().unwrap();
        let a = engine.accounts().unwrap()[0].clone();
        let platform = Arc::new(TestPlatform::default());
        let _app = crate::MailApplication::new(
            engine.clone(),
            platform.clone(),
            Arc::new(Events::default()),
        );
        assert!(engine.mail_proxy(&a.id, &a.imap_host).unwrap().is_none());
        *platform.route.lock().unwrap() = Some(ProxyRoute {
            kind: "http".into(),
            host: "127.0.0.1".into(),
            port: 7897,
        });
        assert_eq!(
            engine
                .mail_proxy(&a.id, &a.imap_host)
                .unwrap()
                .unwrap()
                .port,
            7897
        );
        engine
            .set_setting(setting_key(&a.id), "invalid-json".into())
            .unwrap();
        assert!(engine.mail_proxy(&a.id, &a.imap_host).is_err());
    }

    #[test]
    fn account_and_proxy_save_is_atomic_and_invalid_proxy_does_not_replace_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let engine = MailEngine::new(dir.path().to_string_lossy().into()).unwrap();
        engine.seed_demo().unwrap();
        let mut account = engine.accounts().unwrap()[0].clone();
        account.enabled = false;
        let platform = Arc::new(TestPlatform::default());
        let app = crate::MailApplication::new(
            engine.clone(),
            platform.clone(),
            Arc::new(Events::default()),
        );
        let rt = crate::platform::runtime();
        rt.block_on(app.clone().save_account_with_proxy(
            account.clone(),
            String::new(),
            settings(AccountProxyMode::Http, "127.0.0.1", 7897),
        ))
        .unwrap();
        assert_eq!(engine.account_proxy(&account.id).unwrap().port, 7897);
        let name = account.name.clone();
        account.name = "must-not-save".into();
        assert!(rt
            .block_on(app.save_account_with_proxy(
                account.clone(),
                "must-not-save".into(),
                settings(AccountProxyMode::Http, "user:pass@host", 7897)
            ))
            .is_err());
        assert_eq!(engine.account(&account.id).unwrap().name, name);
        assert_eq!(engine.account_proxy(&account.id).unwrap().port, 7897);
        assert!(platform
            .read_secret(format!("account:{}", account.id))
            .unwrap()
            .is_none());
    }
}
