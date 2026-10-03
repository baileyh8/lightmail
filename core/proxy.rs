use crate::{
    models::{fail, Result},
    PlatformServices, ProxyRoute,
};
use futures_util::{stream::FuturesUnordered, StreamExt};
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proxy {
    pub kind: String,
    pub host: String,
    pub port: u16,
}
impl Proxy {
    pub(crate) fn from_route(route: ProxyRoute) -> Result<Option<Self>> {
        if route.kind == "direct" {
            return Ok(None);
        }
        if !["http", "socks5"].contains(&route.kind.as_str())
            || route.host.is_empty()
            || route.host.chars().any(char::is_whitespace)
            || route.port == 0
        {
            return Err(fail("代理配置无效"));
        }
        Ok(Some(Self {
            kind: route.kind,
            host: route.host,
            port: route.port,
        }))
    }
}

// Race address families with a short stagger. Sequential hostname dialing can
// spend seconds on an unavailable IPv6 route before attempting working IPv4.
async fn tcp_connect(host: &str, port: u16) -> std::io::Result<TcpStream> {
    let mut addresses = tokio::net::lookup_host((host, port))
        .await?
        .collect::<Vec<_>>();
    addresses.dedup();
    // Preserve the resolver's first choice but try the other family second.
    if let Some(first) = addresses.first() {
        if let Some(other) = addresses
            .iter()
            .position(|address| address.is_ipv6() != first.is_ipv6())
        {
            let address = addresses.remove(other);
            addresses.insert(1, address);
        }
    }
    race_addresses(addresses.into_iter().take(8), |address| {
        TcpStream::connect(address)
    })
    .await
}

async fn race_addresses<F, Fut>(
    addresses: impl Iterator<Item = SocketAddr>,
    connect: F,
) -> std::io::Result<TcpStream>
where
    F: Fn(SocketAddr) -> Fut + Copy,
    Fut: std::future::Future<Output = std::io::Result<TcpStream>>,
{
    let mut attempts = FuturesUnordered::new();
    for (index, address) in addresses.enumerate() {
        attempts.push(async move {
            if index > 0 {
                tokio::time::sleep(Duration::from_millis(200 * index as u64)).await;
            }
            connect(address).await
        });
    }
    let work = async {
        let mut error =
            std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "No resolved address");
        while let Some(result) = attempts.next().await {
            match result {
                Ok(stream) => {
                    stream.set_nodelay(true)?;
                    return Ok(stream);
                }
                Err(e) => error = e,
            }
        }
        Err(error)
    };
    tokio::time::timeout(Duration::from_secs(20), work)
        .await
        .map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::TimedOut, "TCP connection timed out")
        })?
}

#[derive(Default)]
pub struct Routes {
    routes: Mutex<HashMap<String, Proxy>>,
    platform: Mutex<Option<Arc<dyn PlatformServices>>>,
}
impl Routes {
    pub(crate) fn set_platform(&self, platform: Arc<dyn PlatformServices>) {
        *self.platform.lock().unwrap() = Some(platform);
    }
    pub(crate) fn resolve(&self, destination: &str) -> Result<Option<ProxyRoute>> {
        let platform = self.platform.lock().unwrap().clone();
        if let Some(platform) = platform {
            return platform.proxy_for(destination.to_owned()).map(Some);
        }
        Ok(self.get(destination).map(|proxy| ProxyRoute {
            kind: proxy.kind,
            host: proxy.host,
            port: proxy.port,
        }))
    }
    pub fn get(&self, destination: &str) -> Option<Proxy> {
        self.routes
            .lock()
            .expect("proxy routes lock")
            .get(destination)
            .cloned()
    }
    pub fn set(&self, destination: String, kind: String, host: String, port: u16) -> Result<bool> {
        if destination.is_empty() || destination.chars().any(char::is_whitespace) {
            return Err(fail("邮箱服务器地址无效"));
        }
        let next = if kind == "direct" {
            None
        } else {
            if !["socks5", "http"].contains(&kind.as_str())
                || host.is_empty()
                || host.chars().any(char::is_whitespace)
                || port == 0
            {
                return Err(fail("代理配置无效"));
            }
            Some(Proxy { kind, host, port })
        };
        let mut routes = self.routes.lock().map_err(|_| fail("无法更新代理"))?;
        if routes.get(&destination) == next.as_ref() {
            return Ok(false);
        }
        if let Some(proxy) = next {
            routes.insert(destination, proxy);
        } else {
            routes.remove(&destination);
        }
        Ok(true)
    }
}

/// Establish only the transport tunnel. Mail TLS still authenticates the original server.
pub async fn connect(host: &str, port: u16, proxy: Option<&Proxy>) -> Result<TcpStream> {
    if host.is_empty() || host.chars().any(char::is_whitespace) || port == 0 {
        return Err(fail("邮箱服务器地址无效"));
    }
    let Some(proxy) = proxy else {
        return tcp_connect(host, port)
            .await
            .map_err(|_| fail("无法连接邮件服务器，请检查网络和地址"));
    };
    let mut stream = tcp_connect(proxy.host.as_str(), proxy.port)
        .await
        .map_err(|_| fail("无法连接代理，请检查代理是否运行"))?;
    let tunnel_error = |_| fail("代理连接中断，请检查网络");
    if proxy.kind == "socks5" {
        stream.write_all(&[5, 1, 0]).await.map_err(tunnel_error)?;
        let mut reply = [0; 2];
        stream.read_exact(&mut reply).await.map_err(tunnel_error)?;
        if reply != [5, 0] {
            return Err(fail("SOCKS 代理要求认证或不支持匿名连接"));
        }
        let length = u8::try_from(host.len()).map_err(|_| fail("邮件服务器域名过长"))?;
        let mut request = vec![5, 1, 0, 3, length];
        request.extend_from_slice(host.as_bytes());
        request.extend_from_slice(&port.to_be_bytes());
        stream.write_all(&request).await.map_err(tunnel_error)?;
        let mut header = [0; 4];
        stream.read_exact(&mut header).await.map_err(tunnel_error)?;
        if header[0] != 5 || header[1] != 0 || header[2] != 0 {
            return Err(fail("SOCKS 代理拒绝邮件连接，请检查代理规则"));
        }
        let size = match header[3] {
            1 => 4,
            4 => 16,
            3 => stream.read_u8().await.map_err(tunnel_error)? as usize,
            _ => return Err(fail("SOCKS 代理响应无效")),
        };
        stream
            .read_exact(&mut vec![0; size + 2])
            .await
            .map_err(tunnel_error)?;
    } else {
        let target = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        stream
            .write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n").as_bytes())
            .await
            .map_err(tunnel_error)?;
        let mut header = Vec::new();
        // Do not buffer beyond the header: SMTP may send its greeting immediately.
        while !header.ends_with(b"\r\n\r\n") {
            if header.len() >= 16 * 1024 {
                return Err(fail("HTTP 代理响应过长"));
            }
            header.push(stream.read_u8().await.map_err(tunnel_error)?);
        }
        let line = std::str::from_utf8(&header)
            .map_err(|_| fail("HTTP 代理响应无效"))?
            .lines()
            .next()
            .unwrap_or("");
        let mut fields = line.split_whitespace();
        if !matches!(fields.next(), Some("HTTP/1.0" | "HTTP/1.1")) || fields.next() != Some("200") {
            return Err(fail("HTTP 代理拒绝邮件连接，请检查代理规则或认证"));
        }
    }
    Ok(stream)
}

/// A one-connection, loopback-only tunnel keeps lettre's TLS and STARTTLS handling intact.
/// It forwards opaque bytes and is closed on send completion, cancellation, or timeout.
pub struct SmtpTunnel {
    pub port: u16,
    task: tokio::task::JoinHandle<()>,
}
impl SmtpTunnel {
    pub async fn open(host: &str, port: u16, proxy: &Proxy) -> Result<Self> {
        let mut upstream =
            tokio::time::timeout(Duration::from_secs(20), connect(host, port, Some(proxy)))
                .await
                .map_err(|_| fail("代理连接发件服务器超时"))??;
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|_| fail("无法创建本机发件连接"))?;
        let port = listener.local_addr().map_err(fail)?.port();
        let task = tokio::spawn(async move {
            if let Ok(Ok((mut downstream, _))) =
                tokio::time::timeout(Duration::from_secs(10), listener.accept()).await
            {
                drop(listener);
                let _ = tokio::io::copy_bidirectional(&mut downstream, &mut upstream).await;
            }
        });
        Ok(Self { port, task })
    }
}
impl Drop for SmtpTunnel {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[cfg(test)]
mod diagnostics {
    #[test]
    fn stalled_address_does_not_delay_working_family() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
                .await
                .unwrap();
            let destination = listener.local_addr().unwrap();
            let stalled =
                std::net::SocketAddr::new(std::net::Ipv6Addr::LOCALHOST.into(), destination.port());
            let dropped = std::sync::atomic::AtomicBool::new(false);
            struct Cancel<'a>(&'a std::sync::atomic::AtomicBool);
            impl Drop for Cancel<'_> {
                fn drop(&mut self) {
                    self.0.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
            let connect = |address: std::net::SocketAddr| {
                let dropped = &dropped;
                async move {
                    if address.is_ipv6() {
                        let _cancel = Cancel(dropped);
                        std::future::pending::<()>().await;
                    }
                    tokio::net::TcpStream::connect(address).await
                }
            };
            let stream = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                super::race_addresses([stalled, destination].into_iter(), connect),
            )
            .await
            .expect("working IPv4 must not wait for stalled IPv6")
            .unwrap();
            assert_eq!(stream.peer_addr().unwrap(), destination);
            assert!(
                dropped.load(std::sync::atomic::Ordering::SeqCst),
                "losing connection future must be canceled"
            );
        });
    }
    #[test]
    #[ignore = "explicit, no-auth live TLS diagnostic only"]
    fn live_mail_proxy_tls() {
        use lettre::{
            transport::smtp::{
                client::{Tls, TlsParameters},
                extension::ClientId,
            },
            AsyncSmtpTransport, Tokio1Executor,
        };
        use tokio::io::AsyncReadExt;
        let port: u16 = std::env::var("LIGHTMAIL_DIAGNOSTIC_PROXY_PORT")
            .expect("explicit proxy port")
            .parse()
            .unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let records = rt.block_on(async {
            let mut records = Vec::new();
            for kind in ["http", "socks5"] {
                for (host, target_port) in [("imap.gmail.com", 993), ("smtp.gmail.com", 465), ("smtp.gmail.com", 587)] {
                    let proxy = super::Proxy { kind: kind.into(), host: "127.0.0.1".into(), port };
                    let started = std::time::Instant::now();
                    let check = async {
                        if target_port == 993 {
                            let tcp = super::connect(host, target_port, Some(&proxy)).await.map_err(|_| "proxy tunnel failed")?;
                            let mut builder = native_tls::TlsConnector::builder();
                            builder.min_protocol_version(Some(native_tls::Protocol::Tlsv12));
                            let tls = tokio_native_tls::TlsConnector::from(builder.build().map_err(|_| "TLS setup failed")?);
                            let mut stream = tls.connect(host, tcp).await.map_err(|_| "TLS validation failed")?;
                            let mut greeting = [0; 1024];
                            let read = stream.read(&mut greeting).await.map_err(|_| "server greeting failed")?;
                            if !greeting[..read].starts_with(b"* OK") { return Err("unexpected IMAP greeting"); }
                        } else {
                            let tunnel = super::SmtpTunnel::open(host, target_port, &proxy).await.map_err(|_| "SMTP proxy tunnel failed")?;
                            let tls = TlsParameters::new(host.into()).map_err(|_| "TLS setup failed")?;
                            let transport = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous("127.0.0.1")
                                .port(tunnel.port).hello_name(ClientId::Domain("lightmail-diagnostic.invalid".into()))
                                .tls(if target_port == 465 { Tls::Wrapper(tls) } else { Tls::Required(tls) })
                                .timeout(Some(std::time::Duration::from_secs(12))).build::<Tokio1Executor>();
                            // No credentials, AUTH, MAIL FROM, RCPT TO or DATA.
                            if !transport.test_connection().await.map_err(|_| "SMTP TLS or NOOP failed")? { return Err("SMTP NOOP rejected"); }
                        }
                        Ok::<_, &str>(())
                    };
                    let result = tokio::time::timeout(std::time::Duration::from_secs(20), check).await;
                    let error = match result { Ok(Ok(())) => None, Ok(Err(error)) => Some(error), Err(_) => Some("timeout") };
                    let row = serde_json::json!({"kind":kind,"proxy":"127.0.0.1","proxyPort":port,"host":host,"port":target_port,"passed":error.is_none(),"error":error,"elapsedMs":started.elapsed().as_millis()});
                    println!("{row}"); records.push(row);
                }
            }
            records
        });
        if let Ok(directory) = std::env::var("LIGHTMAIL_DIAGNOSTIC_REPORT_DIR") {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                std::path::Path::new(&directory).join("mail-tls.json"),
                serde_json::to_vec_pretty(&records).unwrap(),
            )
            .unwrap();
        }
        assert!(
            records.iter().all(|row| row["passed"] == true),
            "live mail proxy diagnostic failed"
        );
    }
}
