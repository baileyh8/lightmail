use crate::*;
use rusqlite::params;

fn engine() -> (tempfile::TempDir, std::sync::Arc<MailEngine>) {
    let dir = tempfile::tempdir().unwrap();
    let e = MailEngine::new(dir.path().to_string_lossy().into()).unwrap();
    (dir, e)
}
fn query() -> MessageQuery {
    MessageQuery {
        account_id: String::new(),
        folder_id: String::new(),
        scope: "inbox".into(),
        search: String::new(),
        unread_only: false,
        limit: 100,
        offset: 0,
    }
}
fn draft(id: &str) -> Draft {
    Draft {
        id: id.into(),
        account_id: "demo-work".into(),
        to: "recipient@example.com".into(),
        cc: String::new(),
        bcc: String::new(),
        subject: "Work in progress".into(),
        body: "Preserve this draft".into(),
        attachment_paths: vec![],
        reply_to_message_id: String::new(),
        references: String::new(),
        status: "draft".into(),
        last_error: String::new(),
        created_at: now(),
        updated_at: now(),
        send_after: 0,
    }
}

#[test]
fn account_filter_is_strict() {
    let (_d, e) = engine();
    e.seed_demo().unwrap();
    let mut q = query();
    q.account_id = "demo-work".into();
    let rows = e.list_messages(q).unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|m| m.account_id == "demo-work"));
}
#[test]
fn shared_gmail_labels_deduplicate_only_in_same_account() {
    let (_d, e) = engine();
    e.seed_demo().unwrap();
    let m = e.list_messages(query()).unwrap().remove(0);
    let mut alias = m.clone();
    alias.id = "alias-row".into();
    alias.folder_id = folder_id(&m.account_id, "Projects");
    {
        let db = e.connection().unwrap();
        MailEngine::put_message(&db, &alias).unwrap();
    }
    let mut q = query();
    q.scope = "all".into();
    assert_eq!(e.list_messages(q).unwrap().len(), 7);
}
#[test]
fn same_message_id_across_accounts_is_not_merged() {
    let (_d, e) = engine();
    e.seed_demo().unwrap();
    let rows = e.list_messages(query()).unwrap();
    let mut m = rows[0].clone();
    m.id = "other-account".into();
    m.account_id = "demo-personal".into();
    m.folder_id = folder_id("demo-personal", "INBOX");
    m.canonical_id = "demo-personal:distinct".into();
    {
        let db = e.connection().unwrap();
        MailEngine::put_message(&db, &m).unwrap();
    }
    assert_eq!(e.list_messages(query()).unwrap().len(), 8);
}
#[test]
fn chinese_two_character_search_finds_cached_body() {
    let (_d, e) = engine();
    e.seed_demo().unwrap();
    let mut q = query();
    q.search = "周四".into();
    let found = e.list_messages(q).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].from_name, "林悦");
}
#[test]
fn cache_clear_keeps_drafts_and_headers() {
    let (_d, e) = engine();
    e.seed_demo().unwrap();
    e.save_draft(draft("safe")).unwrap();
    e.clear_body_cache().unwrap();
    assert_eq!(e.list_messages(query()).unwrap().len(), 7);
    assert_eq!(e.drafts().unwrap()[0].body, "Preserve this draft");
    assert_eq!(e.storage_info().unwrap().body_count, 7);
}
#[test]
fn crash_during_send_becomes_unknown_not_requeued() {
    let (dir, e) = engine();
    let mut d = draft("crash");
    e.save_draft(d.clone()).unwrap();
    d.status = "sending".into();
    e.connection()
        .unwrap()
        .execute(
            "UPDATE drafts SET status='sending',data=?1 WHERE id=?2",
            params![serde_json::to_string(&d).unwrap(), d.id],
        )
        .unwrap();
    drop(e);
    let reopened = MailEngine::new(dir.path().to_string_lossy().into()).unwrap();
    assert_eq!(reopened.drafts().unwrap()[0].status, "delivery_unknown");
}
#[test]
fn deleting_account_is_scoped() {
    let (_d, e) = engine();
    e.seed_demo().unwrap();
    e.remove_account("demo-work".into()).unwrap();
    assert_eq!(e.accounts().unwrap().len(), 3);
    assert_eq!(e.list_messages(query()).unwrap().len(), 4);
    assert_eq!(e.storage_info().unwrap().body_count, 4);
}
#[test]
fn demo_does_not_send() {
    let (_d, e) = engine();
    e.seed_demo().unwrap();
    e.save_draft(draft("refuse-demo")).unwrap();
    assert!(e.send_draft("refuse-demo".into(), "".into()).is_err());
    assert_eq!(e.drafts().unwrap()[0].status, "draft");
}
#[test]
fn headers_do_not_store_credentials() {
    let (_d, e) = engine();
    e.seed_demo().unwrap();
    let db = e.connection().unwrap();
    let count: i64 = db
        .query_row(
            "SELECT count(*) FROM accounts WHERE data LIKE ?1",
            params!["%password%"],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
#[test]
fn sql_wildcards_are_literal() {
    let (_d, e) = engine();
    e.seed_demo().unwrap();
    let mut q = query();
    q.search = "%".into();
    assert!(e.list_messages(q).unwrap().is_empty());
}

#[test]
#[ignore = "requires isolated local TLS protocol fixtures"]
fn local_protocol_integration() {
    use std::io::{Read, Write};
    let ports: serde_json::Value = serde_json::from_slice(
        &std::fs::read(std::env::var("LIGHTMAIL_TEST_PORTS").unwrap()).unwrap(),
    )
    .unwrap();
    let state = || {
        let mut stream =
            std::net::TcpStream::connect(("127.0.0.1", ports["control"].as_u64().unwrap() as u16))
                .unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(b"GET / HTTP/1.0\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        serde_json::from_str::<serde_json::Value>(response.split("\r\n\r\n").nth(1).unwrap())
            .unwrap()
    };
    let (_dir, e) = engine();
    let a = Account {
        id: "fixture".into(),
        name: "Local TLS fixture".into(),
        address: "sender@example.com".into(),
        provider: "gmail".into(),
        imap_host: "localhost".into(),
        imap_port: ports["imap"].as_u64().unwrap() as u16,
        smtp_host: "localhost".into(),
        smtp_port: ports["smtp"].as_u64().unwrap() as u16,
        auth_kind: "password".into(),
        color: "#226451".into(),
        enabled: true,
        sent_mode: "server".into(),
    };
    let proxy_kind = std::env::var("LIGHTMAIL_PROXY_KIND").unwrap_or_else(|_| "direct".into());
    if proxy_kind != "direct" {
        e.configure_proxy(
            "localhost".into(),
            proxy_kind.clone(),
            "127.0.0.1".into(),
            ports[&proxy_kind].as_u64().unwrap() as u16,
        )
        .unwrap();
    }
    e.save_account(a).unwrap();
    let sync = e
        .sync_account("fixture".into(), "fixture-only".into())
        .unwrap();
    assert_eq!(sync.added, 1);
    let rows = e.list_messages(query()).unwrap();
    assert_eq!(rows.len(), 1);
    let m = &rows[0];
    assert!(m.has_attachments);
    assert!(m.canonical_id.ends_with("gmail:1001"));
    assert_eq!(state()["body_fetches"], 0);
    assert_eq!(state()["attachment_fetches"], 0);
    // Deliberately hold the whole-account sync connection while opening a body.
    // The old shared connection blocks here; the reader must finish independently.
    let sync_slot = e.pool.slot("fixture");
    let sync_guard = e.runtime.block_on(sync_slot.lock());
    let started = std::time::Instant::now();
    let body = e.fetch_body(m.id.clone(), "fixture-only".into()).unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    drop(sync_guard);
    assert!(body.markdown.contains("12.50"));
    assert_eq!(body.attachments.len(), 1);
    assert_eq!(state()["body_fetches"], 1);
    assert_eq!(state()["attachment_fetches"], 0);
    let dest = _dir.path().join("attachment.bin");
    e.download_attachment(
        m.id.clone(),
        "2".into(),
        "fixture-only".into(),
        dest.to_string_lossy().into(),
    )
    .unwrap();
    assert_eq!(std::fs::read(dest).unwrap(), b"fixture attachment bytes");
    e.change_flag(m.id.clone(), "fixture-only".into(), "seen".into(), true)
        .unwrap();
    assert!(!e.list_messages(query()).unwrap()[0].unread);
    assert!(e
        .wait_for_change("fixture".into(), "fixture-only".into())
        .unwrap());
    let mut accepted = draft("accepted");
    accepted.account_id = "fixture".into();
    accepted.bcc = "hidden@example.com".into();
    e.save_draft(accepted).unwrap();
    assert_eq!(
        e.send_draft("accepted".into(), "fixture-only".into())
            .unwrap()
            .status,
        "accepted"
    );
    assert!(e
        .send_draft("accepted".into(), "fixture-only".into())
        .is_err());
    assert_eq!(state()["smtp_accepted"], 1);
    assert_eq!(state()["bcc_header_seen"], false);
    let mut rejected = draft("rejected");
    rejected.account_id = "fixture".into();
    rejected.to = "reject@example.com".into();
    e.save_draft(rejected).unwrap();
    assert_eq!(
        e.send_draft("rejected".into(), "fixture-only".into())
            .unwrap()
            .status,
        "failed"
    );
    let mut unknown = draft("unknown");
    unknown.account_id = "fixture".into();
    unknown.to = "drop@example.com".into();
    e.save_draft(unknown).unwrap();
    assert_eq!(
        e.send_draft("unknown".into(), "fixture-only".into())
            .unwrap()
            .status,
        "delivery_unknown"
    );
    assert!(e
        .send_draft("unknown".into(), "fixture-only".into())
        .is_err());
    if proxy_kind != "direct" {
        assert!(state()["proxy_connects"].as_u64().unwrap() >= 5);
    } else {
        assert_eq!(state()["proxy_connects"], 0);
    }
    println!("Validated {proxy_kind}: TLS IMAP metadata, selective MIME fetch, attachment decode, flags, IDLE, SMTP accepted/rejected/ambiguous, Bcc privacy and duplicate-send prevention.");
}

#[test]
fn stale_autosave_cannot_override_queued_or_accepted() {
    let (_dir, e) = engine();
    let original = draft("guarded");
    e.save_draft(original.clone()).unwrap();
    let mut queued = original.clone();
    queued.status = "queued".into();
    e.save_draft(queued).unwrap();
    assert!(e.save_draft(original.clone()).is_err());
    assert_eq!(e.drafts().unwrap()[0].status, "queued");
    e.cancel_queued(original.id.clone()).unwrap();
    assert_eq!(e.drafts().unwrap()[0].status, "draft");
    e.connection()
        .unwrap()
        .execute("UPDATE drafts SET status='accepted'", [])
        .unwrap();
    assert!(e.save_draft(original).is_err());
}
#[test]
fn imported_attachment_survives_clear_cache() {
    let (dir, e) = engine();
    e.seed_demo().unwrap();
    let source = dir.path().join("mail.eml");
    std::fs::write(&source,b"From: a@example.com\r\nSubject: Import\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain\r\n\r\nKept locally.\r\n--x\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=sample.txt\r\nContent-Transfer-Encoding: base64\r\n\r\naGVsbG8=\r\n--x--\r\n").unwrap();
    let m = e
        .import_eml(source.to_string_lossy().into(), "demo-work".into())
        .unwrap();
    e.clear_body_cache().unwrap();
    assert!(e
        .fetch_body(m.id.clone(), "".into())
        .unwrap()
        .text
        .contains("Kept locally"));
    let dest = dir.path().join("export.txt");
    e.download_attachment(
        m.id,
        "local:0".into(),
        "".into(),
        dest.to_string_lossy().into(),
    )
    .unwrap();
    assert_eq!(std::fs::read(dest).unwrap(), b"hello");
}
#[test]
fn indexed_search_matches_full_body() {
    let (_dir, e) = engine();
    e.seed_demo().unwrap();
    let rows = e.list_messages(query()).unwrap();
    let m = &rows[0];
    let mut b = e.cached_body(m.id.clone()).unwrap().unwrap();
    b.text = "独特的中文全文测试串 and specific-phrase@example.com".into();
    e.store_body_internal(&b).unwrap();
    for term in ["中文全文", "specific-phrase", "@example.com"] {
        let mut q = query();
        q.search = term.into();
        assert!(!e.list_messages(q).unwrap().is_empty(), "{term}");
    }
}

#[test]
#[ignore = "release benchmark with 100k messages and 10k cached bodies"]
fn performance_dataset() {
    use std::time::Instant;
    let (dir, e) = engine();
    e.seed_demo().unwrap();
    let accounts = e.accounts().unwrap();
    let template = e.list_messages(query()).unwrap()[0].clone();
    let text="Business review 更新项目进展。The delivery schedule depends on final approval; please preserve dates and totals. ".repeat(22);
    let mut body = MailBody {
        message_id: String::new(),
        text: text.clone(),
        markdown: text.clone(),
        html: String::new(),
        attachments: vec![],
        content_hash: mime::hash(&text),
    };
    {
        let mut db = e.connection().unwrap();
        let tx = db.transaction().unwrap();
        tx.execute("DELETE FROM messages", []).unwrap();
        tx.execute("DELETE FROM bodies", []).unwrap();
        let extra = Account {
            id: "benchmark-fifth".into(),
            ..accounts[0].clone()
        };
        tx.execute(
            "INSERT INTO accounts VALUES(?1,?2)",
            params![extra.id, serde_json::to_string(&extra).unwrap()],
        )
        .unwrap();
        let mut ids = accounts.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
        ids.push(extra.id);
        for (index, account) in ids.iter().enumerate() {
            let f = Folder {
                id: folder_id(account, "INBOX"),
                account_id: account.clone(),
                path: "INBOX".into(),
                role: "inbox".into(),
                name: "Inbox".into(),
                total_count: 20000,
                unread_count: 0,
            };
            tx.execute("INSERT OR REPLACE INTO folders(id,account_id,path,role,data) VALUES(?1,?2,?3,?4,?5)",params![f.id,f.account_id,f.path,f.role,serde_json::to_string(&f).unwrap()]).unwrap();
            for i in 0..20000 {
                let n = index * 20000 + i;
                let mut m = template.clone();
                m.id = format!("bench:{n}");
                m.canonical_id = m.id.clone();
                m.account_id = account.clone();
                m.folder_id = f.id.clone();
                m.uid = i as u32 + 1;
                m.timestamp = 1700000000 + n as i64;
                m.subject = format!("Review {n} 项目进展");
                MailEngine::put_message(&tx, &m).unwrap();
                if i % 10 == 0 {
                    body.message_id = m.id.clone();
                    let json = serde_json::to_string(&body).unwrap();
                    tx.execute(
                        "INSERT INTO bodies VALUES(?1,?2,?3,?4,?5)",
                        params![m.id, m.account_id, json, json.len(), now()],
                    )
                    .unwrap();
                    tx.execute(
                        "UPDATE messages SET search_text=search_text||?1 WHERE id=?2",
                        params![text, m.id],
                    )
                    .unwrap();
                }
            }
        }
        tx.commit().unwrap();
        db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); PRAGMA optimize;")
            .unwrap();
    }
    let measure = |name: &str, mut op: Box<dyn FnMut()>| {
        let mut times = Vec::new();
        for _ in 0..30 {
            let start = Instant::now();
            op();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(|a, b| a.total_cmp(b));
        serde_json::json!({"case":name,"p50_ms":times[15],"p95_ms":times[28]})
    };
    let inbox = measure(
        "unified_inbox_100",
        Box::new(|| {
            assert_eq!(e.list_messages(query()).unwrap().len(), 100);
        }),
    );
    let search = measure(
        "indexed_fulltext",
        Box::new(|| {
            let mut q = query();
            q.search = "final approval".into();
            assert_eq!(e.list_messages(q).unwrap().len(), 100);
        }),
    );
    let short = measure(
        "two_character_search",
        Box::new(|| {
            let mut q = query();
            q.search = "项目".into();
            assert_eq!(e.list_messages(q).unwrap().len(), 100);
        }),
    );
    let cached = measure(
        "cached_body",
        Box::new(|| {
            assert!(e.cached_body("bench:0".into()).unwrap().is_some());
        }),
    );
    let result = serde_json::json!({"accounts":5,"messages":100000,"cached_bodies":10000,"body_text_bytes_each":text.len(),"sqlite_bytes":std::fs::metadata(dir.path().join("mail.sqlite")).unwrap().len(),"samples_per_case":30,"measurements":[inbox,search,short,cached],"scope":"local warm core calls; not UI latency or real provider performance"});
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    if let Ok(path) = std::env::var("LIGHTMAIL_BENCH_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
}

#[test]
fn queued_mail_restores_as_draft_after_restart() {
    let (dir, e) = engine();
    let mut d = draft("queued-restart");
    d.status = "queued".into();
    e.save_draft(d).unwrap();
    drop(e);
    let reopened = MailEngine::new(dir.path().to_string_lossy().into()).unwrap();
    assert_eq!(reopened.drafts().unwrap()[0].status, "draft");
}
#[test]
fn reply_to_header_is_preserved() {
    let (_dir, e) = engine();
    e.seed_demo().unwrap();
    let a = e.accounts().unwrap().remove(0);
    let f = e.folders(a.id.clone()).unwrap().remove(0);
    let m=mime::summary(b"From: noreply@example.com\r\nReply-To: support@example.com\r\nSubject: Reply\r\n\r\nHello",&a,&f,1,1);
    assert_eq!(m.reply_to_address, "support@example.com");
}

#[test]
fn each_account_keeps_its_own_twenty_bodies_and_evicts_on_arrival() {
    let (_dir, e) = engine();
    e.seed_demo().unwrap();
    let template = e.list_messages(query()).unwrap()[0].clone();
    let account_template = e.accounts().unwrap()[0].clone();
    for account in ["cache-a", "cache-b"] {
        let a = Account {
            id: account.into(),
            provider: "gmail".into(),
            address: format!("{account}@example.com"),
            ..account_template.clone()
        };
        e.save_account(a).unwrap();
        let f = Folder {
            id: folder_id(account, "INBOX"),
            account_id: account.into(),
            name: "Inbox".into(),
            path: "INBOX".into(),
            role: "inbox".into(),
            total_count: 20,
            unread_count: 20,
        };
        e.connection()
            .unwrap()
            .execute(
                "INSERT INTO folders(id,account_id,path,role,data) VALUES(?1,?2,?3,?4,?5)",
                params![
                    f.id,
                    account,
                    f.path,
                    f.role,
                    serde_json::to_string(&f).unwrap()
                ],
            )
            .unwrap();
        for uid in 1..=20 {
            let m = MessageSummary {
                id: format!("{account}-{uid}"),
                canonical_id: format!("{account}-{uid}"),
                account_id: account.into(),
                folder_id: f.id.clone(),
                uid,
                timestamp: uid as i64,
                ..template.clone()
            };
            MailEngine::put_message(&e.connection().unwrap(), &m).unwrap();
            e.store_body_internal(&MailBody {
                message_id: m.id.clone(),
                content_hash: crate::mime::hash(&m.id),
                text: "Cache content".into(),
                markdown: "Cache content".into(),
                html: String::new(),
                attachments: vec![],
            })
            .unwrap();
        }
    }
    // Alias rows do not consume additional entries in the twenty-message window.
    let mut alias = e.message("cache-a-20").unwrap();
    alias.id = "label-alias".into();
    MailEngine::put_message(&e.connection().unwrap(), &alias).unwrap();
    assert_eq!(e.recent_messages("cache-a".into()).unwrap().len(), 20);
    let first = e.cached_body("cache-a-1".into()).unwrap().unwrap();
    let next = MessageSummary {
        id: "cache-a-21".into(),
        canonical_id: "cache-a-21".into(),
        account_id: "cache-a".into(),
        folder_id: folder_id("cache-a", "INBOX"),
        uid: 21,
        timestamp: 21,
        ..template
    };
    MailEngine::put_message(&e.connection().unwrap(), &next).unwrap();
    // Eviction happens on new headers, before the new body's download finishes.
    assert_eq!(e.prune_body_cache("cache-a".into()).unwrap(), 1);
    assert!(e.cached_body("cache-a-1".into()).unwrap().is_none());
    assert!(e.cached_body("cache-b-1".into()).unwrap().is_some());
    // An old message can be read, but cannot take a cache slot again.
    e.store_body_internal(&first).unwrap();
    assert!(e.cached_body("cache-a-1".into()).unwrap().is_none());
    e.store_body_internal(&MailBody {
        message_id: next.id,
        content_hash: "new".into(),
        ..first
    })
    .unwrap();
    for account in ["cache-a", "cache-b"] {
        let count: i64 = e
            .connection()
            .unwrap()
            .query_row(
                "SELECT count(*) FROM bodies WHERE account_id=?1",
                [account],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 20);
    }
    assert_eq!(
        e.list_messages(MessageQuery {
            account_id: "cache-a".into(),
            ..query()
        })
        .unwrap()
        .len(),
        21
    );
}

#[test]
fn cached_previews_survive_header_refresh_aliases_and_restart() {
    let (dir, e) = engine();
    e.seed_demo().unwrap();
    let mut message = e.list_messages(query()).unwrap()[0].clone();
    message.snippet.clear();
    let body = crate::mime::parse_body(b"From: fixture@example.com\r\nSubject: Preview\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p style='color:red'>A real <b>preview</b> <a href='https://example.com'>link</a></p>", &message.id).unwrap();
    e.store_body_internal(&body).unwrap();
    let expected = e.message(&message.id).unwrap().snippet;
    assert!(expected.contains("A real preview"));
    assert!(!expected.contains('<') && !expected.contains("](https"));
    // Periodic header sync brings empty snippets; preserve the downloaded one.
    message.unread = !message.unread;
    MailEngine::put_message(&e.connection().unwrap(), &message).unwrap();
    assert_eq!(e.message(&message.id).unwrap().snippet, expected);
    let mut alias = message.clone();
    alias.id = "preview-label-copy".into();
    MailEngine::put_message(&e.connection().unwrap(), &alias).unwrap();
    assert_eq!(e.message(&alias.id).unwrap().snippet, expected);
    // Simulate the old build's empty previews, then repair only from local bodies.
    e.connection()
        .unwrap()
        .execute(
            "UPDATE messages SET data=json_set(data,'$.snippet','') WHERE canonical_id=?1",
            [&message.canonical_id],
        )
        .unwrap();
    e.connection()
        .unwrap()
        .execute("DELETE FROM settings WHERE key='body-preview-version'", [])
        .unwrap();
    drop(e);
    let e = MailEngine::new(dir.path().to_string_lossy().into()).unwrap();
    assert_eq!(e.message(&message.id).unwrap().snippet, expected);
    assert_eq!(e.message(&alias.id).unwrap().snippet, expected);
    assert!(e.cached_body(message.id).unwrap().is_some());
    assert_eq!(crate::mime::preview(" \n\t"), "（正文为空）");
    assert_eq!(
        crate::mime::preview(&"字".repeat(100_000)).chars().count(),
        140
    );
}

#[test]
fn qq_missing_transfer_encoding_keeps_message_structure() {
    use imap_proto::types::{AttributeValue, BodyStructure, ContentEncoding, Response};
    // Synthetic shape matching QQ's NIL encoding response; no real mail data.
    for encoding in ["NIL", "nil", "\"7BIT\"", "\"BASE64\""] {
        let wire = format!("* 1 FETCH (UID 1 BODYSTRUCTURE (\"TEXT\" \"PLAIN\" (\"CHARSET\" \"UTF-8\") NIL NIL {encoding} 12 1 NIL NIL NIL))\r\n");
        let (remaining, response) = imap_proto::parser::parse_response(wire.as_bytes())
            .expect("QQ's missing encoding must not abort the FETCH batch");
        assert!(remaining.is_empty());
        let Response::Fetch(_, attributes) = response else {
            panic!("expected FETCH")
        };
        let AttributeValue::BodyStructure(BodyStructure::Text { other, .. }) = &attributes[1]
        else {
            panic!("expected text structure")
        };
        assert_eq!(
            other.transfer_encoding,
            if encoding == "\"BASE64\"" {
                ContentEncoding::Base64
            } else {
                ContentEncoding::SevenBit
            }
        );
        assert_eq!(other.octets, 12);
    }
    let invalid = b"* 1 FETCH (UID 1 BODYSTRUCTURE (\"TEXT\" \"PLAIN\" NIL NIL NIL NIL NIL 1))\r\n";
    assert!(
        imap_proto::parser::parse_response(invalid).is_err(),
        "do not relax the size field"
    );
}
