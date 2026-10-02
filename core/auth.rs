//! Google desktop OAuth: PKCE, loopback callback, identity check and token refresh.
//! Platform code only opens the URL and stores secrets through PlatformServices.
use crate::{models::*, platform::*};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// Private endpoint set: native callers cannot redirect OAuth credentials.
#[derive(Clone)]
pub(crate) struct Endpoints {
    pub token: String,
    pub identity: String,
}
impl Default for Endpoints {
    fn default() -> Self {
        Self {
            token: "https://oauth2.googleapis.com/token".into(),
            identity: "https://openidconnect.googleapis.com/v1/userinfo".into(),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GoogleTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<GoogleClientConfiguration>,
}

pub(crate) async fn token_request(
    platform: &dyn PlatformServices,
    fields: Vec<(&str, String)>,
    previous: &str,
    endpoints: &Endpoints,
) -> Result<GoogleTokens> {
    let endpoint = endpoints.token.as_str();
    let response = http_client(platform, endpoint, 30)?
        .post(endpoint)
        .form(&fields)
        .send()
        .await
        .map_err(|_| fail("Google 令牌连接失败"))?;
    if !response.status().is_success() {
        return Err(fail("Google 令牌获取失败，请检查 OAuth 配置或重新授权"));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&limited_body(response, 64 * 1024).await?)
            .map_err(|_| fail("Google 令牌响应无效"))?;
    let access = value["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| fail("Google 未返回访问令牌"))?;
    let refresh = value["refresh_token"].as_str().unwrap_or(previous);
    if refresh.is_empty() {
        return Err(fail("Google 未返回长期授权，请重新登录并允许离线访问"));
    }
    Ok(GoogleTokens {
        access_token: access.into(),
        refresh_token: refresh.into(),
        expires_at: now() as f64 + value["expires_in"].as_f64().unwrap_or(3600.0),
        client: None,
    })
}

#[derive(uniffi::Object)]
pub struct GoogleLogin {
    pub(crate) endpoints: Endpoints,
    platform: Arc<dyn PlatformServices>,
    pub(crate) account: Account,
    client_id: String,
    state: String,
    verifier: String,
    redirect: String,
    listener: Mutex<Option<std::net::TcpListener>>,
}
#[uniffi::export]
impl GoogleLogin {
    #[uniffi::constructor]
    pub fn new(
        account: Account,
        client_id: String,
        platform: Arc<dyn PlatformServices>,
    ) -> Result<Arc<Self>> {
        if client_id.trim().is_empty() {
            return Err(fail("请先在设置中填写 Google OAuth Desktop Client ID"));
        }
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .map_err(|_| fail("无法监听本机 Google 回调"))?;
        listener
            .set_nonblocking(true)
            .map_err(|_| fail("无法建立 Google 回调"))?;
        let redirect = format!(
            "http://127.0.0.1:{}/oauth/callback",
            listener
                .local_addr()
                .map_err(|_| fail("无法读取回调端口"))?
                .port()
        );
        // UUID v4 is backed by the operating system CSPRNG; two provide 244 random bits.
        let verifier = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        Ok(Arc::new(Self {
            endpoints: Endpoints::default(),
            platform,
            account,
            client_id,
            state: uuid::Uuid::new_v4().to_string(),
            verifier,
            redirect,
            listener: Mutex::new(Some(listener)),
        }))
    }
    pub fn authorization_url(&self) -> String {
        let mut url = url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth").unwrap();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()));
        url.query_pairs_mut().extend_pairs([
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", self.redirect.as_str()),
            ("response_type", "code"),
            ("scope", "openid email https://mail.google.com/"),
            ("access_type", "offline"),
            ("prompt", "consent select_account"),
            ("state", self.state.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("login_hint", self.account.address.as_str()),
        ]);
        url.into()
    }
    pub async fn finish(self: Arc<Self>, client_secret: String) -> Result<()> {
        let platform = self.platform.clone();
        let grant = self.exchange(client_secret).await?;
        run(async move {
            secret_write(
                platform,
                format!("account:{}", grant.account.id),
                serde_json::to_string(&grant.token).map_err(fail)?,
            )
            .await
        })
        .await
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoogleClientConfiguration {
    pub client_id: String,
    pub client_secret: String,
}

/// Verified but uncommitted authorization. Tokens never pass through the UI.
pub struct GoogleGrant {
    pub(crate) account: Account,
    pub(crate) token: GoogleTokens,
}

impl GoogleLogin {
    pub async fn exchange(self: Arc<Self>, client_secret: String) -> Result<GoogleGrant> {
        run(async move {
            let listener = self
                .listener
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| fail("此登录流程已经结束"))?;
            let listener = tokio::net::TcpListener::from_std(listener)
                .map_err(|_| fail("无法建立登录回调"))?;
            let code = tokio::time::timeout(
                Duration::from_secs(180),
                wait_callback(listener, &self.state),
            )
            .await
            .map_err(|_| fail("Google 登录等待超时，请重试"))??;
            let mut fields = vec![
                ("client_id", self.client_id.clone()),
                ("code", code),
                ("redirect_uri", self.redirect.clone()),
                ("grant_type", "authorization_code".into()),
                ("code_verifier", self.verifier.clone()),
            ];
            let configuration = GoogleClientConfiguration {
                client_id: self.client_id.clone(),
                client_secret: client_secret.clone(),
            };
            if !client_secret.is_empty() {
                fields.push(("client_secret", client_secret));
            }
            let mut token =
                token_request(self.platform.as_ref(), fields, "", &self.endpoints).await?;
            let endpoint = self.endpoints.identity.as_str();
            let response = http_client(self.platform.as_ref(), endpoint, 20)?
                .get(endpoint)
                .bearer_auth(&token.access_token)
                .send()
                .await
                .map_err(|_| fail("Google 账号验证连接失败"))?;
            if !response.status().is_success() {
                return Err(fail("无法验证 Google 授权账号"));
            }
            let identity: serde_json::Value =
                serde_json::from_slice(&limited_body(response, 64 * 1024).await?)
                    .map_err(|_| fail("Google 账号响应无效"))?;
            if !identity["email"]
                .as_str()
                .is_some_and(|s| s.eq_ignore_ascii_case(&self.account.address))
            {
                return Err(fail("Google 授权账号与填写的邮箱不一致，请选择对应账号"));
            }
            token.client = Some(configuration);
            Ok(GoogleGrant {
                account: self.account.clone(),
                token,
            })
        })
        .await
    }
}

fn callback_code(request: &str, state: &str) -> Option<Result<String>> {
    let mut pieces = request.lines().next()?.split_whitespace();
    if pieces.next()? != "GET" {
        return None;
    }
    let target = pieces.next()?;
    if !target.starts_with("/oauth/callback?") {
        return None;
    }
    let url = url::Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    if url.path() != "/oauth/callback" {
        return None;
    }
    let mut values = HashMap::new();
    for (k, v) in url.query_pairs() {
        if values.insert(k.to_string(), v.to_string()).is_some() {
            return None;
        }
    }
    if values.get("state").map(String::as_str) != Some(state) {
        return None;
    }
    Some(if values.contains_key("error") {
        Err(fail("Google 登录已取消或未获授权"))
    } else {
        values
            .remove("code")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| fail("Google 回调缺少授权码"))
    })
}
async fn wait_callback(listener: tokio::net::TcpListener, state: &str) -> Result<String> {
    loop {
        let (mut socket, _) = listener
            .accept()
            .await
            .map_err(|_| fail("登录回调已关闭"))?;
        let read = async {
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 1024];
            while bytes.len() < 8192 {
                let n = socket.read(&mut chunk).await.ok()?;
                if n == 0 {
                    return None;
                }
                bytes.extend_from_slice(&chunk[..n]);
                if bytes.windows(4).any(|s| s == b"\r\n\r\n") {
                    return callback_code(std::str::from_utf8(&bytes).ok()?, state);
                }
            }
            None
        };
        let result = tokio::time::timeout(Duration::from_secs(3), read)
            .await
            .ok()
            .flatten();
        let accepted = result.is_some();
        let text = if accepted {
            "授权回调已收到，请返回轻邮查看结果。"
        } else {
            "无效的登录回调。"
        };
        let response=format!("HTTP/1.1 {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",if accepted{"200 OK"}else{"400 Bad Request"},text.len());
        let _ = tokio::time::timeout(
            Duration::from_secs(2),
            socket.write_all(response.as_bytes()),
        )
        .await;
        if let Some(result) = result {
            return result;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_rejects_foreign_state_duplicates_and_path() {
        for url in [
            "/oauth/callback?state=wrong&code=x",
            "/oauth/callback?state=ok&state=ok&code=x",
            "/other?state=ok&code=x",
        ] {
            assert!(callback_code(&format!("GET {url} HTTP/1.1\r\n\r\n"), "ok").is_none());
        }
        assert_eq!(
            callback_code(
                "GET /oauth/callback?state=ok&code=a%2Bb HTTP/1.1\r\n\r\n",
                "ok"
            )
            .unwrap()
            .unwrap(),
            "a+b"
        );
    }
    #[test]
    fn loopback_ignores_unrelated_request_then_accepts_matching_state() {
        runtime().block_on(async {
            let listener=tokio::net::TcpListener::bind(("127.0.0.1",0)).await.unwrap();let address=listener.local_addr().unwrap();
            let browser=tokio::spawn(async move {
                for state in ["foreign","expected"] {
                    let mut socket=tokio::net::TcpStream::connect(address).await.unwrap();
                    socket.write_all(format!("GET /oauth/callback?state={state}&code=synthetic HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes()).await.unwrap();
                    let mut response=String::new();socket.read_to_string(&mut response).await.unwrap();
                    assert!(response.contains(if state=="foreign"{"400 Bad Request"}else{"200 OK"}));
                }
            });
            let code=tokio::time::timeout(Duration::from_secs(3),wait_callback(listener,"expected")).await.unwrap().unwrap();
            assert_eq!(code,"synthetic");browser.await.unwrap();
        });
    }
}
