use crate::{mime, models::*, store::MailEngine};
use async_imap::{types::Flag, Session};
use futures_util::TryStreamExt;
use imap_proto::types::{BodyContentCommon, BodyContentSinglePart, BodyStructure, SectionPath};
use rusqlite::params;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Mutex as AsyncMutex;

type MailSession = Session<tokio_native_tls::TlsStream<tokio::net::TcpStream>>;

// The IMAP library may include whole responses (including headers) in errors.
// Expose only a fixed category so diagnostics never disclose message contents.
fn imap_failure(context: &str, error: async_imap::error::Error) -> MailError {
    use async_imap::error::Error;
    let reason = match error {
        Error::Io(ref e) if e.to_string().contains("during parsing") => "服务器响应格式不兼容",
        Error::Parse(_) => "服务器响应格式不兼容",
        Error::Bad(_) => "服务器不支持此请求",
        Error::No(_) => "服务器拒绝此请求",
        Error::ConnectionLost => "服务器关闭了连接",
        Error::Io(_) => "网络连接中断",
        _ => "邮件协议处理失败",
    };
    fail(format!("{context}：{reason}"))
}

fn next_uids(descending: &[u32], maximum: u32, initial: bool) -> Vec<u32> {
    if initial {
        return descending.iter().copied().take(50).collect();
    }
    // Advance from the oldest unseen UID so a burst over one batch cannot leave a permanent gap.
    descending
        .iter()
        .rev()
        .copied()
        .filter(|uid| *uid > maximum)
        .take(500)
        .collect()
}

#[cfg(test)]
mod batch_tests {
    #[test]
    fn protocol_errors_do_not_expose_server_responses() {
        use async_imap::error::Error;
        let cases = [
            (
                Error::No("private fixture response".into()),
                "服务器拒绝此请求",
            ),
            (
                Error::Bad("private fixture response".into()),
                "服务器不支持此请求",
            ),
            (
                Error::Io(std::io::Error::other(
                    "private fixture during parsing of private headers",
                )),
                "服务器响应格式不兼容",
            ),
            (Error::ConnectionLost, "服务器关闭了连接"),
        ];
        for (error, reason) in cases {
            let message = super::imap_failure("读取失败", error).to_string();
            assert!(message.contains(reason));
            assert!(!message.contains("private"));
        }
    }

    #[test]
    fn large_arrival_burst_has_no_uid_gap() {
        let all = (1..=1200).rev().collect::<Vec<_>>();
        let first = super::next_uids(&all, 100, false);
        let second = super::next_uids(&all, *first.last().unwrap(), false);
        let third = super::next_uids(&all, *second.last().unwrap(), false);
        assert_eq!(
            [first, second, third].concat(),
            (101..=1200).collect::<Vec<_>>()
        );
    }
}
pub(crate) struct Pooled {
    session: MailSession,
    key: String,
}
#[derive(Default)]
pub struct Pool {
    slots: Mutex<HashMap<String, Arc<AsyncMutex<Option<Pooled>>>>>,
}
impl Pool {
    pub fn clear(&self) {
        self.slots.lock().unwrap().clear();
    }
    pub(crate) fn slot(&self, id: &str) -> Arc<AsyncMutex<Option<Pooled>>> {
        self.slots
            .lock()
            .unwrap()
            .entry(id.into())
            .or_insert_with(|| Arc::new(AsyncMutex::new(None)))
            .clone()
    }
    pub fn invalidate(&self, id: &str) {
        self.slots.lock().unwrap().retain(|k, _| {
            k != id
                && !["idle", "reader", "prefetch"]
                    .iter()
                    .any(|role| k == &format!("{role}:{id}"))
        });
    }
}
struct OAuth {
    user: String,
    token: String,
    sent: bool,
}
impl async_imap::Authenticator for OAuth {
    type Response = String;
    fn process(&mut self, _: &[u8]) -> String {
        if self.sent {
            return String::new();
        }
        self.sent = true;
        format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token)
    }
}
async fn connect(
    a: &Account,
    credential: &str,
    route: Option<crate::proxy::Proxy>,
) -> Result<MailSession> {
    if a.provider == "demo" {
        return Err(fail("示例账号不会连接真实邮箱"));
    }
    if credential.is_empty() {
        return Err(fail("请先为此邮箱配置授权信息"));
    }
    let fut = async {
        let tcp = crate::proxy::connect(&a.imap_host, a.imap_port, route.as_ref()).await?;
        let mut connector_builder = native_tls::TlsConnector::builder();
        connector_builder.min_protocol_version(Some(native_tls::Protocol::Tlsv12));
        #[cfg(test)]
        if a.imap_host == "localhost" {
            if let Ok(path) = std::env::var("LIGHTMAIL_TEST_CA") {
                connector_builder.add_root_certificate(
                    native_tls::Certificate::from_pem(&std::fs::read(path).map_err(fail)?)
                        .map_err(fail)?,
                );
            }
        }
        let connector = connector_builder.build().map_err(fail)?;
        let tls = tokio_native_tls::TlsConnector::from(connector)
            .connect(&a.imap_host, tcp)
            .await
            .map_err(|_| fail("收件服务器 TLS 连接失败，请检查网络、代理与证书"))?;
        let mut client = async_imap::Client::new(tls);
        client
            .read_response()
            .await
            .map_err(|_| fail("收件服务器未正确响应"))?
            .ok_or_else(|| fail("收件服务器关闭了连接"))?;
        let mut session = if a.auth_kind == "oauth" {
            client
                .authenticate(
                    "XOAUTH2",
                    OAuth {
                        user: a.address.clone(),
                        token: credential.into(),
                        sent: false,
                    },
                )
                .await
                .map_err(|_| fail("Google 授权失效，请重新登录"))?
        } else {
            client
                .login(&a.address, credential)
                .await
                .map_err(|_| fail("邮箱认证失败，请检查客户端授权码和 IMAP 开关"))?
        };
        let caps = session
            .capabilities()
            .await
            .map_err(|_| fail("无法读取邮箱能力"))?;
        if caps.has_str("ID") {
            let _ = session
                .id([
                    ("name", Some("Lightmail")),
                    ("version", Some(env!("CARGO_PKG_VERSION"))),
                    ("vendor", Some("Lightmail")),
                ])
                .await;
        }
        Ok(session)
    };
    tokio::time::timeout(Duration::from_secs(20), fut)
        .await
        .map_err(|_| fail("连接超时，请检查网络"))?
}
fn connection_key(a: &Account, c: &str, route: &Option<crate::proxy::Proxy>) -> String {
    mime::hash(&format!(
        "{}:{}:{}:{}:{}:{route:?}",
        a.imap_host, a.imap_port, a.address, a.auth_kind, c
    ))
}
async fn take_session(
    e: &MailEngine,
    slot: &mut Option<Pooled>,
    a: &Account,
    c: &str,
) -> Result<(MailSession, String)> {
    let route = e.mail_proxy(&a.id, &a.imap_host)?;
    let key = connection_key(a, c, &route);
    if let Some(p) = slot.take() {
        if p.key == key {
            return Ok((p.session, key));
        }
    }
    connect(a, c, route).await.map(|session| (session, key))
}
fn return_session(slot: &mut Option<Pooled>, s: MailSession, key: String) {
    *slot = Some(Pooled { session: s, key });
}
fn role_for(path: &str, attrs: &str) -> String {
    let p = mime::decode_folder(path).to_lowercase();
    let attrs = attrs.to_lowercase();
    if p == "inbox" {
        "inbox"
    } else if attrs.contains("sent")
        || ["sent", "sent messages", "sent items", "已发送"].contains(&p.as_str())
    {
        "sent"
    } else if attrs.contains("draft") || p.contains("草稿") || p == "drafts" {
        "drafts"
    } else if attrs.contains("trash")
        || p.contains("已删除")
        || p == "trash"
        || p == "deleted messages"
    {
        "trash"
    } else if attrs.contains("junk") || p.contains("垃圾") || p == "spam" || p == "junk" {
        "junk"
    } else if attrs.contains("archive") || p == "archive" || p == "归档" {
        "archive"
    } else if attrs.contains("all") || p == "[gmail]/all mail" {
        "allmail"
    } else {
        "custom"
    }
    .into()
}
async fn discover(s: &mut MailSession, a: &Account) -> Result<Vec<Folder>> {
    let names = s
        .list(Some(""), Some("*"))
        .await
        .map_err(|_| fail("无法读取邮箱文件夹"))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|_| fail("文件夹列表中断"))?;
    let mut folders = Vec::new();
    for n in names {
        if n.attributes()
            .iter()
            .any(|v| format!("{v:?}").contains("NoSelect"))
        {
            continue;
        }
        let path = n.name().to_string();
        let role = role_for(&path, &format!("{:?}", n.attributes()));
        folders.push(Folder {
            id: folder_id(&a.id, &path),
            account_id: a.id.clone(),
            name: mime::decode_folder(&path),
            path,
            role,
            unread_count: 0,
            total_count: 0,
        });
    }
    Ok(folders)
}
pub async fn sync(
    e: &MailEngine,
    id: &str,
    credential: &str,
    path: Option<String>,
    older: bool,
) -> Result<SyncResult> {
    let a = e.account(id)?;
    if a.provider == "demo" {
        return Ok(SyncResult {
            added: 0,
            folders: 0,
            message: "示例数据 · 不连接邮箱".into(),
        });
    }
    let slot = e.pool.slot(id);
    let mut guard = slot.lock().await;
    let (mut session, connection_key) = take_session(e, &mut guard, &a, credential).await?;
    let work = async {
        let folders = discover(&mut session, &a).await?;
        {
            let db = e.connection()?;
            MailEngine::require_account(&db, id)?;
            for f in &folders {
                db.execute("INSERT INTO folders(id,account_id,path,role,data) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET role=excluded.role,data=excluded.data",params![f.id,f.account_id,f.path,f.role,serde_json::to_string(f).map_err(fail)?]).map_err(fail)?;
            }
            let old: Vec<String> = {
                let mut statement = db
                    .prepare("SELECT id FROM folders WHERE account_id=?1 AND path!='local'")
                    .map_err(fail)?;
                let result = statement
                    .query_map([id], |r| r.get(0))
                    .map_err(fail)?
                    .collect::<std::result::Result<_, _>>()
                    .map_err(fail)?;
                result
            };
            for old_id in old
                .iter()
                .filter(|old| !folders.iter().any(|f| &f.id == *old))
            {
                db.execute("DELETE FROM messages WHERE folder_id=?1", [old_id])
                    .map_err(fail)?;
                db.execute("DELETE FROM folders WHERE id=?1", [old_id])
                    .map_err(fail)?;
            }
        }
        let mut targets: Vec<_> = folders
            .iter()
            .filter(|f| {
                if let Some(ref p) = path {
                    f.path == *p
                } else {
                    f.role == "inbox" || f.role == "sent" || f.role == "custom"
                }
            })
            .cloned()
            .collect();
        targets.sort_by_key(|f| match f.role.as_str() {
            "inbox" => 0,
            "sent" => 1,
            _ => 2,
        });
        targets.truncate(if path.is_some() { 1 } else { 12 });
        let caps = session
            .capabilities()
            .await
            .map_err(|_| fail("读取邮箱能力失败"))?;
        let gmail = caps.has_str("X-GM-EXT-1");
        let mut added = 0;
        for mut f in targets {
            let mailbox = session
                .select(&f.path)
                .await
                .map_err(|_| fail(format!("无法打开文件夹：{}", f.name)))?;
            let validity = mailbox.uid_validity.unwrap_or(0);
            let all = session
                .uid_search("ALL")
                .await
                .map_err(|_| fail("无法获取邮件目录"))?;
            let cached: Vec<MessageSummary> = {
                let db = e.connection()?;
                let mut stmt = db
                    .prepare("SELECT data FROM messages WHERE folder_id=?1")
                    .map_err(fail)?;
                let rows = stmt
                    .query_map([&f.id], |r| r.get::<_, String>(0))
                    .map_err(fail)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(fail)?;
                rows.into_iter()
                    .map(|s| serde_json::from_str(&s).map_err(fail))
                    .collect::<Result<_>>()?
            };
            let valid_cached: HashSet<u32> = cached
                .iter()
                .filter(|m| m.uid_validity == validity)
                .map(|m| m.uid)
                .collect();
            let mut sorted: Vec<u32> = all.iter().copied().collect();
            sorted.sort_unstable_by(|a, b| b.cmp(a));
            let min = valid_cached.iter().min().copied().unwrap_or(u32::MAX);
            let max = valid_cached.iter().max().copied().unwrap_or(0);
            let fetch_ids: Vec<u32> = if older {
                sorted
                    .iter()
                    .copied()
                    .filter(|u| *u < min)
                    .take(200)
                    .collect()
            } else {
                next_uids(&sorted, max, valid_cached.is_empty())
            };
            let mut fetched = Vec::new();
            for chunk in fetch_ids.chunks(50) {
                let set = chunk
                    .iter()
                    .map(|u| u.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                let fields = if gmail {
                    "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER] BODYSTRUCTURE X-GM-MSGID)"
                } else {
                    "(UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER] BODYSTRUCTURE)"
                };
                let items = session
                    .uid_fetch(set, fields)
                    .await
                    .map_err(|e| imap_failure("获取邮件摘要失败", e))?
                    .try_collect::<Vec<_>>()
                    .await
                    .map_err(|e| imap_failure("邮件摘要读取中断", e))?;
                for item in items {
                    let Some(uid) = item.uid else {
                        continue;
                    };
                    let mut m = mime::summary(item.header().unwrap_or(&[]), &a, &f, uid, validity);
                    m.unread = !item.flags().any(|f| matches!(f, Flag::Seen));
                    m.starred = item.flags().any(|f| matches!(f, Flag::Flagged));
                    m.size = item.size.unwrap_or(0) as u64;
                    if let Some(ts) = item.internal_date() {
                        m.timestamp = ts.timestamp();
                    }
                    if let Some(gid) = item.gmail_msg_id() {
                        m.canonical_id = format!("{}:gmail:{}", id, gid);
                    }
                    if let Some(bs) = item.bodystructure() {
                        let mut parts = Vec::new();
                        let _ = flatten_parts(bs, vec![], &mut parts, 0);
                        m.has_attachments = parts.iter().any(|p| p.attachment);
                    }
                    fetched.push(m);
                }
            }
            let flags_ids: Vec<_> = valid_cached.intersection(&all).copied().collect();
            let mut updates = HashMap::new();
            for chunk in flags_ids.chunks(200) {
                let set = chunk
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                let items = session
                    .uid_fetch(set, "(UID FLAGS)")
                    .await
                    .map_err(|_| fail("同步已读状态失败"))?
                    .try_collect::<Vec<_>>()
                    .await
                    .map_err(|_| fail("读取邮件状态中断"))?;
                for item in items {
                    if let Some(uid) = item.uid {
                        updates.insert(
                            uid,
                            (
                                !item.flags().any(|f| matches!(f, Flag::Seen)),
                                item.flags().any(|f| matches!(f, Flag::Flagged)),
                            ),
                        );
                    }
                }
            }
            let mut db = e.connection()?;
            let tx = db.transaction().map_err(fail)?;
            MailEngine::require_account(&tx, id)?;
            for mut m in cached {
                if m.uid_validity != validity || !all.contains(&m.uid) {
                    tx.execute("DELETE FROM messages WHERE id=?1", [m.id])
                        .map_err(fail)?;
                } else if let Some((unread, starred)) = updates.get(&m.uid) {
                    m.unread = *unread;
                    m.starred = *starred;
                    MailEngine::put_message(&tx, &m)?;
                }
            }
            added += fetched.len() as u32;
            for m in fetched {
                MailEngine::put_message(&tx, &m)?;
            }
            f.total_count = mailbox.exists;
            tx.execute(
                "UPDATE folders SET validity=?1,data=?2 WHERE id=?3",
                params![validity, serde_json::to_string(&f).map_err(fail)?, f.id],
            )
            .map_err(fail)?;
            tx.commit().map_err(fail)?;
            drop(db);
            e.prune_recent_bodies(id)?;
        }
        Ok(SyncResult {
            added,
            folders: folders.len() as u32,
            message: "刚刚同步".into(),
        })
    };
    let result = tokio::time::timeout(Duration::from_secs(90), work)
        .await
        .map_err(|_| fail("同步超时，已保存完成的文件夹，可稍后刷新"))?;
    if result.is_ok() {
        return_session(&mut guard, session, connection_key);
    }
    result
}

#[derive(Clone)]
struct Part {
    path: Vec<u32>,
    mime: String,
    encoding: String,
    charset: String,
    filename: String,
    size: u64,
    attachment: bool,
    content_id: String,
}
fn flatten_parts(
    bs: &BodyStructure<'_>,
    path: Vec<u32>,
    out: &mut Vec<Part>,
    depth: usize,
) -> Result<()> {
    if depth > 20 || out.len() >= 200 {
        return Err(fail("邮件结构超过读取上限，尚未加载全文"));
    }
    match bs {
        BodyStructure::Multipart { bodies, .. } => {
            for (i, b) in bodies.iter().enumerate() {
                let mut next = path.clone();
                next.push(i as u32 + 1);
                flatten_parts(b, next, out, depth + 1)?;
            }
        }
        BodyStructure::Basic { common, other, .. }
        | BodyStructure::Text { common, other, .. }
        | BodyStructure::Message { common, other, .. } => {
            out.push(part_info(
                common,
                other,
                if path.is_empty() { vec![1] } else { path },
            ));
        }
    }
    Ok(())
}
fn part_info(c: &BodyContentCommon<'_>, o: &BodyContentSinglePart<'_>, path: Vec<u32>) -> Part {
    let param = |key: &str| {
        c.ty.params
            .as_ref()
            .and_then(|ps| ps.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)))
            .map(|(_, v)| v.to_string())
            .unwrap_or_default()
    };
    let filename = c
        .disposition
        .as_ref()
        .and_then(|d| d.params.as_ref())
        .and_then(|ps| ps.iter().find(|(k, _)| k.eq_ignore_ascii_case("filename")))
        .map(|(_, v)| v.to_string())
        .unwrap_or_else(|| param("name"));
    let mime = format!("{}/{}", c.ty.ty, c.ty.subtype).to_lowercase();
    let attachment = !filename.is_empty()
        || c.disposition
            .as_ref()
            .is_some_and(|d| d.ty.eq_ignore_ascii_case("attachment"))
        || !mime.starts_with("text/");
    let encoding = match &o.transfer_encoding {
        imap_proto::types::ContentEncoding::Base64 => "base64",
        imap_proto::types::ContentEncoding::QuotedPrintable => "quoted-printable",
        _ => "8bit",
    }
    .into();
    Part {
        content_id: o
            .id
            .as_deref()
            .unwrap_or_default()
            .trim_matches(['<', '>'])
            .to_owned(),
        path,
        mime,
        encoding,
        charset: param("charset"),
        filename,
        size: o.octets as u64,
        attachment,
    }
}
fn rendered_parts(bs: &BodyStructure<'_>, path: Vec<u32>, depth: usize) -> Result<Vec<Part>> {
    if depth > 20 {
        return Err(fail("邮件结构过深，尚未加载全文"));
    }
    match bs {
        BodyStructure::Multipart { common, bodies, .. } => {
            let mut groups = Vec::new();
            for (i, b) in bodies.iter().enumerate() {
                let mut next = path.clone();
                next.push(i as u32 + 1);
                groups.push(rendered_parts(b, next, depth + 1)?);
            }
            if common.ty.subtype.eq_ignore_ascii_case("alternative") {
                let best = groups
                    .iter()
                    .rposition(|g| g.iter().any(|p| p.mime == "text/html"))
                    .or_else(|| groups.iter().position(|g| !g.is_empty()));
                Ok(best.map(|i| groups.remove(i)).unwrap_or_default())
            } else {
                Ok(groups.into_iter().flatten().collect())
            }
        }
        BodyStructure::Basic { common, other, .. }
        | BodyStructure::Text { common, other, .. }
        | BodyStructure::Message { common, other, .. } => {
            let part = part_info(common, other, if path.is_empty() { vec![1] } else { path });
            if !part.attachment && ["text/plain", "text/html"].contains(&part.mime.as_str()) {
                Ok(vec![part])
            } else {
                Ok(vec![])
            }
        }
    }
}
async fn parts(s: &mut MailSession, m: &MessageSummary) -> Result<(Vec<Part>, Vec<Part>)> {
    let selected = s
        .select(folder_path(m))
        .await
        .map_err(|_| fail("邮件文件夹不可用"))?;
    if selected.uid_validity.unwrap_or(0) != m.uid_validity {
        return Err(fail("邮箱目录已更新，请先刷新后再打开"));
    }
    let items = s
        .uid_fetch(m.uid.to_string(), "(UID BODYSTRUCTURE)")
        .await
        .map_err(|e| imap_failure("无法获取邮件结构", e))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|e| imap_failure("邮件结构读取失败", e))?;
    let bs = items
        .first()
        .and_then(|i| i.bodystructure())
        .ok_or_else(|| fail("邮件已移动或缺少结构信息"))?;
    let mut out = Vec::new();
    flatten_parts(bs, vec![], &mut out, 0)?;
    let rendered = rendered_parts(bs, vec![], 0)?;
    Ok((out, rendered))
}
fn folder_path(m: &MessageSummary) -> &str {
    m.folder_id
        .strip_prefix(&format!("{}:", m.account_id))
        .unwrap_or("INBOX")
}
async fn fetch_part(s: &mut MailSession, uid: u32, p: &Part, max: u64) -> Result<Vec<u8>> {
    if p.size > max {
        return Err(fail("邮件部分超过读取上限，请使用邮箱网页处理此超大内容"));
    }
    let section = p
        .path
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(".");
    let items = s
        .uid_fetch(uid.to_string(), format!("(UID BODY.PEEK[{section}])"))
        .await
        .map_err(|_| fail("无法下载邮件内容"))?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|_| fail("邮件内容下载中断"))?;
    let data = items
        .first()
        .and_then(|i| i.section(&SectionPath::Part(p.path.clone(), None)))
        .ok_or_else(|| fail("服务器未返回邮件内容"))?;
    if data.len() as u64 > max {
        return Err(fail("邮件内容超过上限"));
    }
    let charset = if p.charset.is_empty() {
        "utf-8"
    } else {
        &p.charset
    };
    let mut raw = format!(
        "Content-Type: {}; charset=\"{}\"\r\nContent-Transfer-Encoding: {}\r\n\r\n",
        p.mime,
        charset.replace(['\r', '\n', '"'], ""),
        p.encoding
    )
    .into_bytes();
    raw.extend_from_slice(data);
    Ok(raw)
}
pub async fn body(e: &MailEngine, id: &str, c: &str) -> Result<MailBody> {
    tokio::time::timeout(Duration::from_secs(30), read_body(e, id, c, "reader"))
        .await
        .map_err(|_| fail("正文读取超时，请重试；后台同步不会阻塞阅读"))?
}
pub async fn prefetch_body(e: &MailEngine, id: &str, c: &str) -> Result<MailBody> {
    tokio::time::timeout(Duration::from_secs(30), read_body(e, id, c, "prefetch"))
        .await
        .map_err(|_| fail("后台正文预加载超时"))?
}
async fn read_body(e: &MailEngine, id: &str, c: &str, role: &str) -> Result<MailBody> {
    let m = e.message(id)?;
    let a = e.account(&m.account_id)?;
    // Foreground reads must never queue behind whole-account sync or prefetch.
    let slot = e.pool.slot(&format!("{role}:{}", a.id));
    let mut guard = slot.lock().await;
    let (mut s, connection_key) = take_session(e, &mut guard, &a, c).await?;
    let work = async {
        let (ps, rendered) = parts(&mut s, &m).await?;
        let mut html = Vec::new();
        let mut md = Vec::new();
        let mut total = 0u64;
        let mut has_html = false;
        for p in &rendered {
            total += p.size;
            if total > 4 * 1024 * 1024 {
                return Err(fail("正文超过 4 MiB，尚未加载全文，请在邮箱网页查看"));
            }
            let raw = fetch_part(&mut s, m.uid, p, 4 * 1024 * 1024).await?;
            let b = mime::parse_body(&raw, id)?;
            md.push(b.markdown);
            if p.mime == "text/html" {
                has_html = true;
                html.push(b.html);
            } else {
                html.push(format!("<pre>{}</pre>", mime::escape_html(&b.text)));
            }
        }
        let markdown = md.join("\n\n");
        let mut html = if has_html {
            html.join("\n")
        } else {
            String::new()
        };
        let cids = crate::html::image_cids(&html);
        let mut resources = HashMap::new();
        let mut inline_bytes = 0usize;
        for p in ps
            .iter()
            .filter(|p| !p.content_id.is_empty() && cids.contains(&p.content_id))
            .take(64)
        {
            if ![
                "image/png",
                "image/jpeg",
                "image/gif",
                "image/webp",
                "image/bmp",
            ]
            .contains(&p.mime.as_str())
            {
                continue;
            }
            let remaining = mime::INLINE_IMAGE_BYTES.saturating_sub(inline_bytes);
            if remaining == 0 || p.size > remaining as u64 {
                continue;
            }
            let raw = fetch_part(&mut s, m.uid, p, remaining as u64).await?;
            if let Some(mail) = mail_parser::MessageParser::default().parse(&raw) {
                if let Some(part) = mail.parts.first() {
                    let bytes = part.contents();
                    if bytes.len() <= remaining {
                        if let Some(uri) = mime::inline_image_uri(&p.mime, bytes) {
                            inline_bytes += bytes.len();
                            resources.entry(p.content_id.clone()).or_insert(uri);
                        }
                    }
                }
            }
        }
        html = crate::html::embed_inline_images(&html, &resources);
        if html.len() > 8 * 1024 * 1024 {
            return Err(fail("包含内嵌图片的正文超过显示上限"));
        }
        let text = mime::plain_text(&html, &markdown);
        let attachments = ps
            .iter()
            .filter(|p| p.attachment)
            .map(|p| AttachmentInfo {
                part_id: p
                    .path
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join("."),
                filename: if p.filename.is_empty() {
                    format!(
                        "附件-{}",
                        p.path
                            .iter()
                            .map(|v| v.to_string())
                            .collect::<Vec<_>>()
                            .join("-")
                    )
                } else {
                    p.filename.clone()
                },
                mime_type: p.mime.clone(),
                size: p.size,
            })
            .collect();
        Ok(MailBody {
            message_id: id.into(),
            content_hash: mime::hash(&markdown),
            text,
            markdown,
            html,
            attachments,
        })
    };
    let result = tokio::time::timeout(Duration::from_secs(60), work)
        .await
        .map_err(|_| fail("正文下载超时"))?;
    if result.is_ok() {
        return_session(&mut guard, s, connection_key);
    }
    result
}
pub async fn flag(e: &MailEngine, id: &str, c: &str, flag: &str, value: bool) -> Result<()> {
    let imapflag = match flag {
        "seen" => "\\Seen",
        "starred" => "\\Flagged",
        _ => return Err(fail("不支持的邮件标记")),
    };
    let m = e.message(id)?;
    let a = e.account(&m.account_id)?;
    if a.provider != "demo" && m.uid > 0 {
        let slot = e.pool.slot(&format!("reader:{}", a.id));
        let mut guard = slot.lock().await;
        let (mut s, connection_key) = take_session(e, &mut guard, &a, c).await?;
        let work = async {
            let mailbox = s
                .select(folder_path(&m))
                .await
                .map_err(|_| fail("无法打开文件夹"))?;
            if mailbox.uid_validity.unwrap_or(0) != m.uid_validity {
                return Err(fail("目录已变更，请刷新"));
            }
            s.uid_store(
                m.uid.to_string(),
                format!("{}FLAGS.SILENT ({imapflag})", if value { "+" } else { "-" }),
            )
            .await
            .map_err(|_| fail("标记失败"))?
            .try_collect::<Vec<_>>()
            .await
            .map_err(|_| fail("标记响应中断"))?;
            Ok(())
        };
        tokio::time::timeout(Duration::from_secs(25), work)
            .await
            .map_err(|_| fail("标记超时，请刷新确认状态"))??;
        return_session(&mut guard, s, connection_key);
    }
    let db = e.connection()?;
    MailEngine::require_account(&db, &a.id)?;
    let copies: Vec<String> = {
        let mut stmt = db
            .prepare("SELECT data FROM messages WHERE canonical_id=?1")
            .map_err(fail)?;
        let result = stmt
            .query_map([&m.canonical_id], |r| r.get(0))
            .map_err(fail)?
            .collect::<std::result::Result<_, _>>()
            .map_err(fail)?;
        result
    };
    for json in copies {
        let mut copy: MessageSummary = serde_json::from_str(&json).map_err(fail)?;
        if flag == "seen" {
            copy.unread = !value
        } else {
            copy.starred = value
        };
        MailEngine::put_message(&db, &copy)?;
    }
    Ok(())
}
pub async fn move_to(e: &MailEngine, id: &str, c: &str, role: &str) -> Result<()> {
    if !["trash", "archive"].contains(&role) {
        return Err(fail("不支持的移动目标"));
    }
    let m = e.message(id)?;
    let a = e.account(&m.account_id)?;
    if a.provider == "demo" {
        e.connection()?
            .execute("DELETE FROM messages WHERE id=?1", [id])
            .map_err(fail)?;
        return Ok(());
    }
    let slot = e.pool.slot(&a.id);
    let mut guard = slot.lock().await;
    let (mut s, connection_key) = take_session(e, &mut guard, &a, c).await?;
    let work = async {
        let mailbox = s
            .select(folder_path(&m))
            .await
            .map_err(|_| fail("无法打开文件夹"))?;
        if mailbox.uid_validity.unwrap_or(0) != m.uid_validity {
            return Err(fail("目录已变更，请刷新"));
        }
        let caps = s
            .capabilities()
            .await
            .map_err(|_| fail("读取邮箱能力失败"))?;
        if a.provider == "gmail" && role == "archive" {
            s.uid_store(m.uid.to_string(), "-X-GM-LABELS.SILENT (\\Inbox)")
                .await
                .map_err(|_| fail("归档失败"))?
                .try_collect::<Vec<_>>()
                .await
                .map_err(|_| fail("归档响应中断"))?;
        } else {
            let folders = e.folders(a.id.clone())?;
            let target = folders
                .iter()
                .find(|f| f.role == role)
                .ok_or_else(|| fail("没有找到目标文件夹，请在邮箱网页创建归档/垃圾箱后刷新"))?;
            if caps.has_str("MOVE") {
                s.uid_mv(m.uid.to_string(), &target.path)
                    .await
                    .map_err(|_| fail("移动邮件失败"))?;
            } else if caps.has_str("UIDPLUS") {
                s.uid_copy(m.uid.to_string(), &target.path)
                    .await
                    .map_err(|_| fail("复制邮件失败"))?;
                s.uid_store(m.uid.to_string(), "+FLAGS.SILENT (\\Deleted)")
                    .await
                    .map_err(|_| fail("移动未完成，请刷新检查"))?
                    .try_collect::<Vec<_>>()
                    .await
                    .map_err(|_| fail("移动状态读取失败"))?;
                s.uid_expunge(m.uid.to_string())
                    .await
                    .map_err(|_| fail("移除原副本未完成"))?
                    .try_collect::<Vec<_>>()
                    .await
                    .map_err(|_| fail("移除响应中断"))?;
            } else {
                return Err(fail("此服务器不支持安全定向移动，请使用邮箱网页操作"));
            }
        }
        Ok(())
    };
    tokio::time::timeout(Duration::from_secs(30), work)
        .await
        .map_err(|_| fail("移动超时，请刷新确认结果"))??;
    return_session(&mut guard, s, connection_key);
    if a.provider == "gmail" && role == "archive" {
        e.connection()?.execute("DELETE FROM messages WHERE canonical_id=?1 AND folder_id IN (SELECT id FROM folders WHERE role='inbox')",[&m.canonical_id]).map_err(fail)?;
    } else {
        e.connection()?
            .execute("DELETE FROM messages WHERE id=?1", [id])
            .map_err(fail)?;
    }
    Ok(())
}
pub async fn idle(e: &MailEngine, id: &str, c: &str) -> Result<bool> {
    let a = e.account(id)?;
    if a.provider == "demo" {
        return Ok(false);
    }
    let slot = e.pool.slot(&format!("idle:{id}"));
    let mut guard = slot.lock().await;
    let (mut s, connection_key) = take_session(e, &mut guard, &a, c).await?;
    let caps = tokio::time::timeout(Duration::from_secs(15), s.capabilities())
        .await
        .map_err(|_| fail("监视连接超时"))?
        .map_err(|_| fail("监视连接中断"))?;
    if !caps.has_str("IDLE") {
        return_session(&mut guard, s, connection_key);
        return Ok(false);
    }
    tokio::time::timeout(Duration::from_secs(15), s.select("INBOX"))
        .await
        .map_err(|_| fail("监视收件箱超时"))?
        .map_err(|_| fail("无法监视收件箱"))?;
    let mut handle = s.idle();
    tokio::time::timeout(Duration::from_secs(15), handle.init())
        .await
        .map_err(|_| fail("开始监视超时"))?
        .map_err(|_| fail("服务器拒绝 IDLE"))?;
    let response = {
        let (fut, _interrupt) = handle.wait_with_timeout(Duration::from_secs(55));
        tokio::time::timeout(Duration::from_secs(60), fut)
            .await
            .map_err(|_| fail("监视连接需要重建"))?
            .map_err(|_| fail("监视连接已中断"))?
    };
    let session = tokio::time::timeout(Duration::from_secs(10), handle.done())
        .await
        .map_err(|_| fail("监视连接超时"))?
        .map_err(|_| fail("监视结束失败"))?;
    return_session(&mut guard, session, connection_key);
    Ok(matches!(
        response,
        async_imap::extensions::idle::IdleResponse::NewData(_)
    ))
}
pub async fn attachment(
    e: &MailEngine,
    id: &str,
    part_id: &str,
    c: &str,
    dest: &str,
) -> Result<()> {
    let m = e.message(id)?;
    let a = e.account(&m.account_id)?;
    if let Some(index) = part_id.strip_prefix("local:") {
        let index: usize = index.parse().map_err(|_| fail("附件编号错误"))?;
        let raw = std::fs::read(e.root.join("imports").join(format!("{}.eml", m.id)))
            .map_err(|_| fail("导入邮件原文不可用"))?;
        let mail = mail_parser::MessageParser::default()
            .parse(&raw)
            .ok_or_else(|| fail("邮件解析失败"))?;
        let part = mail
            .attachments()
            .nth(index)
            .ok_or_else(|| fail("附件不存在"))?;
        std::fs::write(dest, part.contents()).map_err(|_| fail("附件保存失败"))?;
        return Ok(());
    }
    let slot = e.pool.slot(&a.id);
    let mut guard = slot.lock().await;
    let (mut s, connection_key) = take_session(e, &mut guard, &a, c).await?;
    let work = async {
        let (ps, _) = parts(&mut s, &m).await?;
        let part = ps
            .iter()
            .find(|p| {
                p.path
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(".")
                    == part_id
            })
            .ok_or_else(|| fail("附件不存在"))?;
        let raw = fetch_part(&mut s, m.uid, part, 30 * 1024 * 1024).await?;
        let parsed = mail_parser::MessageParser::default()
            .parse(&raw)
            .ok_or_else(|| fail("附件解码失败"))?;
        let data = parsed
            .parts
            .first()
            .ok_or_else(|| fail("附件为空"))?
            .contents();
        std::fs::write(dest, data).map_err(|_| fail("无法写入所选位置"))?;
        Ok(())
    };
    let result = tokio::time::timeout(Duration::from_secs(120), work)
        .await
        .map_err(|_| fail("附件下载超时"))?;
    if result.is_ok() {
        return_session(&mut guard, s, connection_key);
    }
    result
}
pub async fn send(e: &MailEngine, id: &str, credential: &str) -> Result<Draft> {
    send_inner(e, id, credential, true).await
}

pub(crate) async fn send_pending(e: &MailEngine, id: &str, credential: &str) -> Result<Draft> {
    send_inner(e, id, credential, false).await
}

async fn send_inner(
    e: &MailEngine,
    id: &str,
    credential: &str,
    allow_draft: bool,
) -> Result<Draft> {
    use lettre::{
        message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart},
        transport::smtp::{
            authentication::{Credentials, Mechanism},
            client::{Tls, TlsParameters},
        },
        AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    };
    let mut d = e
        .drafts()?
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| fail("草稿不存在"))?;
    if !["queued", "failed"].contains(&d.status.as_str()) && !(allow_draft && d.status == "draft") {
        return Err(fail("此邮件正在发送、已发送或结果待确认，不能重复提交"));
    }
    let a = e.account(&d.account_id)?;
    if a.provider == "demo" {
        return Err(fail("示例模式不发送邮件，请先添加真实账号"));
    }
    if credential.is_empty() {
        return Err(fail("此账号需要重新授权"));
    }
    let addresses = |input: &str| -> Result<Vec<Mailbox>> {
        input
            .split([',', ';', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.parse::<Mailbox>().map_err(|_| fail("收件地址格式不正确")))
            .collect()
    };
    let tos = addresses(&d.to)?;
    let ccs = addresses(&d.cc)?;
    let bccs = addresses(&d.bcc)?;
    if tos.is_empty() && ccs.is_empty() && bccs.is_empty() {
        return Err(fail("请填写收件人"));
    }
    let mut builder = Message::builder()
        .from(a.address.parse().map_err(|_| fail("发件地址格式错误"))?)
        .subject(&d.subject)
        .message_id(Some(format!("<{}@lightmail.local>", d.id)));
    for to in tos {
        builder = builder.to(to);
    }
    for cc in ccs {
        builder = builder.cc(cc);
    }
    for bcc in bccs {
        builder = builder.bcc(bcc);
    }
    if !d.reply_to_message_id.is_empty() {
        builder = builder.in_reply_to(d.reply_to_message_id.clone());
    }
    if !d.references.is_empty() {
        builder = builder.references(d.references.clone());
    }
    let mut multipart = MultiPart::mixed().singlepart(SinglePart::plain(d.body.clone()));
    let mut size = 0u64;
    for path in &d.attachment_paths {
        let meta = std::fs::metadata(path).map_err(|_| fail("附件文件已移动或无法访问"))?;
        size += meta.len();
        if size > 25 * 1024 * 1024 {
            return Err(fail("附件总大小超过 25 MiB"));
        }
        let data = std::fs::read(path).map_err(|_| fail("无法读取附件"))?;
        let filename = std::path::Path::new(path)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        multipart = multipart.singlepart(Attachment::new(filename).body(
            data,
            ContentType::parse("application/octet-stream").unwrap(),
        ));
    }
    let message = builder
        .multipart(multipart)
        .map_err(|_| fail("邮件格式不正确"))?;
    let tls = TlsParameters::new(a.smtp_host.clone()).map_err(|_| fail("发件服务器配置错误"))?;
    #[cfg(test)]
    let tls = if a.smtp_host == "localhost" {
        if let Ok(path) = std::env::var("LIGHTMAIL_TEST_CA") {
            TlsParameters::builder(a.smtp_host.clone())
                .add_root_certificate(
                    lettre::transport::smtp::client::Certificate::from_pem(
                        &std::fs::read(path).map_err(fail)?,
                    )
                    .map_err(fail)?,
                )
                .build_native()
                .map_err(fail)?
        } else {
            tls
        }
    } else {
        tls
    };
    let smtp_route = e.mail_proxy(&a.id, &a.smtp_host)?;
    let smtp_tunnel = if let Some(proxy) = smtp_route.as_ref() {
        Some(crate::proxy::SmtpTunnel::open(&a.smtp_host, a.smtp_port, proxy).await?)
    } else {
        None
    };
    let (smtp_host, smtp_port) = smtp_tunnel
        .as_ref()
        .map(|t| ("127.0.0.1", t.port))
        .unwrap_or((a.smtp_host.as_str(), a.smtp_port));
    let mut smtp = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(smtp_host)
        .port(smtp_port)
        .tls(if a.smtp_port == 465 {
            Tls::Wrapper(tls)
        } else {
            Tls::Required(tls)
        })
        .timeout(Some(Duration::from_secs(45)))
        .credentials(Credentials::new(a.address.clone(), credential.into()));
    if a.auth_kind == "oauth" {
        smtp = smtp.authentication(vec![Mechanism::Xoauth2]);
    }
    let smtp = smtp.build();
    let original = serde_json::to_string(&d).map_err(fail)?;
    d.status = "sending".into();
    d.last_error.clear();
    {
        let db = e.connection()?;
        let changed=db.execute("UPDATE drafts SET status='sending',data=?1 WHERE id=?2 AND data=?3 AND status IN ('draft','queued','failed')",params![serde_json::to_string(&d).map_err(fail)?,d.id,original]).map_err(fail)?;
        if changed != 1 {
            return Err(fail("发送状态已经改变"));
        }
    }
    // Cancellation can happen after SMTP DATA reached the server. Preserve the
    // ambiguity on disk immediately; never turn an interrupted submission into a retry.
    struct Submission<'a>(&'a MailEngine, Draft);
    impl Drop for Submission<'_> {
        fn drop(&mut self) {
            self.1.status = "delivery_unknown".into();
            self.1.last_error = "发送过程已中断，结果待确认，请先检查已发送".into();
            if let (Ok(db), Ok(data)) = (self.0.connection(), serde_json::to_string(&self.1)) {
                let _ = db.execute("UPDATE drafts SET status='delivery_unknown',data=?1 WHERE id=?2 AND status='sending'", params![data,self.1.id]);
            }
        }
    }
    let _submission = Submission(e, d.clone());
    match tokio::time::timeout(Duration::from_secs(75), smtp.send(message.clone())).await {
        Ok(Ok(_)) => {
            d.status = "accepted".into();
            d.last_error.clear();
        }
        Ok(Err(err)) => {
            if err.is_permanent() || err.is_client() {
                d.status = "failed".into();
                d.last_error = "服务器拒绝发送，请检查地址、授权及发送限制".into();
            } else {
                d.status = "delivery_unknown".into();
                d.last_error = "未收到明确发送结果，请先检查已发送，避免重复投递".into();
            }
        }
        Err(_) => {
            d.status = "delivery_unknown".into();
            d.last_error = "发送超时，结果待确认，请先检查已发送".into();
        }
    }
    // Persist acceptance before any sent-folder operation, never replay SMTP for APPEND failure.
    d = e.persist_delivery(d)?;
    if d.status == "accepted" && a.sent_mode == "append" {
        let copy = async {
            let target = e
                .folders(a.id.clone())?
                .into_iter()
                .find(|f| f.role == "sent")
                .ok_or_else(|| fail("未找到已发送文件夹"))?;
            let route = e.mail_proxy(&a.id, &a.imap_host)?;
            let mut s = connect(&a, credential, route).await?;
            s.append(&target.path, Some("(\\Seen)"), None, message.formatted())
                .await
                .map_err(|_| fail("已发送副本保存失败"))?;
            Ok::<(), MailError>(())
        };
        if !matches!(
            tokio::time::timeout(Duration::from_secs(30), copy).await,
            Ok(Ok(()))
        ) {
            d.last_error = "邮件已提交；已发送副本未保存，请在服务端核对。不要重新发送。".into();
            d = e.persist_delivery(d)?;
        }
    }
    Ok(d)
}
