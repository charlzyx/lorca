//! Durable, version-bound proposals owned by one Runner. Only ciphertext is persisted or
//! synced; paired Devices read it and send decisions back to that Runner.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::app::{App, OutboxItem, Slot};
use crate::config::now_secs;
use crate::events::Event;
use crate::model::{Author, Body, Message};

const MAX_ITEM_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewOrigin {
    pub chat_id: String,
    pub message_id: Option<String>,
    pub routine_id: Option<String>,
    /// The canonical task id from the task subject; the queue owns no task records.
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewTarget {
    pub account: String,
    pub resource: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewPayload {
    /// Accepting a draft records the accepted text without publishing it to a service.
    Draft {
        text: String,
    },
    Shell {
        arguments: Value,
    },
    Plugin {
        plugin_id: String,
        server_name: String,
        tool: String,
        arguments: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewedFile {
    pub path: String,
    /// None means the file does not exist. Creating it also invalidates the review.
    pub hash: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReviewPreconditions {
    pub authorization_hash: String,
    pub workdir: String,
    pub connection_hash: Option<String>,
    pub tool_hash: Option<String>,
    pub files: Vec<ReviewedFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Pending,
    Approved,
    Executing,
    Succeeded,
    Failed,
    Rejected,
    Cancelled,
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewChange {
    Created,
    Edited,
    Approved,
    Rejected,
    Cancelled,
    Invalidated,
    Executed,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewActivity {
    pub id: String,
    pub change: ReviewChange,
    pub version: u64,
    pub actor_device_id: String,
    pub at: f64,
    pub previous_payload: Option<ReviewPayload>,
    pub payload: ReviewPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewApproval {
    pub version: u64,
    pub digest: String,
    pub device_id: String,
    pub at: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewOutcome {
    pub summary: String,
    pub result: Option<Value>,
    pub message_id: String,
    pub at: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewItem {
    pub id: String,
    pub runner_id: String,
    pub bot_id: String,
    /// Immutable receipt binding a create request id to its original proposal.
    pub request_hash: String,
    pub origin: ReviewOrigin,
    pub target: ReviewTarget,
    pub rationale: String,
    pub payload: ReviewPayload,
    /// Changes whenever the reviewed payload, context, or preconditions change.
    pub version: u64,
    /// Changes on every persisted state transition, including execution claims.
    pub revision: u64,
    pub preconditions: ReviewPreconditions,
    pub state: ReviewState,
    pub approval: Option<ReviewApproval>,
    pub outcome: Option<ReviewOutcome>,
    pub history: Vec<ReviewActivity>,
    pub created_at: f64,
    pub updated_at: f64,
}

impl ReviewItem {
    pub fn message_id(&self) -> String {
        format!("review-status-{}", self.id)
    }

    /// Matches TaskEvidence's review reference. Linking it never completes a task.
    pub fn outcome_evidence(&self) -> Value {
        json!({ "kind": "review", "label": self.outcome.as_ref().map(|outcome| outcome.summary.as_str()).unwrap_or("Review item"),
            "review_id": self.id, "chat_id": self.origin.chat_id, "message_id": self.message_id() })
    }

    pub fn reviewed_digest(&self) -> String {
        fingerprint(
            &json!({ "id": self.id, "runner_id": self.runner_id, "bot_id": self.bot_id, "origin": self.origin,
            "version": self.version, "payload": self.payload, "target": self.target, "rationale": self.rationale, "preconditions": self.preconditions }),
        )
    }

    pub(crate) fn record(
        &mut self,
        change: ReviewChange,
        actor: &str,
        previous_payload: Option<ReviewPayload>,
    ) {
        self.revision += 1;
        self.updated_at = now_secs();
        self.history.push(ReviewActivity {
            id: uuid::Uuid::new_v4().to_string(),
            change,
            version: self.version,
            actor_device_id: actor.into(),
            at: self.updated_at,
            previous_payload,
            payload: self.payload.clone(),
        });
    }
}

pub fn fingerprint(value: &impl Serialize) -> String {
    crate::keys::b64(&Sha256::digest(
        serde_json::to_vec(value).expect("serializable review value"),
    ))
}

fn key(app: &App) -> Result<[u8; 32], String> {
    app.dek()
        .ok_or_else(|| "Pair or create an identity first.".into())
}

pub(crate) fn load(app: &App, id: &str) -> Result<(ReviewItem, Vec<u8>), String> {
    let raw = app
        .store
        .review(id)
        .map_err(|error| error.to_string())?
        .ok_or("Unknown review item")?;
    let item: ReviewItem = crate::crypto::decrypt_json(&key(app)?, "review", &raw)
        .map_err(|error| error.to_string())?;
    if item.id != id {
        return Err("Review id does not match its encrypted record".into());
    }
    Ok((item, raw))
}

pub fn get(app: &App, id: &str) -> Result<ReviewItem, String> {
    load(app, id).map(|(item, _)| item)
}

pub fn list(app: &App) -> Result<Vec<ReviewItem>, String> {
    let Some(dek) = app.dek() else {
        return Ok(Vec::new());
    };
    let mut items = app
        .store
        .reviews()
        .map_err(|error| error.to_string())?
        .iter()
        .map(|raw| {
            crate::crypto::decrypt_json::<ReviewItem>(&dek, "review", raw)
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    items.sort_by(|a, b| {
        b.updated_at
            .total_cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(items)
}

/// Called with the review lock held. The item and its encrypted relay outbox entry commit in
/// one SQLite transaction, before a decision is acknowledged or an external action starts.
pub(crate) fn save(
    app: &App,
    item: &ReviewItem,
    previous: Option<&[u8]>,
    change: ReviewChange,
) -> Result<(), String> {
    let plain = serde_json::to_vec(item).map_err(|error| error.to_string())?;
    let limit = if matches!(change, ReviewChange::Created | ReviewChange::Edited) {
        MAX_ITEM_BYTES - 96 * 1024
    } else {
        MAX_ITEM_BYTES
    };
    if plain.len() > limit {
        return Err("Review item is too large; create a separate proposal.".into());
    }
    let ciphertext =
        crate::crypto::encrypt(&key(app)?, "review", &plain).map_err(|error| error.to_string())?;
    let upload = OutboxItem {
        id: uuid::Uuid::new_v4().to_string(),
        kind: "review".into(),
        recipient: None,
        ciphertext: ciphertext.clone(),
        slot: Some(Slot::latest(crate::model::relay_name(&format!(
            "review/{}",
            item.id
        )))),
        group: None,
    };
    app.store
        .save_review(&item.id, previous, &ciphertext, Some(&upload))
        .map_err(|error| error.to_string())?;
    app.wake_sync();
    app.emit(Event::ReviewChanged {
        item: item.clone(),
        change,
    });
    Ok(())
}

/// Projects the newest status into the originating work using a stable message id. The
/// encrypted review remains authoritative if writing the chat row is interrupted.
pub(crate) fn publish_origin(app: &App, id: &str) {
    let Ok(item) = get(app, id) else { return };
    if app.chat(&item.origin.chat_id).is_none() {
        return;
    }
    let text = match &item.outcome {
        Some(outcome) => {
            let detail = outcome
                .result
                .as_ref()
                .and_then(|result| result["text"].as_str())
                .unwrap_or("");
            let shown: String = detail.chars().take(2000).collect();
            format!(
                "Review {} · {}{}",
                item.id,
                outcome.summary,
                if shown.is_empty() {
                    String::new()
                } else {
                    format!("\n{shown}")
                }
            )
        }
        None => format!(
            "Review {} · {:?} · {} → {}. {}",
            item.id, item.state, item.target.account, item.target.resource, item.rationale
        ),
    };
    // routine_id on a Notice means a routine's run marker to the transcript builder.
    let mut message = Message::new(
        &item.origin.chat_id,
        Author::System,
        Body::Notice {
            text,
            routine_id: None,
        },
    );
    message.id = item.message_id();
    message.created_at = item.created_at;
    app.upsert_message(message, true);
}

/// A newer encrypted projection arrives. A Runner's own durable claims are authoritative;
/// other Devices never execute a synced approval.
pub fn apply(app: &App, ciphertext: &[u8]) -> Result<(), String> {
    let item: ReviewItem = crate::crypto::decrypt_json(&key(app)?, "review", ciphertext)
        .map_err(|error| error.to_string())?;
    if item.id.is_empty() || item.version == 0 || item.revision == 0 {
        return Err("Invalid review record".into());
    }
    let _lock = app.review_lock.lock().unwrap();
    let previous = app
        .store
        .review(&item.id)
        .map_err(|error| error.to_string())?;
    if let Some(raw) = &previous {
        let old: ReviewItem = crate::crypto::decrypt_json(&key(app)?, "review", raw)
            .map_err(|error| error.to_string())?;
        if old.runner_id != item.runner_id
            || old.revision >= item.revision
            || app.this_device_id().as_deref() == Some(item.runner_id.as_str())
        {
            return Ok(());
        }
    } else if app.this_device_id().as_deref() == Some(item.runner_id.as_str()) {
        // A restored Runner may display its old items, but cannot replay an old approval.
        let mut restored = item.clone();
        if matches!(
            restored.state,
            ReviewState::Approved | ReviewState::Executing
        ) {
            restored.state = ReviewState::Uncertain;
            restored.approval = None;
            restored.outcome = Some(ReviewOutcome { summary: "Recovered from sync; execution cannot be confirmed. Inspect the target before creating another proposal.".into(),
                result: None, message_id: restored.message_id(), at: now_secs() });
            restored.record(ReviewChange::Interrupted, &item.runner_id, None);
            return save(app, &restored, None, ReviewChange::Interrupted);
        }
    }
    app.store
        .save_review(&item.id, previous.as_deref(), ciphertext, None)
        .map_err(|error| error.to_string())?;
    let change = item
        .history
        .last()
        .map(|entry| entry.change)
        .unwrap_or(ReviewChange::Created);
    app.emit(Event::ReviewChanged { item, change });
    Ok(())
}

/// Re-publish owned records after a relay/account resync without letting stale projections
/// overwrite local state.
pub fn enqueue_owned(app: &App) {
    let Ok(items) = list(app) else { return };
    for item in items
        .into_iter()
        .filter(|item| app.this_device_id().as_deref() == Some(item.runner_id.as_str()))
    {
        if let Ok(raw) = crate::crypto::encrypt_json(&app.dek().unwrap(), "review", &item) {
            app.push_slot_blob(
                "review",
                Slot::latest(crate::model::relay_name(&format!("review/{}", item.id))),
                None,
                raw,
            );
        }
    }
}

/// Stable JSON adapter for the feedback subject. Its recorder owns deduplication by event
/// id and exclusions. User edits/approvals/rejections are evidence; silence and interruption
/// generate none. Boxing keeps the optional API integration independent of sibling modules.
pub async fn forward_feedback(app: &Arc<App>, item: &ReviewItem) -> Result<(), String> {
    for event in &item.history {
        let kind = match event.change {
            ReviewChange::Approved => "accepted",
            ReviewChange::Rejected => "rejected",
            ReviewChange::Edited => "edited",
            _ => continue,
        };
        let before = event
            .previous_payload
            .as_ref()
            .map(|payload| serde_json::to_string_pretty(payload).unwrap_or_default());
        let after = (event.change == ReviewChange::Edited)
            .then(|| serde_json::to_string_pretty(&event.payload).unwrap_or_default());
        let feedback = json!({ "kind": kind, "event_id": event.id, "origin": { "chat_id": item.origin.chat_id,
            "message_id": item.origin.message_id.as_ref().cloned().unwrap_or_else(|| item.message_id()), "routine_id": item.origin.routine_id,
            "review_id": item.id, "task_id": item.origin.task_id }, "note": format!("User {kind} review {} version {}", item.id, event.version),
            "before": before, "after": after, "excluded": false });
        Box::pin(crate::api::dispatch(
            app,
            "feedback.record",
            json!({ "bot_id": item.bot_id, "feedback": feedback }),
        ))
        .await?;
    }
    Ok(())
}

/// The task subject is authoritative for evidence and completion. A queue outcome appends a
/// reference through its CAS API and never infers a completed task. The persisted item/chat
/// reference remains available while its task authority is offline or the adapter is absent.
pub async fn forward_task_outcome(app: &Arc<App>, item: &ReviewItem) -> Result<(), String> {
    let Some(task_id) = &item.origin.task_id else {
        return Ok(());
    };
    if item.outcome.is_none()
        || matches!(
            item.state,
            ReviewState::Pending | ReviewState::Approved | ReviewState::Executing
        )
    {
        return Ok(());
    }
    let task = Box::pin(crate::api::dispatch(
        app,
        "tasks.get",
        json!({ "id": task_id }),
    ))
    .await?;
    let mut evidence = task["evidence"].as_array().cloned().unwrap_or_default();
    if evidence
        .iter()
        .any(|entry| entry["review_id"].as_str() == Some(item.id.as_str()))
    {
        return Ok(());
    }
    evidence.push(item.outcome_evidence());
    Box::pin(crate::api::dispatch(
        app,
        "tasks.update",
        json!({ "id": task_id, "expected_revision": task["revision"],
        "request_id": format!("review-{}-outcome", item.id), "evidence": evidence }),
    ))
    .await?;
    Ok(())
}

pub(crate) fn local_bot(app: &App, item: &ReviewItem) -> Result<crate::model::Bot, String> {
    if app.this_device_id().as_deref() != Some(item.runner_id.as_str()) {
        return Err("This review belongs to another Runner.".into());
    }
    let bot = app
        .bot(&item.bot_id)
        .ok_or("The originating bot was deleted.")?;
    if bot.runner_id != item.runner_id {
        return Err(
            "The bot's Runner assignment changed; create a new review on its current Runner."
                .into(),
        );
    }
    let chat = app
        .chat(&item.origin.chat_id)
        .ok_or("The originating chat was deleted.")?;
    if !chat.meta.bot_ids.contains(&bot.id) {
        return Err("The bot is no longer in the originating chat.".into());
    }
    if let Some(id) = &item.origin.routine_id {
        let routine = app
            .routine(id)
            .ok_or("The originating routine was deleted.")?;
        if routine.bot_id != bot.id {
            return Err("The originating routine changed owner.".into());
        }
    }
    Ok(bot)
}

pub(crate) fn require_version(item: &ReviewItem, params: &Value) -> Result<(), String> {
    if params["expected_version"].as_u64() != Some(item.version) {
        return Err("This review version changed. Reload and review it again.".into());
    }
    Ok(())
}

fn require_actor(app: &App, actor: &str) -> Result<(), String> {
    if actor.is_empty()
        || (app.this_device_id().as_deref() != Some(actor) && app.device(actor).is_none())
    {
        return Err("This decision requires a paired Device.".into());
    }
    Ok(())
}

/// App/local-core entry point. Reads work from the encrypted local projection even offline;
/// mutations are sealed to the one Runner that owns the review.
pub async fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    match method {
        "reviews.list" => {
            let items = list(app)?
                .into_iter()
                .filter(|item| {
                    params["chat_id"]
                        .as_str()
                        .is_none_or(|chat| chat == item.origin.chat_id)
                        && params["task_id"]
                            .as_str()
                            .is_none_or(|task| item.origin.task_id.as_deref() == Some(task))
                })
                .collect::<Vec<_>>();
            return Ok(json!(items));
        }
        "reviews.get" => return Ok(json!(get(app, params["id"].as_str().ok_or("missing id")?)?)),
        _ => {}
    }
    let runner = if method == "reviews.create" {
        app.bot(params["bot_id"].as_str().ok_or("missing bot_id")?)
            .ok_or("Unknown bot")?
            .runner_id
    } else {
        get(app, params["id"].as_str().ok_or("missing id")?)?.runner_id
    };
    if app.this_device_id().as_deref() != Some(&runner) {
        return crate::requests::ask(app, &runner, method, params).await;
    }
    serve(
        app,
        method,
        &params,
        &app.this_device_id().unwrap_or_default(),
    )
    .await
}

pub async fn serve(
    app: &Arc<App>,
    method: &str,
    params: &Value,
    actor: &str,
) -> Result<Value, String> {
    require_actor(app, actor)?;
    #[cfg(feature = "runner")]
    if method == "reviews.create" || method == "reviews.edit" || method == "reviews.approve" {
        return crate::review_execution::mutate(app, method, params, actor)
            .await
            .map(|item| json!(item));
    }
    if method != "reviews.reject" && method != "reviews.cancel" {
        return Err("This Runner does not support that review action.".into());
    }
    let id = params["id"].as_str().ok_or("missing id")?;
    let item = {
        let _lock = app.review_lock.lock().unwrap();
        let (mut item, previous) = load(app, id)?;
        require_version(&item, params)?;
        if app.this_device_id().as_deref() != Some(item.runner_id.as_str()) {
            return Err("This review belongs to another Runner.".into());
        }
        let (state, change) = if method == "reviews.reject" {
            (ReviewState::Rejected, ReviewChange::Rejected)
        } else {
            (ReviewState::Cancelled, ReviewChange::Cancelled)
        };
        if item.state == state {
            return Ok(json!(item));
        }
        if !matches!(item.state, ReviewState::Pending | ReviewState::Approved) {
            return Err("This review has already started or ended.".into());
        }
        item.state = state;
        item.approval = None;
        item.outcome = Some(ReviewOutcome {
            summary: params["reason"]
                .as_str()
                .filter(|reason| !reason.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if state == ReviewState::Rejected {
                        "Rejected by the user".into()
                    } else {
                        "Cancelled by the user".into()
                    }
                }),
            result: None,
            message_id: item.message_id(),
            at: now_secs(),
        });
        item.record(change, actor, None);
        save(app, &item, Some(&previous), change)?;
        item
    };
    publish_origin(app, id);
    let _ = forward_feedback(app, &item).await;
    Ok(json!(item))
}
