use crate::{models::*, store::MailEngine};
use rusqlite::{params, Connection};
use std::collections::HashSet;

impl MailEngine {
    pub(crate) fn recent_remote(db: &Connection, account: &str) -> Result<Vec<MessageSummary>> {
        // A Gmail message can have several labels; count its canonical ID once.
        let mut statement = db.prepare("WITH ranked AS (
          SELECT m.data,m.ts,m.canonical_id,
          row_number() OVER (PARTITION BY m.canonical_id ORDER BY CASE f.role WHEN 'inbox' THEN 0 ELSE 1 END,m.id) AS n
          FROM messages m JOIN folders f ON f.id=m.folder_id
          WHERE m.account_id=?1 AND json_extract(m.data,'$.uid')>0 AND f.role NOT IN ('trash','junk')
        ) SELECT data FROM ranked WHERE n=1 ORDER BY ts DESC,canonical_id DESC LIMIT 20").map_err(fail)?;
        let json = statement
            .query_map([account], |row| row.get::<_, String>(0))
            .map_err(fail)?;
        json.map(|row| serde_json::from_str(&row.map_err(fail)?).map_err(fail))
            .collect()
    }
    pub(crate) fn may_cache(db: &Connection, message: &MessageSummary) -> Result<bool> {
        if message.uid == 0 {
            return Ok(true);
        }
        let local: bool = db.query_row("SELECT json_extract(data,'$.provider') IN ('demo','local') FROM accounts WHERE id=?1",[&message.account_id],|row| row.get(0)).map_err(fail)?;
        Ok(local
            || Self::recent_remote(db, &message.account_id)?
                .iter()
                .any(|m| m.canonical_id == message.canonical_id))
    }
    pub(crate) fn prune_recent_bodies(&self, account: &str) -> Result<u32> {
        let mut db = self.connection()?;
        let local: bool = db.query_row("SELECT json_extract(data,'$.provider') IN ('demo','local') FROM accounts WHERE id=?1",[account],|row|row.get(0)).map_err(fail)?;
        if local {
            return Ok(0);
        }
        let keep: HashSet<_> = Self::recent_remote(&db, account)?
            .into_iter()
            .map(|m| m.canonical_id)
            .collect();
        let expired: Vec<String> = {
            let mut statement=db.prepare("SELECT id FROM bodies WHERE account_id=?1 AND id NOT IN (SELECT canonical_id FROM messages WHERE json_extract(data,'$.uid')=0)").map_err(fail)?;
            let rows = statement
                .query_map([account], |row| row.get::<_, String>(0))
                .map_err(fail)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(fail)?
                .into_iter()
                .filter(|id| !keep.contains(id))
                .collect()
        };
        if expired.is_empty() {
            return Ok(0);
        }
        let tx = db.transaction().map_err(fail)?;
        for id in &expired {
            tx.execute("DELETE FROM bodies WHERE id=?1", [id])
                .map_err(fail)?;
            tx.execute(
                "UPDATE messages SET search_text=subject||' '||sender WHERE canonical_id=?1",
                [id],
            )
            .map_err(fail)?;
        }
        let prefix = format!("translation:{account}:");
        tx.execute("DELETE FROM settings WHERE substr(key,1,length(?1))=?1 AND json_valid(value) AND json_extract(value,'$.sourceHash') NOT IN (SELECT json_extract(data,'$.content_hash') FROM bodies WHERE account_id=?2)",params![prefix,account]).map_err(fail)?;
        tx.commit().map_err(fail)?;
        // Incremental vacuum returns freed cache pages without rebuilding the database.
        db.execute_batch("PRAGMA incremental_vacuum(256); PRAGMA wal_checkpoint(PASSIVE);")
            .map_err(fail)?;
        Ok(expired.len() as u32)
    }
}
