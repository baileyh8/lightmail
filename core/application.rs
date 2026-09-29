//! Shared application service. Both native frontends call commands and observe events;
//! neither frontend owns synchronization, credential refresh, cache or outbox policy.
use crate::{
    auth::{self, GoogleTokens},
    composition::validate_translation,
    models::*,
    platform::*,
    translation::*,
    transport, MailEngine,
};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, Notify};

#[derive(Clone, Debug, uniffi::Enum)]
pub enum ApplicationEventKind {
    DataChanged,
    SyncStarted,
    SyncFinished,
    DraftChanged,
    Problem,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ApplicationEvent {
    pub kind: ApplicationEventKind,
    pub account_id: String,
    pub message: String,
    pub failed: bool,
}
#[uniffi::export(with_foreign)]
pub trait ApplicationObserver: Send + Sync {
    fn changed(&self, event: ApplicationEvent);
}

#[derive(Default)]
struct Lane {
    sync: AsyncMutex<()>,
    credential: Arc<AsyncMutex<()>>,
    preload: Notify,
}
struct Job {
    generation: uuid::Uuid,
    abort: tokio::task::AbortHandle,
}
#[derive(uniffi::Object)]
pub struct MailApplication {
    engine: Arc<MailEngine>,
    platform: Arc<dyn PlatformServices>,
    observer: Arc<dyn ApplicationObserver>,
    lanes: Mutex<HashMap<String, Arc<Lane>>>,
    workers: Mutex<HashMap<String, Vec<tokio::task::AbortHandle>>>,
    sends: Mutex<HashMap<String, Job>>,
    active: AtomicBool,
    cancellation: Mutex<tokio_util::sync::CancellationToken>,
}
impl Drop for MailApplication {
    fn drop(&mut self) {
        self.stop();
    }
}

#[uniffi::export]
impl MailApplication {
    #[uniffi::constructor]
    pub fn new(
        engine: Arc<MailEngine>,
        platform: Arc<dyn PlatformServices>,
        observer: Arc<dyn ApplicationObserver>,
    ) -> Arc<Self> {
        Arc::new(Self {
            engine,
            platform,
            observer,
            lanes: Mutex::new(HashMap::new()),
            workers: Mutex::new(HashMap::new()),
            sends: Mutex::new(HashMap::new()),
            active: AtomicBool::new(true),
            cancellation: Mutex::new(tokio_util::sync::CancellationToken::new()),
        })
    }
    /// Idempotent. Account monitoring is independent of foreground reading and sending.
    pub fn start(self: Arc<Self>) -> Result<()> {
        let mut cancellation = self.cancellation.lock().unwrap();
        if cancellation.is_cancelled() {
            *cancellation = tokio_util::sync::CancellationToken::new();
        }
        self.launch_workers()
    }

    pub fn stop(&self) {
        let cancellation = self.cancellation.lock().unwrap();
        cancellation.cancel();
        for (_, handles) in self.workers.lock().unwrap().drain() {
            for handle in handles {
                handle.abort();
            }
        }
        for (id, job) in self.sends.lock().unwrap().drain() {
            let _ = self.engine.cancel_queued(id);
            job.abort.abort();
        }
    }
    pub fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::Relaxed);
    }
    pub fn request_preload(&self, account_id: String) {
        self.lane(&account_id).preload.notify_one();
    }
    pub async fn refresh(self: Arc<Self>) -> Result<()> {
        self.clone()
            .command(async move {
                let accounts = self.engine.accounts()?;
                let results = futures_util::future::join_all(
                    accounts
                        .into_iter()
                        .filter(|a| a.enabled && !["demo", "local"].contains(&a.provider.as_str()))
                        .map(|a| {
                            let app = self.clone();
                            async move { app.sync_inner(&a.id, None, false).await }
                        }),
                )
                .await;
                // Individual errors are published with account identity; one failure must not
                // prevent the remaining accounts from refreshing.
                for result in results {
                    let _ = result;
                }
                Ok(())
            })
            .await
    }
    pub async fn sync(
        self: Arc<Self>,
        account_id: String,
        path: Option<String>,
        older: bool,
    ) -> Result<()> {
        self.clone()
            .command(async move { self.sync_inner(&account_id, path, older).await })
            .await
    }
    pub async fn body(self: Arc<Self>, message_id: String) -> Result<MailBody> {
        self.clone()
            .command(async move { self.body_inner(&message_id).await })
            .await
    }
    pub async fn mark(
        self: Arc<Self>,
        message_id: String,
        flag: String,
        value: bool,
    ) -> Result<()> {
        self.clone()
            .command(async move {
                let m = self.engine.message(&message_id)?;
                let token = self.credential(&m.account_id).await?;
                transport::flag(&self.engine, &message_id, &token, &flag, value).await?;
                self.data(&m.account_id);
                Ok(())
            })
            .await
    }
    pub async fn move_message(self: Arc<Self>, message_id: String, role: String) -> Result<()> {
        self.clone()
            .command(async move {
                let m = self.engine.message(&message_id)?;
                let token = self.credential(&m.account_id).await?;
                transport::move_to(&self.engine, &message_id, &token, &role).await?;
                self.engine.prune_body_cache(m.account_id.clone())?;
                self.data(&m.account_id);
                self.request_preload(m.account_id);
                Ok(())
            })
            .await
    }
    pub async fn download(
        self: Arc<Self>,
        message_id: String,
        part_id: String,
        destination: String,
    ) -> Result<()> {
        self.clone()
            .command(async move {
                let m = self.engine.message(&message_id)?;
                let token = self.credential(&m.account_id).await?;
                transport::attachment(&self.engine, &message_id, &part_id, &token, &destination)
                    .await
            })
            .await
    }
    pub async fn save_account(self: Arc<Self>, account: Account, password: String) -> Result<()> {
        self.clone()
            .command(async move {
                let lane = self.lane(&account.id);
                let guard = lane.credential.clone().lock_owned().await;
                tokio::task::spawn_blocking(move || {
                    let _guard = guard;
                    if !password.is_empty() {
                        self.platform
                            .write_secret(format!("account:{}", account.id), password)?;
                    }
                    self.stop_account(&account.id);
                    self.engine.save_account(account.clone())?;
                    self.data(&account.id);
                    self.reconcile_workers()
                })
                .await
                .map_err(|_| fail("账号保存已中断"))?
            })
            .await
    }
    pub async fn remove_account(self: Arc<Self>, account_id: String) -> Result<()> {
        self.clone()
            .command(async move {
                let lane = self.lane(&account_id);
                let guard = lane.credential.clone().lock_owned().await;
                tokio::task::spawn_blocking(move || {
                    let _guard = guard;
                    self.platform
                        .remove_secret(format!("account:{account_id}"))?;
                    self.stop_account(&account_id);
                    let ids: Vec<_> = self
                        .engine
                        .drafts()?
                        .into_iter()
                        .filter(|d| d.account_id == account_id)
                        .map(|d| d.id)
                        .collect();
                    for id in ids {
                        if let Some(job) = self.sends.lock().unwrap().remove(&id) {
                            job.abort.abort();
                        }
                    }
                    self.engine.remove_account(account_id.clone())?;
                    self.lanes.lock().unwrap().remove(&account_id);
                    self.data(&account_id);
                    Ok(())
                })
                .await
                .map_err(|_| fail("账号移除已中断"))?
            })
            .await
    }

    pub async fn queue(self: Arc<Self>, mut draft: Draft) -> Result<Draft> {
        self.clone()
            .command(async move {
                let account = self.engine.account(&draft.account_id)?;
                if ["local", "demo"].contains(&account.provider.as_str()) {
                    return Err(fail("示例或本地导入账号不能发送邮件，请添加真实邮箱"));
                }
                validate_recipients(&draft)?;
                draft.status = "queued".into();
                draft.send_after = now() + 5;
                let draft = self.engine.save_draft(draft)?;
                let mut jobs = self.sends.lock().unwrap();
                let weak = Arc::downgrade(&self);
                let id = draft.id.clone();
                let generation = uuid::Uuid::new_v4();
                let handle = runtime().spawn(async move {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    if let Some(app) = weak.upgrade() {
                        let _ = app.submit_inner(&id).await;
                        let mut jobs = app.sends.lock().unwrap();
                        if jobs.get(&id).is_some_and(|j| j.generation == generation) {
                            jobs.remove(&id);
                        }
                    }
                });
                if let Some(old) = jobs.insert(
                    draft.id.clone(),
                    Job {
                        generation,
                        abort: handle.abort_handle(),
                    },
                ) {
                    old.abort.abort();
                }
                drop(jobs);
                self.emit(
                    ApplicationEventKind::DraftChanged,
                    &draft.account_id,
                    String::new(),
                    false,
                );
                Ok(draft)
            })
            .await
    }
    pub async fn submit(self: Arc<Self>, id: String) -> Result<Draft> {
        self.clone()
            .command(async move { self.submit_inner(&id).await })
            .await
    }
    pub fn cancel_queued(&self, id: String) -> Result<()> {
        self.engine.cancel_queued(id.clone())?;
        if let Some(job) = self.sends.lock().unwrap().remove(&id) {
            job.abort.abort();
        }
        self.emit(ApplicationEventKind::DraftChanged, "", String::new(), false);
        Ok(())
    }
    pub fn translation_configurations(&self) -> Result<Vec<TranslationConfiguration>> {
        self.engine
            .setting("translation-configs".into())?
            .map(|s| serde_json::from_str(&s).map_err(fail))
            .unwrap_or(Ok(vec![]))
    }
    pub fn save_translation_configurations(
        &self,
        configurations: Vec<TranslationConfiguration>,
        selected_id: String,
    ) -> Result<()> {
        let mut ids = std::collections::HashSet::new();
        for configuration in &configurations {
            validate_translation_configuration(configuration.clone())?;
            if !ids.insert(&configuration.id) {
                return Err(fail("翻译配置标识重复"));
            }
        }
        if !selected_id.is_empty() && !ids.contains(&selected_id) {
            return Err(fail("默认翻译配置不存在"));
        }
        self.engine.set_setting(
            "translation-configs".into(),
            serde_json::to_string(&configurations).map_err(fail)?,
        )?;
        self.engine
            .set_setting("translation-default".into(), selected_id)
    }
    pub fn cached_translation(
        &self,
        message_id: String,
        body: MailBody,
        configuration: TranslationConfiguration,
    ) -> Result<Option<TranslationResult>> {
        let m = self.engine.message(&message_id)?;
        let key = translation_cache_key(m.account_id, body.clone(), configuration, m.subject);
        let result = self
            .engine
            .setting(key)?
            .and_then(|s| serde_json::from_str::<TranslationResult>(&s).ok());
        Ok(result.filter(|r| validate_translation(&body, r).is_ok()))
    }
    pub fn save_system_translation(
        &self,
        message_id: String,
        body: MailBody,
        configuration: TranslationConfiguration,
        result: TranslationResult,
    ) -> Result<()> {
        if configuration.engine != "system" {
            return Err(fail("此配置不是系统翻译"));
        }
        self.save_translation(&message_id, &body, &configuration, &result)
    }
    pub async fn translate(
        self: Arc<Self>,
        message_id: String,
        body: MailBody,
        configuration: TranslationConfiguration,
        force: bool,
        observer: Arc<dyn TranslationObserver>,
    ) -> Result<TranslationResult> {
        self.clone()
            .command(async move {
                if !force {
                    if let Some(result) = self.cached_translation(
                        message_id.clone(),
                        body.clone(),
                        configuration.clone(),
                    )? {
                        return Ok(result);
                    }
                }
                let m = self.engine.message(&message_id)?;
                if configuration.engine != "llm" {
                    return Err(fail("系统翻译须由平台语言服务执行"));
                }
                let key = secret_read(
                    self.platform.clone(),
                    format!("translation:{}", configuration.id),
                )
                .await?
                .unwrap_or_default();
                let client = TranslationClient::new(self.platform.clone());
                let result = client
                    .translate_inner(
                        m.subject,
                        body.markdown.clone(),
                        body.content_hash.clone(),
                        configuration.clone(),
                        key,
                        observer,
                    )
                    .await?;
                self.save_translation(&message_id, &body, &configuration, &result)?;
                Ok(result)
            })
            .await
    }
}

fn retry_delay(failures: u32) -> u64 {
    (15 * (1u64 << failures.min(4))).min(300)
}
fn validate_recipients(draft: &Draft) -> Result<()> {
    let addresses = format!("{}, {}, {}", draft.to, draft.cc, draft.bcc);
    let mut count = 0;
    for address in addresses
        .split([',', ';', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        address
            .parse::<lettre::message::Mailbox>()
            .map_err(|_| fail("收件地址格式不正确"))?;
        count += 1;
    }
    if count == 0 {
        return Err(fail("请填写收件人"));
    }
    Ok(())
}
impl MailApplication {
    fn reconcile_workers(self: &Arc<Self>) -> Result<()> {
        let cancellation = self.cancellation.lock().unwrap();
        if cancellation.is_cancelled() {
            return Ok(());
        }
        self.launch_workers()
    }
    fn launch_workers(self: &Arc<Self>) -> Result<()> {
        for account in self
            .engine
            .accounts()?
            .into_iter()
            .filter(|a| a.enabled && !["demo", "local"].contains(&a.provider.as_str()))
        {
            let mut workers = self.workers.lock().unwrap();
            if workers.contains_key(&account.id) {
                continue;
            }
            let lane = self.lane(&account.id);
            let mut jobs = Vec::new();
            let weak = Arc::downgrade(&self);
            let id = account.id.clone();
            jobs.push(
                runtime()
                    .spawn(async move {
                        loop {
                            let Some(app) = weak.upgrade() else { break };
                            let _ = app.sync_inner(&id, None, false).await;
                            let seconds = if app.active.load(Ordering::Relaxed) {
                                30
                            } else {
                                120
                            };
                            drop(app);
                            tokio::time::sleep(Duration::from_secs(seconds)).await;
                        }
                    })
                    .abort_handle(),
            );
            let weak = Arc::downgrade(&self);
            let id = account.id.clone();
            jobs.push(
                runtime()
                    .spawn(async move {
                        let mut failures = 0u32;
                        loop {
                            let Some(app) = weak.upgrade() else { break };
                            let engine = app.engine.clone();
                            let credential = app.credential(&id).await;
                            drop(app);
                            let result = match credential {
                                Ok(token) => transport::idle(&engine, &id, &token).await,
                                Err(e) => Err(e),
                            };
                            let delay = match result {
                                Ok(changed) => {
                                    failures = 0;
                                    if changed {
                                        if let Some(app) = weak.upgrade() {
                                            let _ = app.sync_inner(&id, None, false).await;
                                        }
                                    }
                                    15
                                }
                                Err(e) => {
                                    failures += 1;
                                    if let Some(app) = weak.upgrade() {
                                        app.emit(
                                            ApplicationEventKind::Problem,
                                            &id,
                                            e.to_string(),
                                            true,
                                        );
                                    }
                                    retry_delay(failures)
                                }
                            };
                            tokio::time::sleep(Duration::from_secs(delay)).await;
                        }
                    })
                    .abort_handle(),
            );
            let weak = Arc::downgrade(&self);
            let id = account.id.clone();
            jobs.push(
                runtime()
                    .spawn(async move {
                        loop {
                            lane.preload.notified().await;
                            let Some(app) = weak.upgrade() else { break };
                            if let Err(error) = app.preload_inner(&id).await {
                                app.emit(
                                    ApplicationEventKind::Problem,
                                    &id,
                                    error.to_string(),
                                    true,
                                );
                            }
                        }
                    })
                    .abort_handle(),
            );
            workers.insert(account.id.clone(), jobs);
            self.lane(&account.id).preload.notify_one();
        }
        Ok(())
    }

    async fn command<T: Send + 'static>(
        self: Arc<Self>,
        work: impl std::future::Future<Output = Result<T>> + Send + 'static,
    ) -> Result<T> {
        let cancelled = self.cancellation.lock().unwrap().clone();
        run(async move {
            match futures_util::future::select(
                Box::pin(cancelled.cancelled_owned()),
                Box::pin(work),
            )
            .await
            {
                futures_util::future::Either::Left(_) => Err(fail("操作已取消")),
                futures_util::future::Either::Right((result, _)) => result,
            }
        })
        .await
    }
    fn lane(&self, id: &str) -> Arc<Lane> {
        self.lanes
            .lock()
            .unwrap()
            .entry(id.into())
            .or_default()
            .clone()
    }
    fn stop_account(&self, id: &str) {
        if let Some(handles) = self.workers.lock().unwrap().remove(id) {
            for handle in handles {
                handle.abort();
            }
        }
    }
    fn emit(&self, kind: ApplicationEventKind, account: &str, message: String, failed: bool) {
        self.observer.changed(ApplicationEvent {
            kind,
            account_id: account.into(),
            message,
            failed,
        });
    }
    fn data(&self, account: &str) {
        self.emit(
            ApplicationEventKind::DataChanged,
            account,
            String::new(),
            false,
        );
    }
    async fn credential(&self, id: &str) -> Result<String> {
        let lane = self.lane(id);
        let guard = lane.credential.clone().lock_owned().await;
        let a = self.engine.account(id)?;
        if ["demo", "local"].contains(&a.provider.as_str()) {
            return Ok(String::new());
        }
        for host in [&a.imap_host, &a.smtp_host] {
            let route = self.platform.proxy_for(host.clone())?;
            self.engine
                .configure_proxy(host.clone(), route.kind, route.host, route.port)?;
        }
        let value = secret_read(self.platform.clone(), format!("account:{id}"))
            .await?
            .ok_or_else(|| fail(format!("{} 尚未授权，请在账号设置中完成登录", a.name)))?;
        if a.auth_kind != "oauth" {
            return Ok(value);
        }
        let token: GoogleTokens =
            serde_json::from_str(&value).map_err(|_| fail("Google 授权格式无效，请重新登录"))?;
        if token.expires_at > now() as f64 + 120.0 {
            return Ok(token.access_token);
        }
        let client = self
            .engine
            .setting("google-client-id".into())?
            .unwrap_or_default();
        let secret = secret_read(self.platform.clone(), "google-client-secret".into())
            .await?
            .unwrap_or_default();
        let mut fields = vec![
            ("client_id", client),
            ("grant_type", "refresh_token".into()),
            ("refresh_token", token.refresh_token.clone()),
        ];
        if !secret.is_empty() {
            fields.push(("client_secret", secret));
        }
        let refreshed =
            auth::token_request(self.platform.as_ref(), fields, &token.refresh_token).await?;
        self.engine.account(id)?;
        let platform = self.platform.clone();
        let key = format!("account:{id}");
        let value = serde_json::to_string(&refreshed).map_err(fail)?;
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            platform.write_secret(key, value)
        })
        .await
        .map_err(|_| fail("凭证保存已中断"))??;
        Ok(refreshed.access_token)
    }
    async fn sync_inner(&self, id: &str, path: Option<String>, older: bool) -> Result<()> {
        if !self.engine.account(id)?.enabled {
            return Ok(());
        }
        let lane = self.lane(id);
        let _guard = if older {
            lane.sync.lock().await
        } else {
            match lane.sync.try_lock() {
                Ok(g) => g,
                Err(_) => return Ok(()),
            }
        };
        self.emit(ApplicationEventKind::SyncStarted, id, String::new(), false);
        struct Finished<'a>(&'a MailApplication, &'a str);
        impl Drop for Finished<'_> {
            fn drop(&mut self) {
                self.0.emit(
                    ApplicationEventKind::SyncFinished,
                    self.1,
                    String::new(),
                    false,
                );
            }
        }
        let _finished = Finished(self, id);
        let result = async {
            let token = self.credential(id).await?;
            if path.is_none() {
                transport::sync(&self.engine, id, &token, Some("INBOX".into()), false).await?;
                self.engine.prune_body_cache(id.into())?;
                self.data(id);
                lane.preload.notify_one();
            }
            let result = transport::sync(&self.engine, id, &token, path, older).await?;
            self.engine.prune_body_cache(id.into())?;
            self.data(id);
            lane.preload.notify_one();
            Ok::<_, MailError>(result)
        }
        .await;
        match result {
            Ok(r) => {
                self.emit(ApplicationEventKind::Problem, id, r.message, false);
                Ok(())
            }
            Err(e) => {
                self.emit(ApplicationEventKind::Problem, id, e.to_string(), true);
                Err(e)
            }
        }
    }
    async fn body_inner(&self, id: &str) -> Result<MailBody> {
        if let Some(body) = self.engine.cached_body(id.into())? {
            // A cache hit can repair previously missing list previews too.
            self.data(&self.engine.message(id)?.account_id);
            return Ok(body);
        }
        let m = self.engine.message(id)?;
        let body = if m.uid == 0 {
            let raw = std::fs::read(
                self.engine
                    .root
                    .join("imports")
                    .join(format!("{}.eml", m.id)),
            )
            .map_err(|_| fail("本地邮件原文不可用"))?;
            crate::mime::parse_body(&raw, id)?
        } else {
            let token = self.credential(&m.account_id).await?;
            transport::body(&self.engine, id, &token).await?
        };
        self.engine.store_body_internal(&body)?;
        self.data(&m.account_id);
        Ok(body)
    }
    async fn preload_inner(&self, id: &str) -> Result<()> {
        let token = self.credential(id).await?;
        for message in self.engine.recent_messages(id.into())? {
            if let Some(body) = self.engine.cached_body(message.id.clone())? {
                if body.html.is_empty() || body.html.contains(crate::html::RENDER_MARKER) {
                    continue;
                }
            }
            if !MailEngine::may_cache(&*self.engine.connection()?, &message)? {
                continue;
            }
            if let Ok(body) = transport::prefetch_body(&self.engine, &message.id, &token).await {
                self.engine.store_body_internal(&body)?;
                self.data(id);
            }
        }
        self.engine.prune_body_cache(id.into())?;
        self.data(id);
        Ok(())
    }
    async fn submit_inner(&self, id: &str) -> Result<Draft> {
        let d = self
            .engine
            .drafts()?
            .into_iter()
            .find(|d| d.id == id)
            .ok_or_else(|| fail("草稿不存在"))?;
        if !["queued", "failed"].contains(&d.status.as_str()) {
            return Err(fail("此邮件未排队或不能重复发送"));
        }
        let result = async {
            let token = self.credential(&d.account_id).await?;
            // Re-read after credential resolution so a simultaneous undo cannot send a draft.
            let current = self
                .engine
                .drafts()?
                .into_iter()
                .find(|d| d.id == id)
                .ok_or_else(|| fail("草稿不存在"))?;
            if !["queued", "failed"].contains(&current.status.as_str()) {
                return Err(fail("发送已撤销"));
            }
            transport::send_pending(&self.engine, id, &token).await
        }
        .await;
        match &result {
            Ok(d) => self.emit(
                ApplicationEventKind::DraftChanged,
                &d.account_id,
                if d.status == "accepted" && d.last_error.is_empty() {
                    "已提交到发件服务器".into()
                } else {
                    d.last_error.clone()
                },
                d.status != "accepted",
            ),
            Err(e) => {
                self.engine.fail_unsubmitted(id.into(), e.to_string())?;
                self.emit(
                    ApplicationEventKind::DraftChanged,
                    &d.account_id,
                    e.to_string(),
                    true,
                );
            }
        }
        result
    }
    fn save_translation(
        &self,
        id: &str,
        body: &MailBody,
        config: &TranslationConfiguration,
        result: &TranslationResult,
    ) -> Result<()> {
        validate_translation(body, result)?;
        let m = self.engine.message(id)?;
        // Old mail may be translated on demand, but must not grow persistent body/translation caches.
        if m.uid != 0 && !MailEngine::may_cache(&*self.engine.connection()?, &m)? {
            return Ok(());
        }
        let key = translation_cache_key(m.account_id, body.clone(), config.clone(), m.subject);
        self.engine
            .set_setting(key, serde_json::to_string(result).map_err(fail)?)
    }
}
