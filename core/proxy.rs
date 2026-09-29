use crate::models::{fail, Result};
use std::{collections::HashMap, sync::Mutex, time::Duration};
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

#[derive(Default)]
pub struct Routes(Mutex<HashMap<String, Proxy>>);
impl Routes {
    pub fn get(&self, destination: &str) -> Option<Proxy> {
        self.0
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
                return Err(fail("系统代理配置无效"));
            }
            Some(Proxy { kind, host, port })
        };
        let mut routes = self.0.lock().map_err(|_| fail("无法更新系统代理"))?;
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
        return TcpStream::connect((host, port))
            .await
            .map_err(|_| fail("无法连接邮件服务器，请检查网络和地址"));
    };
    let mut stream = TcpStream::connect((proxy.host.as_str(), proxy.port))
        .await
        .map_err(|_| fail("无法连接系统代理，请检查代理是否运行"))?;
    let tunnel_error = |_| fail("系统代理连接中断，请检查网络");
    if proxy.kind == "socks5" {
        stream.write_all(&[5, 1, 0]).await.map_err(tunnel_error)?;
        let mut reply = [0; 2];
        stream.read_exact(&mut reply).await.map_err(tunnel_error)?;
        if reply != [5, 0] {
            return Err(fail("系统 SOCKS 代理要求认证或不支持匿名连接"));
        }
        let length = u8::try_from(host.len()).map_err(|_| fail("邮件服务器域名过长"))?;
        let mut request = vec![5, 1, 0, 3, length];
        request.extend_from_slice(host.as_bytes());
        request.extend_from_slice(&port.to_be_bytes());
        stream.write_all(&request).await.map_err(tunnel_error)?;
        let mut header = [0; 4];
        stream.read_exact(&mut header).await.map_err(tunnel_error)?;
        if header[0] != 5 || header[1] != 0 || header[2] != 0 {
            return Err(fail("系统 SOCKS 代理拒绝邮件连接，请检查代理规则"));
        }
        let size = match header[3] {
            1 => 4,
            4 => 16,
            3 => stream.read_u8().await.map_err(tunnel_error)? as usize,
            _ => return Err(fail("系统 SOCKS 代理响应无效")),
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
                return Err(fail("系统 HTTP 代理响应过长"));
            }
            header.push(stream.read_u8().await.map_err(tunnel_error)?);
        }
        let line = std::str::from_utf8(&header)
            .map_err(|_| fail("系统 HTTP 代理响应无效"))?
            .lines()
            .next()
            .unwrap_or("");
        let mut fields = line.split_whitespace();
        if !matches!(fields.next(), Some("HTTP/1.0" | "HTTP/1.1")) || fields.next() != Some("200") {
            return Err(fail("系统 HTTP 代理拒绝邮件连接，请检查代理规则或认证"));
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
                .map_err(|_| fail("系统代理连接发件服务器超时"))??;
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
    #[ignore = "explicit, no-auth live TLS diagnostic only"]
    fn live_mail_proxy_tls() {
        let port: u16 = std::env::var("LIGHTMAIL_DIAGNOSTIC_PROXY_PORT")
            .expect("explicit proxy port")
            .parse()
            .unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            for kind in ["http", "socks5"] {
                for (host, target_port) in [("imap.gmail.com", 993), ("smtp.gmail.com", 465)] {
                    let proxy = super::Proxy {
                        kind: kind.into(),
                        host: "127.0.0.1".into(),
                        port,
                    };
                    let check = async {
                        let tcp = super::connect(host, target_port, Some(&proxy))
                            .await
                            .unwrap();
                        let mut builder = native_tls::TlsConnector::builder();
                        builder.min_protocol_version(Some(native_tls::Protocol::Tlsv12));
                        let tls = tokio_native_tls::TlsConnector::from(builder.build().unwrap());
                        tls.connect(host, tcp)
                            .await
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    };
                    println!(
                        "{kind} {host}:{target_port} {:?}",
                        tokio::time::timeout(std::time::Duration::from_secs(15), check).await
                    );
                }
            }
        });
    }
}
