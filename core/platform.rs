//! Platform capabilities only; policy and task ownership live in Application.
use crate::models::*;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

pub(crate) fn runtime() -> Arc<tokio::runtime::Runtime> {
    static RUNTIME: OnceLock<Arc<tokio::runtime::Runtime>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            Arc::new(
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .max_blocking_threads(4)
                    .enable_all()
                    .build()
                    .expect("mail runtime"),
            )
        })
        .clone()
}

// Dropping a Swift/FFI future must cancel the actual worker, not detach it.
pub(crate) async fn run<T: Send + 'static>(
    future: impl std::future::Future<Output = Result<T>> + Send + 'static,
) -> Result<T> {
    struct Abort(tokio::task::AbortHandle);
    impl Drop for Abort {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let job = runtime().spawn(future);
    let _abort = Abort(job.abort_handle());
    job.await.map_err(|_| fail("操作已取消"))?
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct ProxyRoute {
    pub kind: String,
    pub host: String,
    pub port: u16,
}

#[uniffi::export(with_foreign)]
pub trait PlatformServices: Send + Sync {
    /// Must not display an interactive authentication prompt.
    fn read_secret(&self, key: String) -> Result<Option<String>>;
    fn write_secret(&self, key: String, value: String) -> Result<()>;
    fn remove_secret(&self, key: String) -> Result<()>;
    fn proxy_for(&self, host: String) -> Result<ProxyRoute>;
}

// OS vault calls can wait behind explicit authorization in another thread. Never
// block the network executor while the user is interacting with a platform dialog.
pub(crate) async fn secret_read(
    platform: Arc<dyn PlatformServices>,
    key: String,
) -> Result<Option<String>> {
    tokio::task::spawn_blocking(move || platform.read_secret(key))
        .await
        .map_err(|_| fail("凭证读取已中断"))?
}
pub(crate) async fn secret_write(
    platform: Arc<dyn PlatformServices>,
    key: String,
    value: String,
) -> Result<()> {
    tokio::task::spawn_blocking(move || platform.write_secret(key, value))
        .await
        .map_err(|_| fail("凭证保存已中断"))?
}

pub(crate) fn http_client(
    platform: &dyn PlatformServices,
    endpoint: &str,
    seconds: u64,
) -> Result<reqwest::Client> {
    let url = url::Url::parse(endpoint).map_err(|_| fail("服务地址无效"))?;
    let route = platform.proxy_for(url.host_str().unwrap_or_default().into())?;
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(seconds))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.stop();
            }
            if attempt
                .previous()
                .first()
                .is_some_and(|old| old.origin() == attempt.url().origin())
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }));
    if route.kind != "direct" {
        if !["http", "socks5"].contains(&route.kind.as_str())
            || route.host.is_empty()
            || route.port == 0
        {
            return Err(fail("代理配置无效"));
        }
        let scheme = if route.kind == "socks5" {
            "socks5h"
        } else {
            "http"
        };
        let host = if route.host.contains(':') {
            format!("[{}]", route.host.trim_matches(['[', ']']))
        } else {
            route.host
        };
        let proxy = format!("{scheme}://{host}:{}", route.port);
        builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(|_| fail("代理地址无效"))?);
    }
    builder.build().map_err(|_| fail("无法建立网络会话"))
}

pub(crate) async fn limited_body(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| fail("服务连接提前中断"))?
    {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(fail("服务响应超过处理上限"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
