use crate::{
    events::{Event, Events, TranslationProgress},
    platform::DesktopPlatform,
};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::{prelude::*, *};
use lightmail_core::Result;
use lightmail_core::*;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

/// Everything the lists show, read together off the UI thread.
struct Lists {
    accounts: Vec<Account>,
    folders: Vec<Folder>,
    drafts: Vec<Draft>,
    messages: Vec<MessageSummary>,
    /// The cached body of the message that was selected when the read began.
    body: Option<MailBody>,
}
fn read_lists(engine: &MailEngine, query: MessageQuery, selected: Option<String>) -> Result<Lists> {
    Ok(Lists {
        accounts: engine.accounts()?,
        folders: engine.folders(String::new())?,
        drafts: engine.drafts()?,
        messages: engine.list_messages(query)?,
        body: match selected {
            Some(id) => engine.cached_body(id)?,
            None => None,
        },
    })
}
fn same_view(a: &MessageQuery, b: &MessageQuery) -> bool {
    (
        &a.account_id,
        &a.folder_id,
        &a.scope,
        &a.search,
        a.unread_only,
        a.offset,
    ) == (
        &b.account_id,
        &b.folder_id,
        &b.scope,
        &b.search,
        b.unread_only,
        b.offset,
    )
}

#[derive(Clone, Copy, PartialEq)]
pub enum Page {
    Mail,
    Accounts,
    Translation,
    Storage,
    Compose,
}
pub struct MailDesktop {
    pub focus: FocusHandle,
    pub data_root: PathBuf,
    pub focus_reading: bool,
    pub plain_reading: bool,
    pub adding_account: bool,
    pub extra_recipients: bool,
    pub probes: HashMap<String, Bounds<Pixels>>,
    pub engine: Arc<MailEngine>,
    pub service: Arc<MailApplication>,
    pub platform: Arc<DesktopPlatform>,
    pub events: Arc<Events>,
    pub remote_images: Arc<crate::images::RemoteImages>,
    pub accounts: Vec<Account>,
    pub folders: Vec<Folder>,
    pub messages: Vec<MessageSummary>,
    pub drafts: Vec<Draft>,
    pub account_id: String,
    pub folder_id: String,
    pub scope: String,
    pub title: String,
    pub unread_only: bool,
    pub expanded: HashSet<String>,
    pub selected: Option<MessageSummary>,
    pub body: Option<MailBody>,
    pub loading: bool,
    pub status: String,
    pub syncing: HashSet<String>,
    pub errors: HashMap<String, String>,
    pub page: Page,
    pub fields: HashMap<&'static str, Entity<InputState>>,
    /// Multi-line fields: Kit separates textareas from single-line inputs.
    pub areas: HashMap<&'static str, Entity<TextareaState>>,
    pub editing: Option<Account>,
    pub provider: String,
    pub oauth: bool,
    pub enabled: bool,
    pub append_sent: bool,
    pub removal: bool,
    pub configuration: Option<TranslationConfiguration>,
    pub configurations: Vec<TranslationConfiguration>,
    pub translation: Option<TranslationResult>,
    pub translating: bool,
    pub progress: String,
    pub mode: ExportMode,
    pub images: bool,
    pub draft: Option<Draft>,
    pub attachments: Vec<String>,
    pub demo: bool,
    pub busy: bool,
    pub reader: Option<crate::reader::Document>,
    pub reader_dirty: bool,
    pub reader_error: Option<String>,
    pub storage: Option<StorageInfo>,
    pub page_offset: u32,
    pub search_text: String,
    pub clear_secrets: bool,
    pub config_stream: bool,
    pub config_json: bool,
    pub acceptance: Option<PathBuf>,
    pub actions: Vec<String>,
    generation: u64,
    translation_generation: u64,
    body_task: Option<Task<()>>,
    blitz_task: Option<Task<()>>,
    blitz_generation: u64,
    translation_task: Option<Task<()>>,
    search_task: Option<Task<()>>,
    lists_task: Option<Task<()>>,
    lists_generation: u64,
    autosave_task: Option<Task<()>>,
    _events_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}
impl Drop for MailDesktop {
    fn drop(&mut self) {
        self.service.stop();
    }
}

impl MailDesktop {
    pub fn new(
        engine: Arc<MailEngine>,
        data_root: PathBuf,
        demo: bool,
        acceptance: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let platform = Arc::new(DesktopPlatform::default());
        let (events, receiver) = Events::new();
        let service = MailApplication::new(engine.clone(), platform.clone(), events.clone());
        let remote_images = {
            let events = events.clone();
            crate::images::RemoteImages::new(platform.clone(), move || events.image_ready())
        };
        let mut fields = HashMap::new();
        let mut areas = HashMap::new();
        for (key, prompt) in [
            ("search", "搜索已同步邮件"),
            ("name", "显示名称"),
            ("address", "完整邮箱地址"),
            ("password", "客户端授权码 / 应用专用密码"),
            ("imap", "IMAP 服务器"),
            ("imap_port", "993"),
            ("smtp", "SMTP 服务器"),
            ("smtp_port", "465"),
            ("client_id", "Google Desktop Client ID"),
            ("client_secret", "Client Secret（如提供）"),
            ("translation_name", "翻译配置名称"),
            ("base_url", "https://api.openai.com/v1"),
            ("model", "Model"),
            ("api_key", "API Key（保存在系统凭据管理器）"),
            ("language", "简体中文"),
            ("glossary", "每行一个术语"),
            ("to", "收件人"),
            ("cc", "抄送"),
            ("bcc", "密送"),
            ("subject", "主题"),
            ("draft_body", "开始写邮件…"),
        ] {
            if ["glossary", "draft_body"].contains(&key) {
                let area = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .placeholder(prompt)
                        .rows(if key == "draft_body" { 14 } else { 3 })
                });
                areas.insert(key, area);
                continue;
            }
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(prompt)
                    .masked(["password", "client_secret", "api_key"].contains(&key))
            });
            fields.insert(key, input);
        }
        let search = fields["search"].clone();
        let subscription = cx.subscribe(&search, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_task = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(180))
                        .await;
                    let _ = this.update(cx, |s, cx| {
                        s.reload_search(cx);
                        cx.notify();
                    });
                }));
            }
        });
        let mut subscriptions = vec![subscription];
        for key in ["to", "cc", "bcc", "subject"] {
            subscriptions.push(cx.subscribe(&fields[key], |s, _, event, cx| {
                if matches!(event, InputEvent::Change) && s.page == Page::Compose {
                    s.schedule_autosave(cx);
                }
            }));
        }
        subscriptions.push(cx.subscribe(&areas["draft_body"], |s, _, event, cx| {
            if matches!(event, InputEvent::Change) && s.page == Page::Compose {
                s.schedule_autosave(cx);
            }
        }));
        let event_source = events.clone();
        let event_task = cx.spawn(async move |this, cx| {
            while receiver.recv().await.is_ok() {
                let updates = event_source.drain();
                if this.update(cx, |s, cx| s.consume(updates, cx)).is_err() {
                    break;
                }
            }
        });
        let mut app = Self {
            focus,
            data_root,
            focus_reading: false,
            plain_reading: false,
            adding_account: false,
            extra_recipients: false,
            probes: HashMap::new(),
            engine,
            service,
            platform,
            events,
            remote_images,
            accounts: vec![],
            folders: vec![],
            messages: vec![],
            drafts: vec![],
            account_id: String::new(),
            folder_id: String::new(),
            scope: "inbox".into(),
            title: "全部收件箱".into(),
            unread_only: false,
            expanded: HashSet::new(),
            selected: None,
            body: None,
            loading: false,
            status: String::new(),
            syncing: HashSet::new(),
            errors: HashMap::new(),
            page: Page::Mail,
            fields,
            areas,
            editing: None,
            provider: "gmail".into(),
            oauth: true,
            enabled: true,
            append_sent: false,
            removal: false,
            configuration: None,
            configurations: vec![],
            translation: None,
            translating: false,
            progress: String::new(),
            mode: ExportMode::Original,
            images: false,
            draft: None,
            attachments: vec![],
            demo,
            busy: false,
            reader: None,
            reader_dirty: false,
            reader_error: None,
            storage: None,
            page_offset: 0,
            search_text: String::new(),
            clear_secrets: false,
            config_stream: true,
            config_json: false,
            acceptance,
            actions: vec![],
            generation: 0,
            translation_generation: 0,
            body_task: None,
            blitz_task: None,
            blitz_generation: 0,
            translation_task: None,
            search_task: None,
            lists_task: None,
            lists_generation: 0,
            autosave_task: None,
            _events_task: event_task,
            _subscriptions: subscriptions,
        };
        // The first read happens before the window shows, so a configured
        // mailbox never flashes the welcome page.
        let first = read_lists(&app.engine, app.query(0), None);
        app.apply_lists(first, true);
        app.load_translation_config(window, cx);
        if app
            .engine
            .setting("windows-automatic-receiving".into())
            .ok()
            .flatten()
            .as_deref()
            == Some("false")
        {
            let _ = app.service.set_automatic_receiving(false);
        }
        if !demo {
            if let Err(e) = app.service.clone().start() {
                app.status = e.to_string();
            }
        }
        app.record("startup");
        app
    }
    pub fn switch_demo(&mut self, demo: bool, cx: &mut Context<Self>) {
        self.persist_compose(cx);
        let result = (|| -> std::io::Result<()> {
            let mut command = std::process::Command::new(std::env::current_exe()?);
            command.arg("--data-dir").arg(&self.data_root);
            if demo {
                command.arg("--demo");
            }
            command.spawn()?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.service.stop();
                cx.quit();
            }
            Err(_) => {
                self.status = "无法打开示例窗口，请重试".into();
                cx.notify();
            }
        }
    }
    pub fn value(&self, key: &'static str, cx: &App) -> String {
        match self.areas.get(key) {
            Some(area) => area.read(cx).value().to_string(),
            None => self.fields[key].read(cx).value().to_string(),
        }
    }
    pub fn set(
        &self,
        key: &'static str,
        value: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let value: SharedString = value.into();
        match self.areas.get(key) {
            Some(area) => area.update(cx, |input, cx| input.set_value(value, window, cx)),
            None => self.fields[key].update(cx, |input, cx| input.set_value(value, window, cx)),
        }
    }
    pub fn record(&mut self, action: &str) {
        if let Some(path) = &self.acceptance {
            self.actions.push(action.into());
            let _ = std::fs::create_dir_all(path);
            let report = serde_json::json!({"actions":self.actions,"page":match self.page{Page::Mail=>"mail",Page::Accounts=>"accounts",Page::Translation=>"translation",Page::Compose=>"compose",Page::Storage=>"storage"},"editingAccount":self.editing.as_ref().map(|a|a.id.clone()),"accounts":self.accounts.len(),"messages":self.messages.len(),"bodyLoaded":self.body.is_some(),"loading":self.loading,"readerError":self.reader_error,"demo":self.demo});
            let _ = std::fs::write(path.join("ui-state.json"), report.to_string());
        }
    }
    fn query(&self, offset: u32) -> MessageQuery {
        MessageQuery {
            account_id: self.account_id.clone(),
            folder_id: self.folder_id.clone(),
            scope: self.scope.clone(),
            search: self.search_text.clone(),
            unread_only: self.unread_only,
            limit: 100,
            offset,
        }
    }
    /// Reads the lists off the UI thread, where a sync holding the engine's
    /// connection would stall input. Only the newest read lands, so a scope or
    /// page the user already left never replaces the current one, and a body
    /// read for an earlier selection is dropped.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.lists_generation += 1;
        let generation = self.lists_generation;
        let selection = self.generation;
        let engine = self.engine.clone();
        let query = self.query(self.page_offset);
        let selected = self.selected.as_ref().map(|m| m.id.clone());
        self.lists_task = Some(cx.spawn(async move |this, cx| {
            let lists = cx
                .background_spawn(async move { read_lists(&engine, query, selected) })
                .await;
            let _ = this.update(cx, |s, cx| {
                if s.lists_generation == generation {
                    s.apply_lists(lists, s.generation == selection);
                    cx.notify();
                }
            });
        }));
    }
    fn apply_lists(&mut self, lists: Result<Lists>, same_selection: bool) {
        let lists = match lists {
            Ok(lists) => lists,
            Err(e) => {
                self.status = e.to_string();
                return;
            }
        };
        self.accounts = lists.accounts;
        self.folders = lists.folders;
        self.drafts = lists.drafts;
        self.messages = lists.messages;
        let Some(selected) = &self.selected else {
            return;
        };
        if let Some(updated) = self.messages.iter().find(|m| m.id == selected.id) {
            self.selected = Some(updated.clone());
        }
        if let Some(body) = lists.body.filter(|_| same_selection) {
            if self
                .body
                .as_ref()
                .is_none_or(|old| old.content_hash != body.content_hash || old.html != body.html)
            {
                self.body = Some(body);
                self.reader_dirty = true;
            }
        }
    }
    /// Takes the search box text, then reloads.
    pub fn reload_search(&mut self, cx: &mut Context<Self>) {
        let search = self.value("search", cx);
        if search != self.search_text {
            self.page_offset = 0;
            self.search_text = search;
        }
        self.reload(cx);
    }
    fn consume(&mut self, updates: Vec<Event>, cx: &mut Context<Self>) {
        let mut reload = false;
        for event in updates {
            match event {
                Event::Core(e) => match e.kind {
                    ApplicationEventKind::SyncStarted => {
                        self.syncing.insert(e.account_id);
                    }
                    ApplicationEventKind::SyncFinished => {
                        self.syncing.remove(&e.account_id);
                    }
                    ApplicationEventKind::DataChanged => reload = true,
                    ApplicationEventKind::DraftChanged => {
                        reload = true;
                        if !e.message.is_empty() {
                            self.status = e.message;
                        }
                    }
                    ApplicationEventKind::Problem => {
                        if e.failed {
                            self.errors.insert(e.account_id, e.message.clone());
                            self.status = e.message;
                        } else {
                            self.errors.remove(&e.account_id);
                        }
                    }
                },
                Event::Translation(generation, done, total)
                    if generation == self.translation_generation && self.translating =>
                {
                    self.progress = format!("已翻译 {done} / {total} 段")
                }
                _ => {}
            }
        }
        if reload {
            self.reload_search(cx);
        }
        self.record("core-update");
        cx.notify();
    }
    pub fn change_scope(
        &mut self,
        title: String,
        account: String,
        folder: String,
        scope: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.persist_compose(cx);
        self.focus_reading = false;
        self.page_offset = 0;
        self.search_text.clear();
        self.cancel_translation();
        self.body_task = None;
        self.generation += 1;
        self.loading = false;
        self.selected = None;
        self.body = None;
        self.reader_dirty = true;
        self.title = title;
        self.account_id = account;
        self.folder_id = folder;
        self.scope = scope;
        self.page = Page::Mail;
        self.unread_only = false;
        self.set("search", "", window, cx);
        self.reload(cx);
        self.record("scope");
        cx.notify();
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let result = service.refresh().await;
            let _ = this.update(cx, |s, cx| {
                if let Err(e) = result {
                    s.status = e.to_string();
                }
                s.reload_search(cx);
                s.record("refresh");
                cx.notify();
            });
        })
        .detach();
    }
    #[cfg(windows)]
    pub fn set_automatic_receiving(&mut self, enabled: bool) -> Result<()> {
        self.engine
            .set_setting("windows-automatic-receiving".into(), enabled.to_string())?;
        if let Err(error) = self.service.set_automatic_receiving(enabled) {
            // Restore the setting if starting workers failed.
            let _ = self
                .engine
                .set_setting("windows-automatic-receiving".into(), (!enabled).to_string());
            let _ = self.service.set_automatic_receiving(!enabled);
            return Err(error);
        }
        if !enabled {
            self.syncing.clear();
        }
        self.record("automatic-receiving");
        Ok(())
    }
    pub fn select(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(message) = self.messages.iter().find(|m| m.id == id).cloned() else {
            return;
        };
        self.open(message, cx);
    }
    fn open(&mut self, message: MessageSummary, cx: &mut Context<Self>) {
        let id = message.id.clone();
        self.cancel_translation();
        self.selected = Some(message.clone());
        self.body = None;
        self.translation = None;
        self.mode = ExportMode::Original;
        self.images = false;
        self.remote_images.reset();
        self.loading = true;
        self.reader_dirty = true;
        self.reader_error = None;
        self.generation += 1;
        let generation = self.generation;
        let service = self.service.clone();
        self.body_task = Some(cx.spawn(async move |this, cx| {
            let result = service.clone().body(id.clone()).await;
            let mut mark = false;
            let _ = this.update(cx, |s, cx| {
                if s.generation != generation {
                    return;
                }
                s.loading = false;
                match result {
                    Ok(body) => {
                        if let Some(config) = &s.configuration {
                            s.translation = s
                                .service
                                .cached_translation(id.clone(), body.clone(), config.clone())
                                .ok()
                                .flatten();
                        }
                        s.body = Some(body);
                        s.reader_dirty = true;
                        mark = message.unread;
                    }
                    Err(e) => s.reader_error = Some(e.to_string()),
                }
                s.reload_search(cx);
                s.record("body-loaded");
                cx.notify();
            });
            if mark {
                let _ = service.mark(id, "seen".into(), true).await;
            }
        }));
        self.record("select");
        cx.notify();
    }
    pub fn mark_selected(&mut self, flag: &str, value: bool, cx: &mut Context<Self>) {
        let Some(m) = &self.selected else { return };
        let service = self.service.clone();
        let id = m.id.clone();
        let flag = flag.into();
        cx.spawn(async move |this, cx| {
            let result = service.mark(id, flag, value).await;
            let _ = this.update(cx, |s, cx| {
                if let Err(e) = result {
                    s.status = e.to_string();
                }
                s.reload(cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub fn move_selected(&mut self, role: &str, cx: &mut Context<Self>) {
        let Some(m) = &self.selected else { return };
        let service = self.service.clone();
        let id = m.id.clone();
        let role = role.into();
        cx.spawn(async move |this, cx| {
            let result = service.move_message(id, role).await;
            let _ = this.update(cx, |s, cx| {
                match result {
                    Ok(()) => {
                        s.selected = None;
                        s.body = None;
                        s.reader_dirty = true;
                        s.status = "邮件已移动".into();
                    }
                    Err(e) => s.status = e.to_string(),
                }
                s.reload(cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub fn older(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        let view = self.query(self.page_offset);
        let next = self.query(self.page_offset + 100);
        let engine = self.engine.clone();
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let rows = |engine: Arc<MailEngine>, next: MessageQuery| async move {
                engine.list_messages(next).map(|rows| !rows.is_empty())
            };
            let local = cx
                .background_spawn(rows(engine.clone(), next.clone()))
                .await
                .unwrap_or(false);
            let mut error = None;
            if !local {
                let _ = this.update(cx, |s, cx| {
                    s.status = "正在读取更早的摘要…".into();
                    cx.notify();
                });
                error = service
                    .load_older(
                        next.account_id.clone(),
                        next.folder_id.clone(),
                        next.scope.clone(),
                    )
                    .await
                    .err()
                    .map(|e| e.to_string());
            }
            let more = local
                || cx
                    .background_spawn(rows(engine, next))
                    .await
                    .unwrap_or(false);
            let _ = this.update(cx, |s, cx| {
                s.busy = false;
                if !local {
                    s.status = error.unwrap_or("已读取可用的摘要".into());
                }
                // The user may have moved on while this ran.
                if same_view(&view, &s.query(s.page_offset)) {
                    if more {
                        s.page_offset += 100;
                    }
                    s.reload(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }
    pub fn previous_page(&mut self, cx: &mut Context<Self>) {
        self.page_offset = self.page_offset.saturating_sub(100);
        self.reload(cx);
        cx.notify();
    }
    /// Saves the draft shortly after typing pauses, not on every keystroke.
    pub fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        self.autosave_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(800))
                .await;
            let _ = this.update(cx, |s, cx| s.save_compose_now(cx));
        }));
    }
    /// Saves immediately, for example before leaving the composer.
    pub fn persist_compose(&mut self, cx: &App) {
        self.autosave_task = None;
        self.save_compose_now(cx);
    }
    fn save_compose_now(&mut self, cx: &App) {
        if self.page == Page::Compose {
            if let Some(draft) = self.compose_value(cx) {
                match self.engine.save_draft(draft) {
                    Ok(d) => self.draft = Some(d),
                    Err(e) => self.status = e.to_string(),
                }
            }
        }
    }
    pub fn open_storage(&mut self, cx: &mut Context<Self>) {
        self.persist_compose(cx);
        self.page = Page::Storage;
        let engine = self.engine.clone();
        cx.spawn(async move |this, cx| {
            let info = cx
                .background_spawn(async move { engine.storage_info() })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.storage = info.ok();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub fn clear_cache(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.status = "正在清理正文缓存…".into();
        let engine = self.engine.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    engine.clear_body_cache()?;
                    engine.storage_info()
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.busy = false;
                match result {
                    Ok(info) => {
                        s.storage = Some(info);
                        s.status = "正文缓存已清理；摘要、本地导入邮件和草稿保留".into();
                    }
                    Err(e) => s.status = e.to_string(),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub fn import_mail(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self
            .accounts
            .iter()
            .find(|a| a.id == self.account_id)
            .or(self.accounts.first())
        else {
            self.status = "请先添加一个邮箱".into();
            cx.notify();
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("邮件", &["eml"])
            .pick_file()
        else {
            return;
        };
        match self
            .engine
            .import_eml(path.to_string_lossy().into(), account.id.clone())
        {
            Ok(message) => {
                self.account_id = message.account_id.clone();
                self.folder_id = message.folder_id.clone();
                self.scope = "inbox".into();
                self.title = "本地导入".into();
                self.page = Page::Mail;
                self.page_offset = 0;
                self.open(message, cx);
                self.reload(cx);
            }
            Err(e) => self.status = e.to_string(),
        }
        cx.notify();
    }
    pub fn retry_send(&mut self, id: String, cx: &mut Context<Self>) {
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let result = service.submit(id).await;
            let _ = this.update(cx, |s, cx| {
                s.status = result
                    .map(|_| "邮件已提交".into())
                    .unwrap_or_else(|e| e.to_string());
                s.reload(cx);
                cx.notify();
            });
        })
        .detach();
    }
    pub fn open_accounts(
        &mut self,
        id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.persist_compose(cx);
        self.page = Page::Accounts;
        self.adding_account = false;
        self.removal = false;
        self.editing = id.and_then(|id| self.accounts.iter().find(|a| a.id == id).cloned());
        let a = self.editing.clone();
        self.provider = a
            .as_ref()
            .map(|a| a.provider.clone())
            .unwrap_or("gmail".into());
        self.apply_preset(self.provider.clone(), window, cx);
        for (key, value) in [
            (
                "name",
                a.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
            ),
            (
                "address",
                a.as_ref().map(|a| a.address.clone()).unwrap_or_default(),
            ),
            ("password", String::new()),
        ] {
            self.set(key, value, window, cx);
        }
        if let Some(a) = a {
            self.oauth = a.auth_kind == "oauth";
            self.enabled = a.enabled;
            self.append_sent = a.sent_mode == "append";
            self.set("imap", a.imap_host, window, cx);
            self.set("smtp", a.smtp_host, window, cx);
            self.set("imap_port", a.imap_port.to_string(), window, cx);
            self.set("smtp_port", a.smtp_port.to_string(), window, cx);
        }
        self.set(
            "client_id",
            self.engine
                .setting("google-client-id".into())
                .ok()
                .flatten()
                .unwrap_or_default(),
            window,
            cx,
        );
        self.set("client_secret", "", window, cx);
        self.record("accounts");
        cx.notify();
    }
    pub fn apply_preset(&mut self, provider: String, window: &mut Window, cx: &mut Context<Self>) {
        let p = provider_preset(provider.clone());
        // A code typed for another provider must not be saved with this one.
        if provider != self.provider {
            self.set("password", "", window, cx);
        }
        self.provider = provider;
        self.oauth = p.auth_kind == "oauth";
        self.enabled = true;
        self.append_sent = p.sent_mode == "append";
        self.set("imap", p.imap_host, window, cx);
        self.set("smtp", p.smtp_host, window, cx);
        self.set("imap_port", p.imap_port.to_string(), window, cx);
        self.set("smtp_port", p.smtp_port.to_string(), window, cx);
        cx.notify();
    }
    pub fn save_account(&mut self, login: bool, cx: &mut Context<Self>) {
        let imap = self.value("imap_port", cx).parse();
        let smtp = self.value("smtp_port", cx).parse();
        let (Ok(imap_port), Ok(smtp_port)) = (imap, smtp) else {
            self.status = "端口格式不正确".into();
            return;
        };
        let account = Account {
            id: self
                .editing
                .as_ref()
                .map(|a| a.id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name: self.value("name", cx),
            address: self.value("address", cx).trim().into(),
            provider: self.provider.clone(),
            imap_host: self.value("imap", cx),
            imap_port,
            smtp_host: self.value("smtp", cx),
            smtp_port,
            auth_kind: if self.oauth { "oauth" } else { "password" }.into(),
            color: self
                .editing
                .as_ref()
                .map(|a| a.color.clone())
                .unwrap_or_else(|| provider_color(&self.provider).into()),
            enabled: self.enabled,
            sent_mode: if self.append_sent { "append" } else { "server" }.into(),
        };
        let password = if self.oauth {
            String::new()
        } else {
            self.value("password", cx)
        };
        let client_id = self.value("client_id", cx);
        let client_secret = self.value("client_secret", cx);
        if self.editing.is_none() && !self.oauth && password.is_empty() {
            self.status = "请填写客户端授权码".into();
            return;
        }
        let service = self.service.clone();
        let engine = self.engine.clone();
        let platform = self.platform.clone();
        self.busy = true;
        self.status = "正在保存账号…".into();
        cx.spawn(async move |this, cx| {
            let result = async {
                engine.set_setting("google-client-id".into(), client_id.clone())?;
                if !client_secret.is_empty() {
                    platform.write_secret("google-client-secret".into(), client_secret.clone())?;
                }
                if login && account.auth_kind == "oauth" {
                    let auth = GoogleLogin::new(account.clone(), client_id, platform.clone())?;
                    let url = auth.authorization_url();
                    cx.update(|cx| cx.open_url(&url));
                    let secret = platform
                        .read_secret("google-client-secret".into())?
                        .unwrap_or_default();
                    auth.finish(secret).await?;
                }
                service.save_account(account.clone(), password).await
            }
            .await;
            let _ = this.update(cx, |s, cx| {
                s.busy = false;
                match result {
                    Ok(()) => {
                        s.cancel_translation();
                        s.generation += 1;
                        s.body_task = None;
                        s.selected = None;
                        s.body = None;
                        s.reader_dirty = true;
                        cx.activate(true);
                        s.page = Page::Mail;
                        s.account_id = account.id;
                        s.folder_id.clear();
                        s.scope = "inbox".into();
                        s.title = account.name;
                        s.page_offset = 0;
                        s.search_text.clear();
                        s.clear_secrets = true;
                        s.status = "邮箱已连接，正在同步".into();
                        s.reload(cx);
                    }
                    Err(e) => s.status = e.to_string(),
                }
                s.record("account-save");
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub fn remove_account(&mut self, cx: &mut Context<Self>) {
        let Some(a) = self.editing.clone() else {
            return;
        };
        let service = self.service.clone();
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = service.remove_account(a.id).await;
            let _ = this.update(cx, |s, cx| {
                s.busy = false;
                match result {
                    Ok(()) => {
                        s.editing = None;
                        s.page = Page::Mail;
                        s.account_id.clear();
                        s.folder_id.clear();
                        s.scope = "inbox".into();
                        s.title = "全部收件箱".into();
                        s.selected = None;
                        s.body = None;
                        s.status = "已从此设备移除邮箱".into();
                        s.reload(cx);
                    }
                    Err(e) => s.status = e.to_string(),
                }
                s.removal = false;
                cx.notify();
            });
        })
        .detach();
    }
    pub fn new_draft(&mut self, mode: ComposeMode, window: &mut Window, cx: &mut Context<Self>) {
        let account = self
            .selected
            .as_ref()
            .and_then(|m| self.accounts.iter().find(|a| a.id == m.account_id))
            .or_else(|| self.accounts.iter().find(|a| a.id == self.account_id))
            .or_else(|| self.accounts.first());
        let Some(account) = account else {
            self.open_accounts(None, window, cx);
            return;
        };
        let draft = compose_draft(
            account.clone(),
            self.selected.clone(),
            self.body.clone(),
            mode,
            self.selected
                .as_ref()
                .map(|m| date(m.timestamp))
                .unwrap_or_default(),
        );
        self.edit_draft(draft, window, cx);
    }
    pub fn edit_draft(&mut self, draft: Draft, window: &mut Window, cx: &mut Context<Self>) {
        self.persist_compose(cx);
        self.page = Page::Mail;
        for (key, value) in [
            ("to", draft.to.clone()),
            ("cc", draft.cc.clone()),
            ("bcc", draft.bcc.clone()),
            ("subject", draft.subject.clone()),
            ("draft_body", draft.body.clone()),
        ] {
            self.set(key, value, window, cx);
        }
        self.attachments = draft.attachment_paths.clone();
        self.extra_recipients = !draft.cc.is_empty() || !draft.bcc.is_empty();
        self.draft = Some(draft);
        self.page = Page::Compose;
        self.record("compose");
        cx.notify();
    }
    fn compose_value(&self, cx: &App) -> Option<Draft> {
        let mut d = self.draft.clone()?;
        d.to = self.value("to", cx);
        d.cc = self.value("cc", cx);
        d.bcc = self.value("bcc", cx);
        d.subject = self.value("subject", cx);
        d.body = self.value("draft_body", cx);
        d.attachment_paths = self.attachments.clone();
        Some(d)
    }
    pub fn save_draft(&mut self, send: bool, cx: &mut Context<Self>) {
        let Some(draft) = self.compose_value(cx) else {
            return;
        };
        if !send {
            match self.engine.save_draft(draft) {
                Ok(d) => {
                    self.draft = Some(d);
                    self.status = "草稿已保存".into();
                    self.page = Page::Mail;
                    self.reload(cx);
                    self.record("draft-save");
                }
                Err(e) => self.status = e.to_string(),
            }
            cx.notify();
            return;
        }
        let service = self.service.clone();
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = service.queue(draft).await;
            let _ = this.update(cx, |s, cx| {
                s.busy = false;
                match result {
                    Ok(_) => {
                        s.page = Page::Mail;
                        s.scope = "outbox".into();
                        s.title = "待发送".into();
                        s.status = "5 秒后发送，可在待发送中撤销".into();
                        s.reload(cx);
                    }
                    Err(e) => s.status = e.to_string(),
                }
                s.record("queue");
                cx.notify();
            });
        })
        .detach();
    }
    /// Records the user's check of an interrupted submission; nothing is sent.
    pub fn resolve_delivery(&mut self, id: String, delivered: bool, cx: &mut Context<Self>) {
        match self.service.resolve_delivery(id, delivered) {
            Ok(_) => {
                self.status = if delivered {
                    "已记录为送达".into()
                } else {
                    "已退回草稿，可核对后重新发送".into()
                };
                self.reload(cx);
            }
            Err(e) => self.status = e.to_string(),
        }
        cx.notify();
    }
    pub fn delete_draft(&mut self, id: String, cx: &mut Context<Self>) {
        match self.engine.delete_draft(id) {
            Ok(()) => {
                self.status = "草稿已删除".into();
                self.reload(cx);
            }
            Err(e) => self.status = e.to_string(),
        }
        cx.notify();
    }
    pub fn cancel_queue(&mut self, id: String, cx: &mut Context<Self>) {
        match self.service.cancel_queued(id) {
            Ok(()) => {
                self.status = "已撤销发送，保留在草稿".into();
                self.reload(cx);
            }
            Err(e) => self.status = e.to_string(),
        }
        cx.notify();
    }
    pub fn load_translation_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.configurations = self
            .service
            .translation_configurations()
            .unwrap_or_default();
        let selected = self
            .engine
            .setting("translation-default".into())
            .ok()
            .flatten()
            .unwrap_or_default();
        self.configuration = self
            .configurations
            .iter()
            .find(|c| c.id == selected && c.engine == "llm")
            .or_else(|| self.configurations.iter().find(|c| c.engine == "llm"))
            .cloned();
        self.fill_translation_fields(window, cx);
    }
    pub fn select_translation_config(
        &mut self,
        id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.configuration = id.and_then(|id| {
            self.configurations
                .iter()
                .find(|c| c.id == id && c.engine == "llm")
                .cloned()
        });
        self.fill_translation_fields(window, cx);
        cx.notify();
    }
    fn fill_translation_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let c = self
            .configuration
            .clone()
            .unwrap_or(TranslationConfiguration {
                id: uuid::Uuid::new_v4().to_string(),
                name: "我的翻译服务".into(),
                base_url: "https://api.openai.com/v1".into(),
                model: String::new(),
                target_language: "简体中文".into(),
                stream: true,
                output_format: "prompt".into(),
                input_characters: 12000,
                glossary: String::new(),
                engine: "llm".into(),
            });
        self.config_stream = c.stream;
        self.config_json = c.output_format == "json";
        for (key, value) in [
            ("translation_name", c.name),
            ("base_url", c.base_url),
            ("model", c.model),
            ("language", c.target_language),
            ("glossary", c.glossary),
            ("api_key", String::new()),
        ] {
            self.set(key, value, window, cx);
        }
    }
    fn translation_value(&self, cx: &App) -> TranslationConfiguration {
        TranslationConfiguration {
            id: self
                .configuration
                .as_ref()
                .map(|c| c.id.clone())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name: self.value("translation_name", cx),
            base_url: self.value("base_url", cx),
            model: self.value("model", cx),
            target_language: self.value("language", cx),
            stream: self.config_stream,
            output_format: if self.config_json { "json" } else { "prompt" }.into(),
            input_characters: self
                .configuration
                .as_ref()
                .map(|c| c.input_characters)
                .unwrap_or(12000),
            glossary: self.value("glossary", cx),
            engine: "llm".into(),
        }
    }
    pub fn save_translation(&mut self, test: bool, cx: &mut Context<Self>) {
        let c = self.translation_value(cx);
        if let Err(e) = validate_translation_configuration(c.clone()) {
            self.status = e.to_string();
            cx.notify();
            return;
        }
        let key = self.value("api_key", cx);
        let platform = self.platform.clone();
        let service = self.service.clone();
        let mut configs = self.configurations.clone();
        configs.retain(|old| old.id != c.id);
        configs.push(c.clone());
        self.busy = true;
        cx.spawn(async move |this, cx| {
            let result = async {
                if !key.is_empty() {
                    platform.write_secret(format!("translation:{}", c.id), key.clone())?;
                }
                service.save_translation_configurations(configs.clone(), c.id.clone())?;
                if test {
                    let key = platform
                        .read_secret(format!("translation:{}", c.id))?
                        .unwrap_or_default();
                    TranslationClient::new(platform)
                        .test(c.clone(), key)
                        .await?;
                }
                Ok::<_, MailError>(())
            }
            .await;
            let _ = this.update(cx, |s, cx| {
                s.busy = false;
                match result {
                    Ok(()) => {
                        s.cancel_translation();
                        s.translation = None;
                        s.mode = ExportMode::Original;
                        s.reader_dirty = true;
                        s.clear_secrets = true;
                        s.configuration = Some(c);
                        s.configurations = configs;
                        s.status = if test {
                            "连接测试通过（仅使用合成文本）"
                        } else {
                            "翻译设置已保存"
                        }
                        .into();
                    }
                    Err(e) => s.status = e.to_string(),
                }
                s.record("translation-settings");
                cx.notify();
            });
        })
        .detach();
    }
    pub fn cancel_translation(&mut self) {
        self.translation_task = None;
        self.translation_generation += 1;
        self.translating = false;
    }
    pub fn translate(&mut self, force: bool, cx: &mut Context<Self>) {
        let (Some(message), Some(body)) = (self.selected.clone(), self.body.clone()) else {
            return;
        };
        let Some(config) = self.configuration.clone() else {
            self.page = Page::Translation;
            self.status = "请先配置翻译服务".into();
            cx.notify();
            return;
        };
        self.cancel_translation();
        let generation = self.translation_generation;
        self.translating = true;
        self.progress = "正在理解整封邮件…".into();
        let service = self.service.clone();
        let observer = Arc::new(TranslationProgress {
            events: self.events.clone(),
            generation,
        });
        self.translation_task = Some(cx.spawn(async move |this, cx| {
            let result = service
                .translate(message.id, body, config, force, observer)
                .await;
            let _ = this.update(cx, |s, cx| {
                if generation != s.translation_generation {
                    return;
                }
                s.translating = false;
                match result {
                    Ok(result) => {
                        s.translation = Some(result);
                        s.mode = ExportMode::Bilingual;
                        s.reader_dirty = true;
                        s.progress = "全文已翻译".into();
                    }
                    Err(e) => {
                        s.progress = "翻译未完成".into();
                        s.status = e.to_string()
                    }
                }
                s.record("translate");
                cx.notify();
            });
        }));
        cx.notify();
    }
    pub fn copy(&mut self, cx: &mut Context<Self>) {
        if let (Some(message), Some(body)) = (self.selected.clone(), self.body.clone()) {
            match export_markdown(
                message.clone(),
                body,
                self.translation.clone(),
                self.mode,
                date(message.timestamp),
            ) {
                Ok(text) => {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                    self.status = "已复制 Markdown".into();
                    self.record("copy");
                }
                Err(e) => self.status = e.to_string(),
            }
        }
        cx.notify();
    }
    pub fn download(&mut self, attachment: AttachmentInfo, cx: &mut Context<Self>) {
        let Some(message) = self.selected.clone() else {
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .set_file_name(&attachment.filename)
            .save_file()
        else {
            return;
        };
        let service = self.service.clone();
        cx.spawn(async move |this, cx| {
            let result = service
                .download(
                    message.id,
                    attachment.part_id,
                    path.to_string_lossy().into(),
                )
                .await;
            let _ = this.update(cx, |s, cx| {
                s.status = result
                    .map(|_| "附件已保存".into())
                    .unwrap_or_else(|e| e.to_string());
                cx.notify();
            });
        })
        .detach();
    }
    /// Image resolver for the current message. Remote images stay placeholders
    /// until the user allows them for this message.
    pub fn image_policy(&self) -> crate::reader::Images {
        if self.images {
            self.remote_images.resolver()
        } else {
            crate::reader::blocked_images()
        }
    }
    /// Prepares reader content when the body, translation or mode changed. The
    /// core validates translations and bounds Markdown work; rendering then
    /// only clones shared strings.
    pub fn update_reader(&mut self, cx: &mut Context<Self>) {
        if !self.reader_dirty {
            return;
        }
        self.reader_dirty = false;
        self.blitz_generation = self.blitz_generation.wrapping_add(1);
        self.blitz_task = None;
        self.reader = None;
        self.loading = false;
        let Some(body) = &self.body else {
            return;
        };
        match reader_content(
            body,
            self.translation.as_ref(),
            self.mode,
            !self.plain_reading,
        ) {
            Ok(lightmail_core::ReaderContent::Html(html)) if !self.plain_reading => {
                let generation = self.blitz_generation;
                let allow_images = self.images;
                let platform = self.platform.clone();
                self.loading = true;
                self.blitz_task = Some(cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move {
                            crate::blitz_reader::render(&html, 720, allow_images, platform)
                        })
                        .await;
                    let _ = this.update(cx, |s, cx| {
                        if s.blitz_generation != generation {
                            return;
                        }
                        s.loading = false;
                        match result {
                            Ok(rendered) => {
                                s.reader = Some(crate::reader::Document::Blitz(Arc::new(rendered)));
                                s.reader_error = None;
                            }
                            Err(error) => {
                                s.status = format!("原生 HTML 阅读器回退：{error}");
                                s.reader_error = None;
                            }
                        }
                        s.record("reader-render");
                        cx.notify();
                    });
                }));
                return;
            }
            Ok(content) => {
                self.reader = Some(content.into());
                self.reader_error = None;
            }
            Err(e) => self.reader_error = Some(e.to_string()),
        }
        self.record("reader-render");
    }
}
pub fn date(timestamp: i64) -> String {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|d| {
            d.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    // No glob import: with Kit's test support it would shadow `#[test]`.
    use super::MailDesktop;
    use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
    use lightmail_core::MailEngine;

    fn open(cx: &mut TestAppContext) -> (tempfile::TempDir, AnyWindowHandle, Entity<MailDesktop>) {
        let root = tempfile::tempdir().unwrap();
        let engine =
            MailEngine::new(root.path().join("Preview").to_string_lossy().into_owned()).unwrap();
        engine.seed_demo().unwrap();
        cx.update(gpui_kit::init);
        let data = root.path().to_path_buf();
        let (window, view) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| MailDesktop::new(engine, data, true, None, window, cx))
            })
            .unwrap()
        });
        (root, window, view)
    }

    #[gpui_kit::test]
    fn a_configured_mailbox_is_listed_before_the_first_frame(cx: &mut TestAppContext) {
        let (_root, _window, view) = open(cx);
        // Nothing has run yet: the welcome page must not flash for existing accounts.
        view.read_with(cx, |s, _| {
            assert_eq!((s.accounts.len(), s.messages.len()), (4, 7));
        });
    }

    #[gpui_kit::test]
    fn original_mail_uses_html_and_plain_mail_still_has_a_fallback(cx: &mut TestAppContext) {
        let (_root, window, view) = open(cx);
        let _ = cx.update_window(window, |_, _, cx| {
            view.update(cx, |s, cx| {
                s.body = Some(lightmail_core::MailBody {
                    message_id: "synthetic-html".into(),
                    text: "plain alternative".into(),
                    markdown: "markdown alternative".into(),
                    html: "<h2>Invoice</h2><p><strong>Total:</strong> 20.00</p>".into(),
                    attachments: vec![],
                    content_hash: "synthetic".into(),
                });
                s.reader_dirty = true;
                s.update_reader(cx);
                s.body.as_mut().unwrap().html.clear();
                s.reader_dirty = true;
                s.update_reader(cx);
                assert!(matches!(
                    s.reader,
                    Some(crate::reader::Document::Markdown(_))
                ));
            });
        });
    }

    // Several scheduler seeds, so both completion orders of the two reads occur.
    #[gpui_kit::test(iterations = 16)]
    fn a_read_for_a_view_the_user_left_never_lands(cx: &mut TestAppContext) {
        let (_root, window, view) = open(cx);
        cx.update_window(window, |_, window, cx| {
            view.update(cx, |s, cx| {
                // The demo's sent folders are empty; leave before that read lands.
                s.change_scope(
                    "已发送".into(),
                    "".into(),
                    "".into(),
                    "sent".into(),
                    window,
                    cx,
                );
                s.change_scope(
                    "收件箱".into(),
                    "".into(),
                    "".into(),
                    "inbox".into(),
                    window,
                    cx,
                );
            })
        })
        .unwrap();
        cx.run_until_parked();
        view.read_with(cx, |s, _| {
            assert_eq!((s.scope.as_str(), s.messages.len()), ("inbox", 7));
        });
    }

    #[gpui_kit::test]
    fn a_body_read_for_an_earlier_selection_is_dropped(cx: &mut TestAppContext) {
        let (_root, window, view) = open(cx);
        cx.update_window(window, |_, _, cx| {
            view.update(cx, |s, cx| {
                let (first, second) = (s.messages[0].clone(), s.messages[1].clone());
                // Every demo body is cached, so this read carries the first body.
                s.selected = Some(first);
                s.reload(cx);
                // Selecting another message before it lands makes it stale.
                s.selected = Some(second);
                s.generation += 1;
                s.body = None;
            })
        })
        .unwrap();
        cx.run_until_parked();
        view.read_with(cx, |s, _| assert!(s.body.is_none(), "stale body shown"));
        cx.update_window(window, |_, _, cx| view.update(cx, |s, cx| s.reload(cx)))
            .unwrap();
        cx.run_until_parked();
        view.read_with(cx, |s, _| {
            let selected = s.selected.as_ref().map(|m| m.id.as_str());
            assert_eq!(s.body.as_ref().map(|b| b.message_id.as_str()), selected);
        });
    }
}
