//! Headless consumer tests: the exact API used by Swift is callable without a UI.
use crate::*;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
pub(crate) struct TestPlatform {
    pub secrets: Mutex<HashMap<String, String>>,
    pub route: Mutex<Option<ProxyRoute>>,
}
impl PlatformServices for TestPlatform {
    fn read_secret(&self, key: String) -> Result<Option<String>> {
        Ok(self.secrets.lock().unwrap().get(&key).cloned())
    }
    fn write_secret(&self, key: String, value: String) -> Result<()> {
        self.secrets.lock().unwrap().insert(key, value);
        Ok(())
    }
    fn remove_secret(&self, key: String) -> Result<()> {
        self.secrets.lock().unwrap().remove(&key);
        Ok(())
    }
    fn proxy_for(&self, _host: String) -> Result<ProxyRoute> {
        Ok(self.route.lock().unwrap().clone().unwrap_or(ProxyRoute {
            kind: "direct".into(),
            host: String::new(),
            port: 0,
        }))
    }
}
#[derive(Default)]
pub(crate) struct Events(pub Mutex<Vec<ApplicationEvent>>);
impl ApplicationObserver for Events {
    fn changed(&self, event: ApplicationEvent) {
        self.0.lock().unwrap().push(event);
    }
}
pub(crate) struct Progress;
impl TranslationObserver for Progress {
    fn progress(&self, _done: u32, _total: u32, _blocks: Vec<TranslationBlock>) {}
}
fn fixture() -> (
    tempfile::TempDir,
    Arc<MailEngine>,
    Arc<MailApplication>,
    Arc<TestPlatform>,
    Arc<Events>,
) {
    let directory = tempfile::tempdir().unwrap();
    let engine = MailEngine::new(directory.path().to_string_lossy().into()).unwrap();
    engine.seed_demo().unwrap();
    let platform = Arc::new(TestPlatform::default());
    let events = Arc::new(Events::default());
    let app = MailApplication::new(engine.clone(), platform.clone(), events.clone());
    (directory, engine, app, platform, events)
}
fn query() -> MessageQuery {
    MessageQuery {
        account_id: "demo-work".into(),
        folder_id: String::new(),
        scope: "inbox".into(),
        search: String::new(),
        unread_only: false,
        limit: 100,
        offset: 0,
    }
}
fn config() -> TranslationConfiguration {
    TranslationConfiguration {
        id: "test".into(),
        name: "Test".into(),
        base_url: "https://example.com/v1".into(),
        model: "fixture-valid".into(),
        target_language: "简体中文".into(),
        stream: false,
        output_format: "prompt".into(),
        input_characters: 12000,
        glossary: String::new(),
        engine: "llm".into(),
    }
}

#[test]
fn headless_body_reply_export_and_account_removal() {
    let (_dir, engine, app, platform, events) = fixture();
    let mut message = engine.list_messages(query()).unwrap().remove(0);
    let body = crate::platform::runtime()
        .block_on(app.clone().body(message.id.clone()))
        .unwrap();
    assert!(!body.markdown.is_empty());
    let account = engine
        .accounts()
        .unwrap()
        .into_iter()
        .find(|a| a.id == message.account_id)
        .unwrap();
    message.reply_to_address = "reply@example.com".into();
    message.from_address = "sender@example.com".into();
    message.to_addresses = format!("{}, A@example.com", account.address.to_uppercase());
    message.cc_addresses = "a@example.com, sender@example.com, reply@example.com".into();
    let d = compose_draft(
        account.clone(),
        Some(message.clone()),
        Some(body.clone()),
        ComposeMode::ReplyAll,
        "2026-09-29".into(),
    );
    assert_eq!(d.to, "reply@example.com");
    assert_eq!(d.cc, "A@example.com");
    assert!(d.body.contains("> "));
    let markdown = export_markdown(
        message.clone(),
        body.clone(),
        None,
        ExportMode::Original,
        "date".into(),
    )
    .unwrap();
    assert!(markdown.contains(&body.markdown));
    assert!(export_markdown(message, body, None, ExportMode::Bilingual, "date".into()).is_err());
    platform
        .write_secret(format!("account:{}", account.id), "synthetic".into())
        .unwrap();
    crate::platform::runtime()
        .block_on(app.remove_account(account.id.clone()))
        .unwrap();
    assert!(!engine
        .accounts()
        .unwrap()
        .iter()
        .any(|a| a.id == account.id));
    assert!(platform
        .read_secret(format!("account:{}", account.id))
        .unwrap()
        .is_none());
    assert!(events
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e.kind, ApplicationEventKind::DataChanged)));
}

#[test]
fn headless_queue_undo_and_stop_prevent_unwanted_submission() {
    let (_dir, engine, app, _platform, events) = fixture();
    let mut account = engine.accounts().unwrap().remove(0);
    account.provider = "imap".into();
    account.enabled = false;
    engine.save_account(account.clone()).unwrap();
    let mut draft = compose_draft(account.clone(), None, None, ComposeMode::New, String::new());
    draft.to = "test@example.com".into();
    let queued = crate::platform::runtime()
        .block_on(app.clone().queue(draft.clone()))
        .unwrap();
    assert_eq!(queued.send_after - queued.updated_at, 5);
    assert_eq!(queued.status, "queued");
    app.cancel_queued(queued.id.clone()).unwrap();
    assert!(crate::platform::runtime()
        .block_on(app.clone().submit(queued.id.clone()))
        .is_err());
    assert!(crate::platform::runtime()
        .block_on(crate::transport::send_pending(
            &engine,
            &queued.id,
            "synthetic"
        ))
        .is_err());
    assert_eq!(engine.drafts().unwrap()[0].status, "draft");
    draft.to = "not an address".into();
    assert!(crate::platform::runtime()
        .block_on(app.clone().queue(draft))
        .is_err());
    let mut next = compose_draft(account, None, None, ComposeMode::New, String::new());
    next.to = "test@example.com".into();
    crate::platform::runtime()
        .block_on(app.clone().queue(next))
        .unwrap();
    app.stop();
    std::thread::sleep(Duration::from_millis(30));
    assert!(events
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e.kind, ApplicationEventKind::DraftChanged)));
    assert!(!engine
        .drafts()
        .unwrap()
        .iter()
        .any(|d| d.status == "sending"));
    let weak = Arc::downgrade(&app);
    drop(app);
    assert!(weak.upgrade().is_none());
}

#[test]
fn translation_configuration_migrates_swift_json_and_rejects_partial_cache() {
    let (_dir, engine, app, _, _) = fixture();
    let c = config();
    let json = serde_json::to_string(&vec![c.clone()]).unwrap();
    assert!(json.contains("baseURL"));
    engine
        .set_setting("translation-configs".into(), json)
        .unwrap();
    assert_eq!(
        app.translation_configurations().unwrap()[0].base_url,
        c.base_url
    );
    let m = engine.list_messages(query()).unwrap().remove(0);
    let b = engine.cached_body(m.id.clone()).unwrap().unwrap();
    let mut system = c.clone();
    system.engine = "system".into();
    let mut result = TranslationResult {
        subject: "Title".into(),
        blocks: translation_blocks(b.markdown.clone()),
        source_hash: b.content_hash.clone(),
        model: "platform".into(),
        input_tokens: None,
        output_tokens: None,
    };
    app.save_system_translation(m.id.clone(), b.clone(), system.clone(), result.clone())
        .unwrap();
    assert!(app
        .cached_translation(m.id.clone(), b.clone(), system.clone())
        .unwrap()
        .is_some());
    result.blocks.pop();
    assert!(app
        .save_system_translation(m.id.clone(), b.clone(), system.clone(), result)
        .is_err());
    let mut changed = b;
    changed.content_hash = "new version".into();
    assert!(app
        .cached_translation(m.id, changed, system)
        .unwrap()
        .is_none());
}

#[test]
fn translation_protection_and_endpoint_contract() {
    let p = ProtectedText::new(
        "Due 2026-10-01: USD 12.50 https://example.com `x=3`".into(),
        "T".into(),
    );
    assert!(p.restore(p.protected_text()).is_ok());
    assert!(p
        .restore(p.protected_text().replace("12.50", "12.60"))
        .is_err());
    assert!(p.restore(p.protected_text() + " ⟦KEEP_OTHER_0⟧").is_err());
    assert!(translation_endpoint("http://example.com/v1".into()).is_err());
    assert!(translation_endpoint("https://key@example.com/v1".into()).is_err());
    assert_eq!(
        translation_endpoint("http://127.0.0.1:1234/v1/".into()).unwrap(),
        "http://127.0.0.1:1234/v1/chat/completions"
    );
    let source = "long ".repeat(5000);
    assert_eq!(
        translation_blocks(source.clone())
            .iter()
            .map(|b| b.text.as_str())
            .collect::<String>(),
        source
    );
}

#[test]
fn oauth_pkce_and_loopback_are_owned_by_the_core() {
    let (_dir, engine, _app, platform, _) = fixture();
    let account = engine.accounts().unwrap().remove(0);
    let login =
        GoogleLogin::new(account.clone(), "synthetic-client".into(), platform.clone()).unwrap();
    let url = url::Url::parse(&login.authorization_url()).unwrap();
    let values: HashMap<_, _> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    assert_eq!(values["code_challenge_method"], "S256");
    assert_eq!(values["code_challenge"].len(), 43);
    assert_eq!(values["login_hint"], account.address);
    let redirect = url::Url::parse(&values["redirect_uri"]).unwrap();
    assert_eq!(redirect.host_str(), Some("127.0.0.1"));
    let other = GoogleLogin::new(account, "synthetic-client".into(), platform).unwrap();
    assert_ne!(login.authorization_url(), other.authorization_url());
    let port = redirect.port().unwrap();
    drop(login);
    let released = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    drop(released);
}

#[test]
fn cancellation_reaches_worker_and_stop_disables_commands() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Dropped(Arc<AtomicBool>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    crate::platform::runtime().block_on(async {
        let started = Arc::new(tokio::sync::Notify::new());
        let notice = started.clone();
        let released = Arc::new(AtomicBool::new(false));
        let flag = released.clone();
        let consumer = tokio::spawn(crate::platform::run(async move {
            let _guard = Dropped(flag);
            notice.notify_one();
            std::future::pending::<()>().await;
            Ok(())
        }));
        started.notified().await;
        consumer.abort();
        let _ = consumer.await;
        for _ in 0..100 {
            if released.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        assert!(
            released.load(Ordering::SeqCst),
            "cancelled FFI future left its worker running"
        );
    });
    let (_dir, engine, app, _, _) = fixture();
    let message = engine.list_messages(query()).unwrap().remove(0);
    app.stop();
    assert!(crate::platform::runtime()
        .block_on(app.clone().body(message.id.clone()))
        .is_err());
    app.clone().start().unwrap();
    assert!(crate::platform::runtime()
        .block_on(app.clone().body(message.id))
        .is_ok());
    app.stop();
}

#[test]
#[ignore = "requires loopback synthetic Chat server"]
fn shared_translation_integration() {
    let (_dir, engine, app, platform, _) = fixture();
    let endpoint = std::env::var("LIGHTMAIL_CHAT_ENDPOINT").unwrap();
    let message = engine.list_messages(query()).unwrap().remove(0);
    let body = engine.cached_body(message.id.clone()).unwrap().unwrap();
    for (model, stream, expected) in [
        ("fixture-valid", false, true),
        ("fixture-valid", true, true),
        ("fixture-truncated", false, false),
        ("fixture-truncated", true, false),
        ("fixture-dropped", true, false),
        ("fixture-omitted", false, false),
        ("fixture-denied", false, false),
        ("fixture-redirect", false, false),
    ] {
        let mut c = config();
        c.base_url = endpoint.clone();
        c.model = model.into();
        c.stream = stream;
        let result = crate::platform::runtime().block_on(app.clone().translate(
            message.id.clone(),
            body.clone(),
            c.clone(),
            true,
            Arc::new(Progress),
        ));
        assert_eq!(
            result.is_ok(),
            expected,
            "{model} stream={stream}: {:?}",
            result.err()
        );
        assert_eq!(
            app.cached_translation(message.id.clone(), body.clone(), c)
                .unwrap()
                .is_some(),
            expected
        );
    }
    let client = TranslationClient::new(platform);
    let mut c = config();
    c.base_url = endpoint;
    assert!(crate::platform::runtime()
        .block_on(client.test(c, String::new()))
        .is_ok());
}

// Real loopback HTTP validates the token exchange/identity/refresh wiring without Google accounts.
struct OAuthFixture {
    endpoint: String,
    requests: Arc<Mutex<Vec<(String, String)>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl OAuthFixture {
    fn new(identity: &str, reject: bool) -> Self {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicBool, Ordering};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = requests.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let identity = identity.to_string();
        let worker = std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let Ok((mut socket, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buf = [0; 1024];
                loop {
                    let n = socket.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buf[..n]);
                    if let Some(end) = bytes.windows(4).position(|p| p == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let length = headers
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                    assert!(bytes.len() < 16384);
                }
                let request = String::from_utf8(bytes).unwrap();
                let path = request.split_whitespace().nth(1).unwrap().to_string();
                let body = request.split_once("\r\n\r\n").unwrap().1.to_string();
                observed.lock().unwrap().push((path.clone(), body));
                let body = if path == "/identity" {
                    serde_json::json!({"email":identity,"email_verified":true}).to_string()
                } else {
                    serde_json::json!({"access_token":"synthetic-access","refresh_token":"synthetic-refresh","expires_in":3600}).to_string()
                };
                let status = if reject { "400 Bad Request" } else { "200 OK" };
                let _=write!(socket,"HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body);
            }
        });
        Self {
            endpoint,
            requests,
            stop,
            worker: Some(worker),
        }
    }
    fn endpoints(&self) -> crate::auth::Endpoints {
        crate::auth::Endpoints {
            token: format!("{}/token", self.endpoint),
            identity: format!("{}/identity", self.endpoint),
        }
    }
}
impl Drop for OAuthFixture {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}

#[test]
fn oauth_callback_exchange_and_identity_mismatch() {
    crate::platform::runtime().block_on(async {
        for matches in [true, false] {
            let server = OAuthFixture::new(
                if matches {
                    "demo-work@example.com"
                } else {
                    "other@example.com"
                },
                false,
            );
            let (_dir, engine, _app, platform, _events) = fixture();
            let account = engine
                .accounts()
                .unwrap()
                .into_iter()
                .find(|a| a.id == "demo-work")
                .unwrap();
            let mut login =
                GoogleLogin::new(account, "synthetic-client".into(), platform.clone()).unwrap();
            Arc::get_mut(&mut login).unwrap().endpoints = server.endpoints();
            let url = url::Url::parse(&login.authorization_url()).unwrap();
            let params: HashMap<_, _> = url
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let endpoint = format!(
                "{}?state={}&code=synthetic-code",
                params["redirect_uri"], params["state"]
            );
            let task = tokio::spawn(login.finish(String::new()));
            let result = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(endpoint)
                .send()
                .await
                .unwrap();
            assert!(result.status().is_success());
            assert_eq!(task.await.unwrap().is_ok(), matches);
            assert_eq!(
                platform
                    .read_secret("account:demo-work".into())
                    .unwrap()
                    .is_some(),
                matches
            );
            let observed = server.requests.lock().unwrap();
            assert_eq!(observed.len(), 2);
            let fields: HashMap<_, _> = url::form_urlencoded::parse(observed[0].1.as_bytes())
                .into_owned()
                .collect();
            assert_eq!(fields["grant_type"], "authorization_code");
            assert_eq!(fields["redirect_uri"], params["redirect_uri"]);
            use base64::Engine;
            use sha2::Digest;
            assert_eq!(
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(sha2::Sha256::digest(fields["code_verifier"].as_bytes())),
                params["code_challenge"]
            );
        }
    });
}
#[test]
fn oauth_refresh_is_single_flight_and_failure_preserves_secret() {
    crate::platform::runtime().block_on(async {
        for reject in [false,true] {
            let server=OAuthFixture::new("unused",reject);let (_dir,engine,mut app,platform,_events)=fixture();
            let mut account=engine.accounts().unwrap().remove(0);account.provider="gmail".into();account.auth_kind="oauth".into();engine.save_account(account.clone()).unwrap();
            Arc::get_mut(&mut app).unwrap().oauth_endpoints=server.endpoints();
            engine.set_setting("google-client-id".into(),"synthetic-client".into()).unwrap();
            let original=serde_json::json!({"accessToken":"expired","refreshToken":"keep-on-failure","expiresAt":0}).to_string();
            let key=format!("account:{}",account.id);platform.write_secret(key.clone(),original.clone()).unwrap();
            if reject {assert!(app.credential(&account.id).await.is_err());assert_eq!(platform.read_secret(key).unwrap().unwrap(),original);}
            else {let mut tasks=Vec::new();for _ in 0..10 {let app=app.clone();let id=account.id.clone();tasks.push(tokio::spawn(async move {app.credential(&id).await}));}for task in tasks {assert_eq!(task.await.unwrap().unwrap(),"synthetic-access");}}
            assert_eq!(server.requests.lock().unwrap().len(),1);
        }
    });
}
