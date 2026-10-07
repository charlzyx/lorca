use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lorca_agent::{
    BeforeToolCallContext, BeforeToolCallResult, Tool, ToolError, ToolResult, ToolUpdateFn,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::model::{Author, Body, Bot, Message};
use crate::plugins::mcp::Server;

#[path = "profile.rs"]
mod profile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    Stopped,
    Bot,
    TakingOver,
    Human,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub bot_id: String,
    pub runner_id: String,
    pub account: String,
    pub profile: String,
    pub state: Control,
    pub selected: bool,
    pub revision: u64,
    pub created_at: f64,
}

struct Runtime {
    meta: Mutex<Session>,
    /// Held until the MCP call finishes, even when the user is waiting for takeover.
    input: Arc<AsyncMutex<()>>,
    server: Mutex<Option<Arc<Server>>>,
    opening: Mutex<CancellationToken>,
    stopping: AtomicUsize,
}

struct Stopping<'a>(&'a AtomicUsize);
impl Drop for Stopping<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(server) = self.server.lock().unwrap().take() {
            server.stop();
        }
    }
}

#[derive(Default)]
pub struct Sessions {
    account: Mutex<Option<String>>,
    items: Mutex<BTreeMap<String, Arc<Runtime>>>,
    persistence: Mutex<()>,
    changed: tokio::sync::Notify,
}

/// Holding this value owns the input gate. A new takeover blocks later callers
/// before waiting for this call to drain, preserving the page and model context.
pub struct Input {
    _guard: OwnedMutexGuard<()>,
    runtime: Arc<Runtime>,
    pub server: Arc<Server>,
    pub id: String,
}

impl Input {
    /// A cancelled/timed-out MCP call has no completion acknowledgement. Close
    /// its process so it cannot deliver late input after the gate is released.
    pub fn interrupted(&self, app: &Arc<App>) {
        self.server.stop();
        self.runtime.server.lock().unwrap().take();
        let mut meta = self.runtime.meta.lock().unwrap();
        meta.state = Control::Stopped;
        meta.revision += 1;
        drop(meta);
        let _ = app.browser_sessions.save(app);
    }
}

impl Sessions {
    pub fn local_bot(&self, app: &App, id: &str) -> Result<Bot, String> {
        let bot = app.bot(id).ok_or("Unknown bot")?;
        if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
            return Err("Browser sessions run only on the bot's assigned owned Runner.".into());
        }
        if !app
            .device(&bot.runner_id)
            .is_some_and(|device| device.is_runner())
        {
            return Err("Browser sessions require an owned Runner.".into());
        }
        Ok(bot)
    }

    fn load(&self, app: &Arc<App>) -> Result<(), String> {
        let account_key = app
            .this_device_id()
            .ok_or("Pair or create an identity first.")?;
        let dek = app.dek().ok_or("The account key is unavailable.")?;
        let mut loaded = self.account.lock().unwrap();
        if loaded.as_deref() == Some(&account_key) {
            return Ok(());
        }
        let dir = app.config.home.join("browser");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        crate::config::set_private(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("sessions.enc");
        let saved: Vec<Session> = if path.is_file() {
            crate::crypto::decrypt_json(
                &dek,
                "browser-sessions",
                &std::fs::read(path).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        let mut items = self.items.lock().unwrap();
        items.clear();
        for mut meta in saved {
            if !valid_id(&meta.id) {
                return Err("Invalid browser session id in the encrypted store.".into());
            }
            // A service restart never grants bot input or reopens a screen.
            meta.state = Control::Stopped;
            meta.revision += 1;
            items.insert(
                meta.id.clone(),
                Arc::new(Runtime {
                    meta: Mutex::new(meta),
                    input: Arc::new(AsyncMutex::new(())),
                    server: Mutex::new(None),
                    opening: Mutex::new(CancellationToken::new()),
                    stopping: AtomicUsize::new(0),
                }),
            );
        }
        *loaded = Some(account_key);
        Ok(())
    }

    fn save(&self, app: &Arc<App>) -> Result<(), String> {
        self.changed.notify_waiters();
        let _guard = self.persistence.lock().unwrap();
        let dek = app.dek().ok_or("The account key is unavailable.")?;
        let items: Vec<Session> = self
            .items
            .lock()
            .unwrap()
            .values()
            .map(|runtime| runtime.meta.lock().unwrap().clone())
            .collect();
        let bytes = crate::crypto::encrypt_json(&dek, "browser-sessions", &items)
            .map_err(|e| e.to_string())?;
        let path = app.config.home.join("browser/sessions.enc");
        let pending = path.with_extension("enc.pending");
        crate::config::write_private(&pending, &bytes).map_err(|e| e.to_string())?;
        std::fs::rename(pending, path).map_err(|e| e.to_string())
    }

    pub fn list(&self, app: &Arc<App>, bot_id: &str) -> Result<Vec<Session>, String> {
        self.local_bot(app, bot_id)?;
        self.load(app)?;
        Ok(self
            .items
            .lock()
            .unwrap()
            .values()
            .map(|item| item.meta.lock().unwrap().clone())
            .filter(|s| s.bot_id == bot_id)
            .collect())
    }

    fn owned(&self, app: &Arc<App>, bot_id: &str, id: &str) -> Result<Arc<Runtime>, String> {
        let bot = self.local_bot(app, bot_id)?;
        self.load(app)?;
        let runtime = self
            .items
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or("Unknown browser session")?;
        let meta = runtime.meta.lock().unwrap();
        if meta.bot_id != bot.id || meta.runner_id != bot.runner_id {
            return Err("This browser session belongs to another bot or Runner.".into());
        }
        drop(meta);
        Ok(runtime)
    }

    pub fn create(
        &self,
        app: &Arc<App>,
        bot_id: &str,
        account: &str,
        name: &str,
    ) -> Result<Session, String> {
        let bot = self.local_bot(app, bot_id)?;
        self.load(app)?;
        if app.plugins.lock().unwrap().get(super::PLUGIN_ID).is_none() {
            return Err("Install the Browser plugin on this Runner first.".into());
        }
        let account = label(account)?;
        let name = label(name)?;
        let meta = Session {
            id: format!("browser-{}", uuid::Uuid::new_v4()),
            bot_id: bot.id.clone(),
            runner_id: bot.runner_id,
            account,
            profile: name,
            state: Control::Stopped,
            selected: true,
            revision: 1,
            created_at: crate::config::now_secs(),
        };
        {
            let mut items = self.items.lock().unwrap();
            for item in items.values() {
                let mut old = item.meta.lock().unwrap();
                if old.bot_id == bot.id && old.selected {
                    old.selected = false;
                    old.revision += 1;
                }
            }
            items.insert(
                meta.id.clone(),
                Arc::new(Runtime {
                    meta: Mutex::new(meta.clone()),
                    input: Arc::new(AsyncMutex::new(())),
                    server: Mutex::new(None),
                    opening: Mutex::new(CancellationToken::new()),
                    stopping: AtomicUsize::new(0),
                }),
            );
        }
        self.save(app)?;
        Ok(meta)
    }

    fn select(&self, runtime: &Arc<Runtime>) {
        let selected = runtime.meta.lock().unwrap().clone();
        for item in self.items.lock().unwrap().values() {
            let mut meta = item.meta.lock().unwrap();
            if meta.bot_id == selected.bot_id && meta.selected != (meta.id == selected.id) {
                meta.selected = meta.id == selected.id;
                meta.revision += 1;
            }
        }
    }

    pub fn bot_ready(&self, app: &Arc<App>, bot_id: &str) -> Result<(), String> {
        let sessions = self.list(app, bot_id)?;
        if sessions
            .iter()
            .any(|s| matches!(s.state, Control::TakingOver | Control::Human))
        {
            return Err("The user controls this bot's browser. Wait for an explicit Return to Bot; do not use another session or input route.".into());
        }
        let meta = sessions
            .into_iter()
            .find(|s| s.selected)
            .ok_or("Create and open a browser session with browser_session first.")?;
        if meta.state != Control::Bot {
            return Err(
                "The selected browser session is stopped. Open it with browser_session first."
                    .into(),
            );
        }
        Ok(())
    }

    /// Park the current call/turn in place while the user owns input. Stop and
    /// chat cancellation wake it too. This also fences shell/codemode calls at
    /// the execution boundary, so another input route cannot compete.
    pub async fn wait_if_taken_over(
        &self,
        app: &Arc<App>,
        bot_id: &str,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        loop {
            let changed = self.changed.notified();
            let sessions = self.list(app, bot_id)?;
            if !sessions
                .iter()
                .any(|s| matches!(s.state, Control::Human | Control::TakingOver))
            {
                return Ok(());
            }
            tokio::select! {
                _ = changed => {},
                _ = cancel.cancelled() => return Err("Stopped".into()),
            }
        }
    }

    pub async fn wait_for_bot(
        &self,
        app: &Arc<App>,
        bot_id: &str,
        cancel: &CancellationToken,
    ) -> Result<(), String> {
        self.wait_if_taken_over(app, bot_id, cancel).await?;
        self.bot_ready(app, bot_id)
    }

    pub async fn input(
        &self,
        app: &Arc<App>,
        bot_id: &str,
        cancel: &CancellationToken,
    ) -> Result<Input, String> {
        self.wait_for_bot(app, bot_id, cancel).await?;
        let meta = self
            .list(app, bot_id)?
            .into_iter()
            .find(|s| s.selected)
            .unwrap();
        let runtime = self.owned(app, bot_id, &meta.id)?;
        loop {
            self.wait_for_bot(app, bot_id, cancel).await?;
            let guard = tokio::select! {
                guard = runtime.input.clone().lock_owned() => guard,
                _ = cancel.cancelled() => return Err("Stopped".into()),
            };
            self.owned(app, bot_id, &meta.id)?;
            let latest = runtime.meta.lock().unwrap().clone();
            if matches!(latest.state, Control::Human | Control::TakingOver) {
                drop(guard);
                continue;
            }
            if !latest.selected || latest.state != Control::Bot {
                return Err("Browser selection or control changed before this call ran.".into());
            }
            let server = runtime
                .server
                .lock()
                .unwrap()
                .clone()
                .filter(|s| !s.is_closed())
                .ok_or("The browser closed. Explicitly open its session again.")?;
            if !server.is_current(app) {
                return Err("The Browser plugin changed or was removed. Stop and reopen its session before using it.".into());
            }
            return Ok(Input {
                _guard: guard,
                runtime,
                server,
                id: meta.id,
            });
        }
    }

    pub async fn open(
        &self,
        app: &Arc<App>,
        bot_id: &str,
        id: &str,
        human: bool,
    ) -> Result<Session, String> {
        let runtime = self.owned(app, bot_id, id)?;
        let requested_revision = runtime.meta.lock().unwrap().revision;
        let _guard = runtime.input.lock().await;
        self.owned(app, bot_id, id)?;
        let opening = CancellationToken::new();
        let opening_revision = {
            let meta = runtime.meta.lock().unwrap();
            if meta.revision != requested_revision || runtime.stopping.load(Ordering::Acquire) != 0
            {
                return Err(
                    "Session control changed while Open waited. Refresh and open it again.".into(),
                );
            }
            *runtime.opening.lock().unwrap() = opening.clone();
            meta.revision
        };
        if app.plugins.lock().unwrap().get(super::PLUGIN_ID).is_none() {
            return Err("Install the Browser plugin on this Runner first.".into());
        }
        if !human
            && self
                .list(app, bot_id)?
                .iter()
                .any(|s| matches!(s.state, Control::Human | Control::TakingOver))
        {
            return Err(
                "The user has browser control. Only the user can return control to the bot.".into(),
            );
        }
        let existing = runtime.server.lock().unwrap().clone();
        let existing = match existing {
            Some(server) if !server.is_current(app) => {
                server.stop();
                server.wait_stopped().await?;
                runtime.server.lock().unwrap().take();
                None
            }
            other => other.filter(|s| !s.is_closed()),
        };
        let server = match existing {
            Some(server) => server,
            None => {
                let dek = app.dek().ok_or("The account key is unavailable.")?;
                let dir = profile::directory(&app.config.home.join("browser/profiles"), id);
                let root = dir.parent().unwrap();
                std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
                crate::config::set_private(root).map_err(|e| e.to_string())?;
                let owned_id = id.to_string();
                tokio::task::spawn_blocking(move || profile::restore(&dir, &dek, &owned_id))
                    .await
                    .map_err(|e| e.to_string())??;
                let server = tokio::select! {
                    server = crate::plugins::mcp::visible_browser(app, id) => server?,
                    _ = opening.cancelled() => return Err("The session was stopped while it opened.".into()),
                };
                *runtime.server.lock().unwrap() = Some(server.clone());
                server
            }
        };
        let opened = tokio::select! {
            opened = server.open_visible() => opened,
            _ = opening.cancelled() => { server.stop(); return Err("The session was stopped while it opened.".into()); },
        };
        if let Err(error) = opened {
            server.stop();
            runtime.server.lock().unwrap().take();
            return Err(error);
        }
        // Stop can run while a browser opens. It revokes input immediately.
        if server.is_closed() {
            return Err("The session was stopped while it opened.".into());
        }
        self.owned(app, bot_id, id)?;
        {
            let mut meta = runtime.meta.lock().unwrap();
            if meta.revision != opening_revision {
                server.stop();
                return Err("Session control changed while it opened. Refresh the session.".into());
            }
            meta.state = if human { Control::Human } else { Control::Bot };
            meta.revision += 1;
        }
        self.select(&runtime);
        if let Err(error) = self.save(app) {
            server.stop();
            runtime.meta.lock().unwrap().state = Control::Stopped;
            return Err(error);
        }
        let meta = runtime.meta.lock().unwrap().clone();
        Ok(meta)
    }

    pub async fn takeover(
        &self,
        app: &Arc<App>,
        bot_id: &str,
        id: &str,
    ) -> Result<Session, String> {
        let runtime = self.owned(app, bot_id, id)?;
        {
            let mut meta = runtime.meta.lock().unwrap();
            if meta.state == Control::Stopped {
                return Err("Open the session on its Runner before taking over.".into());
            }
            if meta.state == Control::Human {
                return Ok(meta.clone());
            }
            meta.state = Control::TakingOver;
            meta.revision += 1;
        }
        self.save(app)?;
        // No bot call passes its second state check now. The active call keeps
        // this gate until completion; we never claim human control prematurely.
        let _guard = runtime.input.lock().await;
        self.owned(app, bot_id, id)?;
        let meta = {
            let mut meta = runtime.meta.lock().unwrap();
            if meta.state != Control::TakingOver {
                return Err("The session stopped or control changed while takeover waited.".into());
            }
            meta.state = Control::Human;
            meta.revision += 1;
            meta.clone()
        };
        self.save(app)?;
        Ok(meta)
    }

    pub async fn resume(
        &self,
        app: &Arc<App>,
        bot_id: &str,
        id: &str,
        revision: u64,
    ) -> Result<Session, String> {
        let runtime = self.owned(app, bot_id, id)?;
        let _guard = runtime.input.lock().await;
        self.owned(app, bot_id, id)?;
        {
            let mut meta = runtime.meta.lock().unwrap();
            if meta.revision != revision {
                return Err("Browser control changed. Refresh before returning control.".into());
            }
            if meta.state != Control::Human {
                return Err("Only a session under human control can return to the bot.".into());
            }
            if !runtime
                .server
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|s| !s.is_closed())
            {
                return Err("The browser closed. Open it again on its Runner.".into());
            }
            meta.state = Control::Bot;
            meta.revision += 1;
        }
        self.select(&runtime);
        self.save(app)?;
        let meta = runtime.meta.lock().unwrap().clone();
        Ok(meta)
    }

    pub async fn stop(&self, app: &Arc<App>, bot_id: &str, id: &str) -> Result<Session, String> {
        let runtime = self.owned(app, bot_id, id)?;
        runtime.stopping.fetch_add(1, Ordering::AcqRel);
        let _stopping = Stopping(&runtime.stopping);
        let meta = {
            let mut meta = runtime.meta.lock().unwrap();
            meta.state = Control::Stopped;
            meta.revision += 1;
            meta.clone()
        };
        // Stop is available even while takeover or launch holds the input gate.
        runtime.opening.lock().unwrap().cancel();
        let server = runtime.server.lock().unwrap().take();
        let immediate = runtime.input.clone().try_lock_owned().ok();
        if let Some(server) = &server {
            if immediate.is_some() {
                // With no active input, close Chromium gracefully so recent
                // cookies and storage are flushed before checkpointing.
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(3),
                    server.browser_call("browser_close", json!({})),
                )
                .await;
            }
            server.stop();
        }
        self.save(app)?;
        let _guard = match immediate {
            Some(guard) => guard,
            None => runtime.input.clone().lock_owned().await,
        };
        if let Some(server) = server {
            server.wait_stopped().await?;
        }
        let dek = app.dek().ok_or("The account key is unavailable.")?;
        let dir = profile::directory(&app.config.home.join("browser/profiles"), id);
        let owned_id = id.to_string();
        tokio::task::spawn_blocking(move || profile::seal(&dir, &dek, &owned_id))
            .await
            .map_err(|e| e.to_string())??;
        let output = app.config.home.join("browser/evidence").join(id);
        if output.is_dir() {
            std::fs::remove_dir_all(output).map_err(|e| e.to_string())?;
        }
        Ok(meta)
    }

    pub async fn screenshot(
        &self,
        app: &Arc<App>,
        bot_id: &str,
        id: &str,
        chat_id: &str,
        summary: &str,
    ) -> Result<Message, String> {
        let runtime = self.owned(app, bot_id, id)?;
        require_chat(app, chat_id, bot_id)?;
        let _guard = runtime.input.lock().await;
        self.owned(app, bot_id, id)?;
        let server = runtime
            .server
            .lock()
            .unwrap()
            .clone()
            .filter(|s| !s.is_closed())
            .ok_or("Open the browser before taking a screenshot.")?;
        let result = server
            .browser_call("browser_take_screenshot", json!({ "type": "png" }))
            .await?;
        publish_image(app, bot_id, chat_id, id, summary, &result)
    }

    pub fn reset(&self) {
        let mut items = self.items.lock().unwrap();
        for runtime in items.values() {
            runtime.opening.lock().unwrap().cancel();
            if let Some(server) = runtime.server.lock().unwrap().as_ref() {
                server.stop();
            }
            let mut meta = runtime.meta.lock().unwrap();
            meta.state = Control::Stopped;
            meta.revision += 1;
        }
        items.clear();
        drop(items);
        *self.account.lock().unwrap() = None;
        self.changed.notify_waiters();
    }

    pub async fn shutdown(&self, app: &Arc<App>) {
        for job in app.running_jobs.lock().unwrap().values() {
            job.cancel.cancel();
        }
        let sessions: Vec<_> = self.items.lock().unwrap().values().cloned().collect();
        // Revoke and stop every process before waiting on any of its gates.
        for runtime in &sessions {
            let mut meta = runtime.meta.lock().unwrap();
            meta.state = Control::Stopped;
            meta.revision += 1;
            runtime.opening.lock().unwrap().cancel();
            if let Some(server) = runtime.server.lock().unwrap().as_ref() {
                server.stop();
            }
        }
        self.changed.notify_waiters();
        for runtime in sessions {
            let meta = runtime.meta.lock().unwrap().clone();
            let _guard = runtime.input.lock().await;
            let server = runtime.server.lock().unwrap().take();
            if let Some(server) = server {
                if let Err(error) = server.wait_stopped().await {
                    tracing::warn!(%error, "closing browser session");
                    continue;
                }
            }
            if let Some(dek) = app.dek() {
                let dir = profile::directory(&app.config.home.join("browser/profiles"), &meta.id);
                let sealed =
                    tokio::task::spawn_blocking(move || profile::seal(&dir, &dek, &meta.id)).await;
                if let Err(error) = sealed.unwrap_or_else(|e| Err(e.to_string())) {
                    tracing::warn!(%error, "sealing browser profile");
                }
            }
        }
        if !self.items.lock().unwrap().is_empty() {
            let _ = self.save(app);
        }
    }
}

fn valid_id(id: &str) -> bool {
    id.strip_prefix("browser-")
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
}

fn label(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 100 || text.chars().any(char::is_control) {
        return Err("Account and profile labels require 1–100 printable characters.".into());
    }
    Ok(text.to_string())
}

fn require_chat(app: &App, chat_id: &str, bot_id: &str) -> Result<(), String> {
    let chat = app.chat(chat_id).ok_or("Unknown chat")?;
    if !chat.meta.bot_ids.iter().any(|id| id == bot_id) {
        return Err("The evidence chat does not contain this session's bot.".into());
    }
    Ok(())
}

/// The screenshot is an immutable chat message with an existing encrypted file
/// blob. #80's outputs::publish adds output metadata at this one publication point.
pub fn publish_image(
    app: &Arc<App>,
    bot_id: &str,
    chat_id: &str,
    session_id: &str,
    summary: &str,
    result: &rmcp::model::CallToolResult,
) -> Result<Message, String> {
    use base64::Engine;
    require_chat(app, chat_id, bot_id)?;
    let image = result
        .content
        .iter()
        .find_map(|block| match block {
            rmcp::model::ContentBlock::Image(image) => Some(image),
            _ => None,
        })
        .ok_or("The Browser server returned no screenshot image.")?;
    if image.mime_type != "image/png" {
        return Err("Verification requires a PNG screenshot.".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&image.data)
        .map_err(|e| e.to_string())?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("The Browser server returned invalid PNG evidence.".into());
    }
    let path = app
        .config
        .home
        .join("browser")
        .join(format!("{}.png", uuid::Uuid::new_v4()));
    crate::config::write_private(&path, &bytes).map_err(|e| e.to_string())?;
    let attachment = crate::files::store(
        app,
        &crate::files::OutgoingFile {
            id: None,
            path: path.display().to_string(),
            name: Some(format!("{session_id}.png")),
            mime: Some("image/png".into()),
            width: None,
            height: None,
        },
    );
    let _ = std::fs::remove_file(path);
    let attachment = attachment.map_err(|e| e.to_string())?;
    crate::files::push_blob(app, Some(chat_id), &attachment).map_err(|e| e.to_string())?;
    let mut message = Message::new(
        chat_id,
        Author::Bot {
            bot_id: bot_id.to_string(),
        },
        Body::text(format!(
            "{}\nBrowser session: {session_id}. Verification screenshot; outcome unverified.",
            label(summary)?
        )),
    );
    if let Body::Text { attachments, .. } = &mut message.body {
        attachments.push(attachment);
    }
    app.upsert_message(message.clone(), true);
    Ok(message)
}

pub struct SessionTool {
    pub app: Arc<App>,
    pub bot: Bot,
    pub chat_id: String,
}

#[async_trait]
impl Tool for SessionTool {
    fn name(&self) -> &str {
        "browser_session"
    }
    fn description(&self) -> &str {
        "Create, list, or explicitly open a persistent visible Browser session on your assigned Runner. Select an account label and a separate profile label when creating one; its profile belongs only to you. The user can Take Over, Return to Bot, or Stop in the Browser Sessions sheet. Never compete for input during takeover. After opening, use the Browser plugin through codemode; browser_take_screenshot attaches encrypted verification evidence to this chat. Stronger isolation uses a dedicated Runner."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": { "action": { "type": "string", "enum": ["list", "create", "open"] }, "session_id": { "type": "string" }, "account": { "type": "string" }, "profile": { "type": "string" } }, "required": ["action"] })
    }
    async fn execute(
        &self,
        _id: &str,
        args: Value,
        cancel: CancellationToken,
        _on_update: ToolUpdateFn,
    ) -> Result<ToolResult, ToolError> {
        if cancel.is_cancelled() {
            return Err(ToolError("Stopped".into()));
        }
        self.app
            .browser_sessions
            .local_bot(&self.app, &self.bot.id)
            .map_err(ToolError)?;
        let result = match args["action"].as_str() {
            Some("list") => json!(self
                .app
                .browser_sessions
                .list(&self.app, &self.bot.id)
                .map_err(ToolError)?),
            Some("create") => {
                if self
                    .app
                    .browser_sessions
                    .list(&self.app, &self.bot.id)
                    .map_err(ToolError)?
                    .iter()
                    .any(|s| matches!(s.state, Control::Human | Control::TakingOver))
                {
                    return Err(ToolError("The user controls this bot's browser.".into()));
                }
                json!(self
                    .app
                    .browser_sessions
                    .create(
                        &self.app,
                        &self.bot.id,
                        args["account"].as_str().unwrap_or("Default"),
                        args["profile"].as_str().unwrap_or("Browser")
                    )
                    .map_err(ToolError)?)
            }
            Some("open") => {
                let id = args["session_id"]
                    .as_str()
                    .ok_or_else(|| ToolError("missing session_id".into()))?;
                let opened = tokio::select! {
                    opened = self.app.browser_sessions.open(&self.app, &self.bot.id, id, false) => opened.map_err(ToolError)?,
                    _ = cancel.cancelled() => {
                        let _ = self.app.browser_sessions.stop(&self.app, &self.bot.id, id).await;
                        return Err(ToolError("Stopped".into()));
                    }
                };
                json!(opened)
            }
            _ => return Err(ToolError("Unknown browser session action".into())),
        };
        Ok(ToolResult::text(serde_json::to_string(&result).unwrap()))
    }
}

pub async fn review_call(
    app: &Arc<App>,
    bot: &Bot,
    chat_id: &str,
    trigger: &crate::plugins::review::Trigger,
    unattended: bool,
    ctx: &BeforeToolCallContext<'_>,
) -> Option<BeforeToolCallResult> {
    if ctx.tool_call.name != "browser_session" || ctx.args["action"] == "list" {
        return None;
    }
    let description =
        "Create or open this bot's visible, persistent browser profile on its assigned Runner.";
    let outcome = crate::plugins::review::decide(
        app,
        bot,
        chat_id,
        trigger,
        "browser-session",
        "Browser",
        "browser_session",
        description,
        ctx.args,
        None,
        ctx.cancel,
    )
    .await;
    let crate::plugins::review::Outcome::Ask { reason, .. } = outcome else {
        return None;
    };
    if unattended {
        return Some(crate::local_review::blocked(
            "Opening a visible browser needs approval; nobody is here to approve it.".into(),
        ));
    }
    match crate::plugins::mcp::ask(
        app,
        chat_id,
        &bot.id,
        "browser-session",
        "Browser",
        "browser_session",
        description,
        ctx.args.clone(),
        reason,
        ctx.cancel,
    )
    .await
    {
        crate::plugins::mcp::Decision::Allowed | crate::plugins::mcp::Decision::Always => None,
        crate::plugins::mcp::Decision::Dismissed => Some(crate::local_review::dismissed(
            "The user wrote instead of approving the browser session.",
        )),
        _ => Some(crate::local_review::blocked(
            "The browser session was not approved.".into(),
        )),
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
