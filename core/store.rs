use crate::{mime, models::*, transport};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

#[derive(uniffi::Object)]
pub struct MailEngine {
    pub(crate) db: Mutex<Connection>,
    pub(crate) root: PathBuf,
    pub(crate) runtime: Arc<tokio::runtime::Runtime>,
    pub(crate) pool: transport::Pool,
    pub(crate) proxies: crate::proxy::Routes,
}

fn decode<T: serde::de::DeserializeOwned>(s: String) -> Result<T> {
    serde_json::from_str(&s).map_err(|_| fail("本地数据格式错误"))
}
fn encode<T: serde::Serialize>(v: &T) -> Result<String> {
    serde_json::to_string(v).map_err(|_| fail("无法保存本地数据"))
}

impl MailEngine {
    pub(crate) fn connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.db.lock().map_err(|_| fail("数据库暂不可用"))
    }
    pub(crate) fn require_account(db: &Connection, id: &str) -> Result<()> {
        if !db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM accounts WHERE id=?1)",
                [id],
                |r| r.get::<_, bool>(0),
            )
            .map_err(fail)?
        {
            return Err(fail("账号已移除，同步已停止"));
        }
        Ok(())
    }
    pub(crate) fn persist_delivery(&self, mut d: Draft) -> Result<Draft> {
        d.updated_at = now();
        let db = self.connection()?;
        let n=db.execute("UPDATE drafts SET status=?1,updated=?2,data=?3 WHERE id=?4 AND status IN ('sending','accepted')",params![d.status,d.updated_at,encode(&d)?,d.id]).map_err(fail)?;
        if n != 1 {
            return Err(fail("发送结果无法保存，请检查服务端已发送邮箱"));
        }
        Ok(d)
    }
    /// Records the user's own check of an interrupted submission. Only
    /// delivery_unknown drafts change, and neither outcome submits anything:
    /// a confirmed delivery becomes accepted, otherwise the draft is editable again.
    pub fn resolve_delivery(&self, id: &str, delivered: bool) -> Result<Draft> {
        let db = self.connection()?;
        let json: String = db
            .query_row(
                "SELECT data FROM drafts WHERE id=?1 AND status='delivery_unknown'",
                [id],
                |r| r.get(0),
            )
            .map_err(|_| fail("此邮件不是结果待确认状态"))?;
        let mut d: Draft = decode(json)?;
        d.status = if delivered { "accepted" } else { "draft" }.into();
        d.last_error = if delivered {
            "已在服务端确认送达".into()
        } else {
            "已在服务端确认未送达，邮件已退回草稿".into()
        };
        d.send_after = 0;
        d.updated_at = now();
        let n = db
            .execute(
                "UPDATE drafts SET status=?1,updated=?2,data=?3 WHERE id=?4 AND status='delivery_unknown'",
                params![d.status, d.updated_at, encode(&d)?, d.id],
            )
            .map_err(fail)?;
        if n != 1 {
            return Err(fail("发送状态已经改变"));
        }
        Ok(d)
    }
    pub(crate) fn put_message(db: &Connection, m: &MessageSummary) -> Result<()> {
        let mut merged = m.clone();
        if merged.snippet.is_empty() {
            let previous: Option<String> = db.query_row(
                "SELECT data FROM messages WHERE canonical_id=?1 AND json_extract(data,'$.snippet')<>'' LIMIT 1",
                [&m.canonical_id], |r| r.get(0)).optional().map_err(fail)?;
            if let Some(previous) = previous {
                let previous: MessageSummary = decode(previous)?;
                merged.snippet = previous.snippet;
                merged.has_attachments |= previous.has_attachments;
            } else if let Some((text, has_attachments)) = db.query_row(
                "SELECT json_extract(data,'$.text'),json_array_length(data,'$.attachments')>0 FROM bodies WHERE id=?1",
                [&m.canonical_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?)),
            ).optional().map_err(fail)? {
                // Headers may be rediscovered after their row was removed while the
                // canonical body (shared by Gmail labels) is still cached.
                merged.snippet = mime::preview(&text);
                merged.has_attachments |= has_attachments;
            }
        }
        let m = &merged;
        db.execute("INSERT INTO messages(id,account_id,folder_id,canonical_id,ts,unread,starred,subject,sender,search_text,data) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(id) DO UPDATE SET unread=excluded.unread,starred=excluded.starred,subject=excluded.subject,sender=excluded.sender,ts=excluded.ts,data=excluded.data",
          params![m.id,m.account_id,m.folder_id,m.canonical_id,m.timestamp,m.unread,m.starred,m.subject,m.from_address,format!("{} {} {} {}",m.subject,m.from_name,m.from_address,m.snippet),encode(m)?]).map_err(fail)?;
        Ok(())
    }
    pub(crate) fn message(&self, id: &str) -> Result<MessageSummary> {
        let s: String = self
            .connection()?
            .query_row("SELECT data FROM messages WHERE id=?1", [id], |r| r.get(0))
            .map_err(|_| fail("邮件已移动或不存在，请刷新"))?;
        decode(s)
    }
    pub(crate) fn account(&self, id: &str) -> Result<Account> {
        let s: String = self
            .connection()?
            .query_row("SELECT data FROM accounts WHERE id=?1", [id], |r| r.get(0))
            .map_err(|_| fail("邮箱账号不存在"))?;
        decode(s)
    }
    fn store_preview(db: &Connection, canonical: &str, body: &MailBody) -> Result<()> {
        let snippet = mime::preview(&body.text);
        db.execute("UPDATE messages SET data=json_set(data,'$.snippet',?1,'$.has_attachments',json(?2)) WHERE canonical_id=?3 AND (json_extract(data,'$.snippet') IS NOT ?1 OR json_extract(data,'$.has_attachments') IS NOT json_extract(?2,'$'))",
            params![snippet, if body.attachments.is_empty() { "false" } else { "true" }, canonical]).map_err(fail)?;
        Ok(())
    }
    pub(crate) fn repair_cached_previews(&self) -> Result<()> {
        let db = self.connection()?;
        let done: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM settings WHERE key='body-preview-version' AND value='1')", [], |r| r.get(0)).map_err(fail)?;
        // Retain only IDs, never collect every body into memory during migration.
        // Empty previews can reappear after header rediscovery, so repair those
        // on every startup even when the original text migration already ran.
        let ids: Vec<String> = {
            let mut query = db.prepare("SELECT b.id FROM bodies b WHERE ?1=0 OR EXISTS(SELECT 1 FROM messages m WHERE m.canonical_id=b.id AND coalesce(json_extract(m.data,'$.snippet'),'')='')").map_err(fail)?;
            let rows = query.query_map([done], |r| r.get(0)).map_err(fail)?;
            rows.collect::<std::result::Result<_, _>>().map_err(fail)?
        };
        for id in ids {
            let json: String = db
                .query_row("SELECT data FROM bodies WHERE id=?1", [&id], |r| r.get(0))
                .map_err(fail)?;
            let mut body: MailBody = decode(json)?;
            if !done {
                body.text = mime::plain_text(&body.html, &body.markdown);
            }
            Self::store_preview(&db, &id, &body)?;
            if !done {
                let json = encode(&body)?;
                db.execute(
                    "UPDATE bodies SET data=?1,bytes=?2 WHERE id=?3",
                    params![json, json.len() as i64, id],
                )
                .map_err(fail)?;
            }
        }
        db.execute("INSERT OR REPLACE INTO settings(key,value,updated) VALUES('body-preview-version','1',?1)", [now()]).map_err(fail)?;
        Ok(())
    }
    pub(crate) fn store_body_internal(&self, body: &MailBody) -> Result<()> {
        let m = self.message(&body.message_id)?;
        let db = self.connection()?;
        Self::require_account(&db, &m.account_id)?;
        Self::store_preview(&db, &m.canonical_id, body)?;
        if !Self::may_cache(&db, &m)? {
            return Ok(());
        }
        let json = encode(body)?;
        db.execute("INSERT INTO bodies(id,account_id,data,bytes,touched) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET data=excluded.data,bytes=excluded.bytes,touched=excluded.touched",params![m.canonical_id,m.account_id,json,json.len() as i64,now()]).map_err(fail)?;
        db.execute(
            "UPDATE messages SET search_text=?1 WHERE canonical_id=?2",
            params![
                format!(
                    "{} {} {} {}",
                    m.subject, m.from_name, m.from_address, body.text
                ),
                m.canonical_id
            ],
        )
        .map_err(fail)?;
        let total: i64 = db
            .query_row("SELECT coalesce(sum(bytes),0) FROM bodies", [], |r| {
                r.get(0)
            })
            .map_err(fail)?;
        if total > 512 * 1024 * 1024 {
            db.execute("DELETE FROM bodies WHERE id IN (SELECT b.id FROM bodies b JOIN accounts a ON a.id=b.account_id WHERE json_extract(a.data,'$.provider') NOT IN ('demo','local') AND b.id NOT IN (SELECT canonical_id FROM messages WHERE json_extract(data,'$.uid')=0) ORDER BY touched LIMIT max(1,(SELECT count(*)/10 FROM bodies)))",[]).map_err(fail)?;
            db.execute("UPDATE messages SET search_text=subject||' '||sender WHERE canonical_id NOT IN (SELECT id FROM bodies)",[]).map_err(fail)?;
        }
        Ok(())
    }
}

#[uniffi::export]
impl MailEngine {
    #[uniffi::constructor]
    pub fn new(directory: String) -> Result<Arc<Self>> {
        let root = PathBuf::from(directory);
        std::fs::create_dir_all(root.join("attachments")).map_err(fail)?;
        std::fs::create_dir_all(root.join("imports")).map_err(fail)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
                .map_err(fail)?;
        }
        let db = Connection::open(root.join("mail.sqlite")).map_err(fail)?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(fail)?;
        let vacuum: i64 = db
            .query_row("PRAGMA auto_vacuum", [], |row| row.get(0))
            .map_err(fail)?;
        if vacuum != 2 {
            db.execute_batch("PRAGMA auto_vacuum=INCREMENTAL; VACUUM;")
                .map_err(fail)?;
        }
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;
        CREATE TABLE IF NOT EXISTS accounts(id TEXT PRIMARY KEY,data TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS folders(id TEXT PRIMARY KEY,account_id TEXT NOT NULL,path TEXT NOT NULL,role TEXT NOT NULL,validity INTEGER NOT NULL DEFAULT 0,data TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS messages(id TEXT PRIMARY KEY,account_id TEXT NOT NULL,folder_id TEXT NOT NULL,canonical_id TEXT NOT NULL,ts INTEGER NOT NULL,unread INTEGER NOT NULL,starred INTEGER NOT NULL,subject TEXT NOT NULL,sender TEXT NOT NULL,search_text TEXT NOT NULL,data TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS message_folder_time ON messages(folder_id,ts DESC);
        CREATE INDEX IF NOT EXISTS message_account_time ON messages(account_id,ts DESC);
        CREATE INDEX IF NOT EXISTS message_time ON messages(ts DESC);
        CREATE INDEX IF NOT EXISTS message_canonical ON messages(canonical_id);
        CREATE TABLE IF NOT EXISTS bodies(id TEXT PRIMARY KEY,account_id TEXT NOT NULL,data TEXT NOT NULL,bytes INTEGER NOT NULL,touched INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS drafts(id TEXT PRIMARY KEY,account_id TEXT NOT NULL,status TEXT NOT NULL,updated INTEGER NOT NULL,data TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL,updated INTEGER NOT NULL);
        CREATE VIRTUAL TABLE IF NOT EXISTS message_fts USING fts5(search_text,content='messages',content_rowid='rowid',tokenize='trigram');
        CREATE TRIGGER IF NOT EXISTS message_ai AFTER INSERT ON messages BEGIN INSERT INTO message_fts(rowid,search_text) VALUES(new.rowid,new.search_text); END;
        CREATE TRIGGER IF NOT EXISTS message_ad AFTER DELETE ON messages BEGIN INSERT INTO message_fts(message_fts,rowid,search_text) VALUES('delete',old.rowid,old.search_text); END;
        CREATE TRIGGER IF NOT EXISTS message_au AFTER UPDATE OF search_text ON messages BEGIN INSERT INTO message_fts(message_fts,rowid,search_text) VALUES('delete',old.rowid,old.search_text); INSERT INTO message_fts(rowid,search_text) VALUES(new.rowid,new.search_text); END;").map_err(fail)?;
        // A crash during submission cannot safely be replayed as another SMTP send.
        let pending: Vec<String> = {
            let mut s = db
                .prepare("SELECT data FROM drafts WHERE status IN ('sending','queued')")
                .map_err(fail)?;
            let rows = s.query_map([], |r| r.get(0)).map_err(fail)?;
            rows.collect::<std::result::Result<_, _>>().map_err(fail)?
        };
        for json in pending {
            let mut d: Draft = decode(json)?;
            if d.status == "sending" {
                d.status = "delivery_unknown".into();
                d.last_error = "上次发送时应用退出，结果待确认。请先检查已发送邮箱。".into();
            } else {
                d.status = "draft".into();
                d.send_after = 0;
                d.last_error = "应用在发送前退出，已恢复为草稿，请检查后发送。".into();
            }
            db.execute(
                "UPDATE drafts SET status=?1,data=?2 WHERE id=?3",
                params![d.status, encode(&d)?, d.id],
            )
            .map_err(fail)?;
        }
        let runtime = crate::platform::runtime();
        // Cached HTML from earlier builds had all inline styles removed.
        let render_version: Option<String> = db
            .query_row(
                "SELECT value FROM settings WHERE key='body-render-version'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(fail)?;
        if render_version.as_deref() != Some("2") {
            db.execute_batch("DELETE FROM bodies WHERE account_id IN (SELECT id FROM accounts WHERE json_extract(data,'$.provider') NOT IN ('demo','local')) AND id NOT IN (SELECT canonical_id FROM messages WHERE json_extract(data,'$.uid')=0);
              UPDATE messages SET search_text=subject||' '||sender WHERE canonical_id NOT IN (SELECT id FROM bodies);
              INSERT OR REPLACE INTO settings(key,value,updated) VALUES('body-render-version','2',0);").map_err(fail)?;
        }
        let engine = Arc::new(Self {
            db: Mutex::new(db),
            root,
            runtime,
            pool: transport::Pool::default(),
            proxies: crate::proxy::Routes::default(),
        });
        for account in engine.accounts()? {
            engine.prune_recent_bodies(&account.id)?;
        }
        engine.repair_cached_previews()?;
        Ok(engine)
    }
    pub fn configure_proxy(
        &self,
        destination: String,
        kind: String,
        host: String,
        port: u16,
    ) -> Result<()> {
        if self.proxies.set(destination, kind, host, port)? {
            self.pool.clear();
        }
        Ok(())
    }
    pub fn accounts(&self) -> Result<Vec<Account>> {
        let db = self.connection()?;
        let mut s = db
            .prepare("SELECT data FROM accounts ORDER BY rowid")
            .map_err(fail)?;
        let result = s
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(fail)?
            .map(|r| decode(r.map_err(fail)?))
            .collect();
        result
    }
    pub fn save_account(&self, account: Account) -> Result<()> {
        if account.id.is_empty()
            || !account.address.contains('@')
            || account.imap_host.contains(['\r', '\n', '/'])
            || account.smtp_host.contains(['\r', '\n', '/'])
        {
            return Err(fail("邮箱配置不完整"));
        }
        self.connection()?.execute("INSERT INTO accounts VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET data=excluded.data",params![account.id,encode(&account)?]).map_err(fail)?;
        self.pool.invalidate(&account.id);
        Ok(())
    }
    pub fn remove_account(&self, account_id: String) -> Result<()> {
        self.pool.invalidate(&account_id);
        let mut db = self.connection()?;
        let tx = db.transaction().map_err(fail)?;
        let imported: Vec<String> = {
            let mut statement = tx
                .prepare(
                    "SELECT id FROM messages WHERE account_id=?1 AND json_extract(data,'$.uid')=0",
                )
                .map_err(fail)?;
            let rows = statement
                .query_map([&account_id], |r| r.get(0))
                .map_err(fail)?
                .collect::<std::result::Result<_, _>>()
                .map_err(fail)?;
            rows
        };
        for id in imported {
            let _ = std::fs::remove_file(self.root.join("imports").join(format!("{id}.eml")));
        }
        for table in ["messages", "folders", "bodies", "drafts"] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE account_id=?1"),
                [&account_id],
            )
            .map_err(fail)?;
        }
        tx.execute("DELETE FROM accounts WHERE id=?1", [&account_id])
            .map_err(fail)?;
        tx.execute(
            "DELETE FROM settings WHERE key LIKE ?1",
            [format!("translation:{}:%", account_id)],
        )
        .map_err(fail)?;
        tx.commit().map_err(fail)?;
        Ok(())
    }
    pub fn folders(&self, account_id: String) -> Result<Vec<Folder>> {
        let db = self.connection()?;
        let mut s=db.prepare("SELECT f.data,(SELECT count(*) FROM messages m WHERE m.folder_id=f.id AND unread=1) FROM folders f WHERE (?1='' OR account_id=?1) ORDER BY CASE role WHEN 'inbox' THEN 0 WHEN 'sent' THEN 1 WHEN 'drafts' THEN 2 ELSE 3 END,path").map_err(fail)?;
        let result = s
            .query_map([account_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
            })
            .map_err(fail)?
            .map(|r| {
                let (json, unread) = r.map_err(fail)?;
                let mut f: Folder = decode(json)?;
                f.unread_count = unread;
                Ok(f)
            })
            .collect();
        result
    }
    pub fn list_messages(&self, query: MessageQuery) -> Result<Vec<MessageSummary>> {
        let db = self.connection()?;
        let search = format!(
            "%{}%",
            query
                .search
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let indexed = query.search.chars().count() >= 3 && !query.search.contains(['%', '_', '\\']);
        let extra = if indexed {
            " AND m.rowid IN (SELECT rowid FROM message_fts WHERE search_text LIKE ?6)"
        } else {
            ""
        };
        let index = if !query.folder_id.is_empty() {
            "message_folder_time"
        } else if !query.account_id.is_empty() {
            "message_account_time"
        } else {
            "message_time"
        };
        let sql=format!("SELECT m.data,m.canonical_id FROM messages m INDEXED BY {index} JOIN folders f ON f.id=m.folder_id WHERE (?1='' OR m.account_id=?1) AND (?2='' OR m.folder_id=?2) AND (?3='all' OR (?3='starred' AND m.starred=1 AND f.role NOT IN ('trash','junk')) OR f.role=?3) AND (?4=0 OR m.unread=1) AND (?5='' OR m.search_text LIKE ?6 ESCAPE '\\'){extra} ORDER BY m.ts DESC");
        let mut statement = db.prepare(&sql).map_err(fail)?;
        let mut rows = statement
            .query(params![
                query.account_id,
                query.folder_id,
                query.scope,
                query.unread_only,
                query.search,
                search
            ])
            .map_err(fail)?;
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        let mut skipped = 0;
        while let Some(row) = rows.next().map_err(fail)? {
            let canonical: String = row.get(1).map_err(fail)?;
            if !seen.insert(canonical) {
                continue;
            }
            if skipped < query.offset {
                skipped += 1;
                continue;
            }
            result.push(decode(row.get(0).map_err(fail)?)?);
            if result.len() >= query.limit.clamp(1, 500) as usize {
                break;
            }
        }
        Ok(result)
    }
    pub fn cached_body(&self, message_id: String) -> Result<Option<MailBody>> {
        let m = self.message(&message_id)?;
        let db = self.connection()?;
        let s: Option<String> = db
            .query_row(
                "SELECT data FROM bodies WHERE id=?1",
                [&m.canonical_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(fail)?;
        if let Some(s) = s {
            let mut body: MailBody = decode(s)?;
            body.message_id = message_id;
            if !body.html.is_empty() && !body.html.contains(crate::html::RENDER_MARKER) {
                // Recover invisible image-only links immediately from old cache.
                // Keep it marked stale so preload can restore the original CSS.
                body.html = crate::html::restore_link_labels(&body.html);
                body.markdown = crate::markdown::from_html(&body.html)?;
                body.text = mime::plain_text(&body.html, &body.markdown);
                body.content_hash = mime::hash(&body.markdown);
            }
            // A cache hit still has to restore the list preview. Both foreground
            // reads and preload_body use this path and otherwise skip persistence.
            Self::store_preview(&db, &m.canonical_id, &body)?;
            db.execute(
                "UPDATE bodies SET touched=?1 WHERE id=?2",
                params![now(), m.canonical_id],
            )
            .map_err(fail)?;
            Ok(Some(body))
        } else {
            Ok(None)
        }
    }
    pub fn setting(&self, key: String) -> Result<Option<String>> {
        self.connection()?
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |r| {
                r.get(0)
            })
            .optional()
            .map_err(fail)
    }
    pub fn set_setting(&self, key: String, value: String) -> Result<()> {
        if value.len() > 8 * 1024 * 1024 {
            return Err(fail("数据过大"));
        }
        let db = self.connection()?;
        db.execute("INSERT INTO settings VALUES(?1,?2,?3) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated=excluded.updated",params![key,value,now()]).map_err(fail)?;
        let total:i64=db.query_row("SELECT coalesce(sum(length(value)),0) FROM settings WHERE key LIKE 'translation:%'",[],|r|r.get(0)).map_err(fail)?;
        if total > 32 * 1024 * 1024 {
            db.execute("DELETE FROM settings WHERE key IN (SELECT key FROM settings WHERE key LIKE 'translation:%' ORDER BY updated LIMIT 10)",[]).map_err(fail)?;
        }
        Ok(())
    }
    pub fn save_draft(&self, mut draft: Draft) -> Result<Draft> {
        if !["draft", "queued"].contains(&draft.status.as_str()) {
            return Err(fail("此操作不能修改发送结果"));
        }
        draft.updated_at = now();
        if draft.id.is_empty() {
            draft.id = uuid::Uuid::new_v4().to_string();
            draft.created_at = now();
        }
        let db = self.connection()?;
        let n=db.execute("INSERT INTO drafts VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET account_id=excluded.account_id,status=excluded.status,updated=excluded.updated,data=excluded.data WHERE drafts.status IN ('draft','failed')",params![draft.id,draft.account_id,draft.status,draft.updated_at,encode(&draft)?]).map_err(fail)?;
        if n != 1 {
            return Err(fail("邮件已进入发送流程，旧草稿未覆盖发送状态"));
        }
        Ok(draft)
    }
    pub fn cancel_queued(&self, id: String) -> Result<()> {
        let db = self.connection()?;
        let json: String = db
            .query_row(
                "SELECT data FROM drafts WHERE id=?1 AND status='queued'",
                [&id],
                |r| r.get(0),
            )
            .map_err(|_| fail("邮件已开始提交，无法撤销"))?;
        let mut d: Draft = decode(json)?;
        d.status = "draft".into();
        d.send_after = 0;
        d.updated_at = now();
        db.execute(
            "UPDATE drafts SET status='draft',updated=?1,data=?2 WHERE id=?3",
            params![d.updated_at, encode(&d)?, id],
        )
        .map_err(fail)?;
        Ok(())
    }
    pub fn fail_unsubmitted(&self, id: String, message: String) -> Result<()> {
        let db = self.connection()?;
        let json: Option<String> = db
            .query_row(
                "SELECT data FROM drafts WHERE id=?1 AND status IN ('queued','failed')",
                [&id],
                |r| r.get(0),
            )
            .optional()
            .map_err(fail)?;
        if let Some(json) = json {
            let mut d: Draft = decode(json)?;
            d.status = "failed".into();
            d.last_error = message;
            d.updated_at = now();
            db.execute(
                "UPDATE drafts SET status='failed',updated=?1,data=?2 WHERE id=?3",
                params![d.updated_at, encode(&d)?, id],
            )
            .map_err(fail)?;
        }
        Ok(())
    }
    pub fn drafts(&self) -> Result<Vec<Draft>> {
        let db = self.connection()?;
        let mut s = db
            .prepare("SELECT data FROM drafts ORDER BY updated DESC")
            .map_err(fail)?;
        let result = s
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(fail)?
            .map(|r| decode(r.map_err(fail)?))
            .collect();
        result
    }
    pub fn delete_draft(&self, id: String) -> Result<()> {
        self.connection()?
            .execute(
                "DELETE FROM drafts WHERE id=?1 AND status NOT IN ('sending','delivery_unknown')",
                [id],
            )
            .map_err(fail)?;
        Ok(())
    }
    pub fn storage_info(&self) -> Result<StorageInfo> {
        let db = self.connection()?;
        Ok(StorageInfo {
            message_count: db
                .query_row(
                    "SELECT count(DISTINCT canonical_id) FROM messages",
                    [],
                    |r| r.get(0),
                )
                .map_err(fail)?,
            body_count: db
                .query_row("SELECT count(*) FROM bodies", [], |r| r.get(0))
                .map_err(fail)?,
            cache_bytes: db
                .query_row("SELECT coalesce(sum(bytes),0) FROM bodies", [], |r| {
                    r.get(0)
                })
                .map_err(fail)?,
            database_bytes: std::fs::metadata(self.root.join("mail.sqlite"))
                .map(|m| m.len())
                .unwrap_or(0),
        })
    }
    pub fn clear_body_cache(&self) -> Result<()> {
        let db = self.connection()?;
        db.execute("DELETE FROM bodies WHERE account_id IN (SELECT id FROM accounts WHERE json_extract(data,'$.provider') NOT IN ('demo','local')) AND id NOT IN (SELECT canonical_id FROM messages WHERE json_extract(data,'$.uid')=0)",[]).map_err(fail)?;
        db.execute("DELETE FROM settings WHERE key LIKE 'translation:%'", [])
            .map_err(fail)?;
        db.execute("UPDATE messages SET search_text=subject||' '||sender WHERE canonical_id NOT IN (SELECT id FROM bodies)",[]).map_err(fail)?;
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")
            .map_err(fail)?;
        Ok(())
    }
    pub fn import_eml(&self, path: String, account_id: String) -> Result<MessageSummary> {
        if std::fs::metadata(&path).map_err(fail)?.len() > 30 * 1024 * 1024 {
            return Err(fail("导入邮件超过 30 MiB"));
        }
        let bytes = std::fs::read(path).map_err(fail)?;
        let account = self.account(&account_id)?;
        let folder = Folder {
            id: folder_id(&account_id, "local"),
            account_id: account_id.clone(),
            path: "local".into(),
            name: "本地导入".into(),
            role: "inbox".into(),
            unread_count: 0,
            total_count: 0,
        };
        let mut m = mime::summary(&bytes, &account, &folder, 0, 0);
        m.id = uuid::Uuid::new_v4().to_string();
        m.canonical_id = format!("{}:{}", account_id, m.id);
        {
            let db = self.connection()?;
            db.execute("INSERT OR IGNORE INTO folders(id,account_id,path,role,data) VALUES(?1,?2,?3,?4,?5)",params![folder.id,account_id,folder.path,folder.role,encode(&folder)?]).map_err(fail)?;
            Self::put_message(&db, &m)?;
        }
        let body = mime::parse_body(&bytes, &m.id)?;
        std::fs::write(
            self.root.join("imports").join(format!("{}.eml", m.id)),
            &bytes,
        )
        .map_err(fail)?;
        self.store_body_internal(&body)?;
        Ok(m)
    }
    pub fn sync_account(&self, account_id: String, credential: String) -> Result<SyncResult> {
        self.runtime
            .block_on(transport::sync(self, &account_id, &credential, None, false))
    }
    pub fn sync_folder(
        &self,
        account_id: String,
        credential: String,
        path: String,
        older: bool,
    ) -> Result<SyncResult> {
        self.runtime.block_on(transport::sync(
            self,
            &account_id,
            &credential,
            Some(path),
            older,
        ))
    }
    pub fn fetch_body(&self, message_id: String, credential: String) -> Result<MailBody> {
        if let Some(b) = self.cached_body(message_id.clone())? {
            return Ok(b);
        }
        let m = self.message(&message_id)?;
        let body = if m.uid == 0 {
            let raw = std::fs::read(self.root.join("imports").join(format!("{}.eml", m.id)))
                .map_err(|_| fail("本地邮件原文不可用"))?;
            mime::parse_body(&raw, &m.id)?
        } else {
            self.runtime
                .block_on(transport::body(self, &message_id, &credential))?
        };
        self.store_body_internal(&body)?;
        Ok(body)
    }
    pub fn recent_messages(&self, account_id: String) -> Result<Vec<MessageSummary>> {
        Self::recent_remote(&*self.connection()?, &account_id)
    }
    pub fn prune_body_cache(&self, account_id: String) -> Result<u32> {
        self.prune_recent_bodies(&account_id)
    }
    pub fn preload_body(&self, message_id: String, credential: String) -> Result<()> {
        if let Some(body) = self.cached_body(message_id.clone())? {
            if body.html.is_empty() || body.html.contains(crate::html::RENDER_MARKER) {
                return Ok(());
            }
        }
        let message = self.message(&message_id)?;
        if !Self::may_cache(&*self.connection()?, &message)? {
            return Ok(());
        }
        let body =
            self.runtime
                .block_on(transport::prefetch_body(self, &message_id, &credential))?;
        self.store_body_internal(&body)
    }
    pub fn change_flag(
        &self,
        message_id: String,
        credential: String,
        flag: String,
        value: bool,
    ) -> Result<()> {
        self.runtime.block_on(transport::flag(
            self,
            &message_id,
            &credential,
            &flag,
            value,
        ))
    }
    pub fn move_message(&self, message_id: String, credential: String, role: String) -> Result<()> {
        self.runtime
            .block_on(transport::move_to(self, &message_id, &credential, &role))
    }
    pub fn wait_for_change(&self, account_id: String, credential: String) -> Result<bool> {
        self.runtime
            .block_on(transport::idle(self, &account_id, &credential))
    }
    pub fn send_draft(&self, id: String, credential: String) -> Result<Draft> {
        self.runtime
            .block_on(transport::send(self, &id, &credential))
    }
    pub fn download_attachment(
        &self,
        message_id: String,
        part_id: String,
        credential: String,
        destination: String,
    ) -> Result<()> {
        self.runtime.block_on(transport::attachment(
            self,
            &message_id,
            &part_id,
            &credential,
            &destination,
        ))
    }
    pub fn seed_demo(&self) -> Result<()> {
        mime::seed_demo(self)
    }
}
