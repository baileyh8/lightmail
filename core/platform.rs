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

#[derive(Default)]
pub(crate) struct OfflinePlatform(std::sync::Mutex<std::collections::HashMap<String, String>>);
impl PlatformServices for OfflinePlatform {
    fn read_secret(&self, key: String) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(&key).cloned())
    }
    fn write_secret(&self, key: String, value: String) -> Result<()> {
        self.0.lock().unwrap().insert(key, value);
        Ok(())
    }
    fn remove_secret(&self, key: String) -> Result<()> {
        self.0.lock().unwrap().remove(&key);
        Ok(())
    }
    fn proxy_for(&self, _: String) -> Result<ProxyRoute> {
        Err(fail("示例模式禁止网络请求"))
    }
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

/// Blocking, size-limited GET for optional resources a native reader shows only
/// on request, such as remote images. It uses the platform proxy route and the
/// same same-origin redirect policy as every other core HTTP request. Returns the
/// body and its Content-Type. Rust-only; call it from a plain thread, never from
/// an async runtime.
pub fn fetch_resource(
    platform: Arc<dyn PlatformServices>,
    url: String,
    limit: usize,
) -> Result<(Vec<u8>, String)> {
    let parsed = url::Url::parse(&url).map_err(|_| fail("资源地址无效"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(fail("只加载网页资源"));
    }
    runtime().block_on(async move {
        let response = http_client(platform.as_ref(), &url, 15)?
            .get(parsed)
            .send()
            .await
            .map_err(|_| fail("资源下载失败"))?;
        if !response.status().is_success() {
            return Err(fail("资源下载失败"));
        }
        if response
            .content_length()
            .is_some_and(|size| size > limit as u64)
        {
            return Err(fail("资源超过处理上限"));
        }
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        Ok((limited_body(response, limit).await?, mime))
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    struct Direct;
    impl PlatformServices for Direct {
        fn read_secret(&self, _key: String) -> Result<Option<String>> {
            Ok(None)
        }
        fn write_secret(&self, _key: String, _value: String) -> Result<()> {
            Ok(())
        }
        fn remove_secret(&self, _key: String) -> Result<()> {
            Ok(())
        }
        fn proxy_for(&self, _host: String) -> Result<ProxyRoute> {
            Ok(ProxyRoute {
                kind: "direct".into(),
                host: String::new(),
                port: 0,
            })
        }
    }

    // Serves each queued body once, with a Content-Length header.
    fn serve(bodies: Vec<Vec<u8>>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for body in bodies {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = [0; 2048];
                let _ = socket.read(&mut request);
                let _ = write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(&body);
            }
        });
        format!("http://{address}/pixel.png")
    }

    #[test]
    fn resources_are_limited_and_only_fetched_over_the_web() {
        let platform: Arc<dyn PlatformServices> = Arc::new(Direct);
        let url = serve(vec![b"small".to_vec(), vec![7; 4096]]);
        let (bytes, mime) = fetch_resource(platform.clone(), url.clone(), 1024).unwrap();
        assert_eq!(
            (bytes.as_slice(), mime.as_str()),
            (&b"small"[..], "image/png")
        );
        assert!(fetch_resource(platform.clone(), url, 1024).is_err());
        for url in [
            "file:///C:/private.png",
            "data:image/png;base64,AAAA",
            "cid:part",
        ] {
            assert!(fetch_resource(platform.clone(), url.into(), 1024).is_err());
        }
    }
}
