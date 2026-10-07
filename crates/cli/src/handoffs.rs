//! Durable delegation contracts. Requests, Runner reports, and requester cancellations merge
//! independently per attempt; encrypted records and their relay outbox share one transaction.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app::{App, OutboxItem, Slot};
use crate::config::now_secs;
use crate::model::{Author, Body, Job, JobCancel, Message, MessageState, MAX_BOT_HOPS};
use crate::runtime::TurnOutcome;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HandoffRequest {
    pub handoff_id: String,
    pub attempt: u32,
    pub job_id: String,
    pub from_bot_id: String,
    pub source_chat_id: String,
    pub source_runner_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub target_bot_id: String,
    pub target_chat_id: String,
    pub target_runner_id: String,
    pub trigger_message_id: String,
    pub message: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub expected_output: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    pub hops: u32,
    pub created_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HandoffJob {
    Request {
        request: HandoffRequest,
    },
    Result {
        handoff_id: String,
        request_job_id: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HandoffStatus {
    Running,
    Completed,
    Failed,
    Blocked,
    Cancelled,
}

impl HandoffStatus {
    pub fn terminal(self) -> bool {
        self != Self::Running
    }
}

/// Wire-compatible with canonical TaskEvidence and immutable output-version references.
/// These are references to supporting records, never another output or task store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResultLink {
    pub kind: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_id: Option<String>,
}

impl ResultLink {
    fn message(message: &Message, label: String) -> Self {
        Self {
            kind: "message".into(),
            label,
            chat_id: Some(message.chat_id.clone()),
            message_id: Some(message.id.clone()),
            attachment_id: None,
            url: None,
            output_id: None,
            version: None,
            review_id: None,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.label.trim().is_empty() {
            return Err("A result link needs a label".into());
        }
        let has_message = self.chat_id.as_ref().is_some_and(|s| !s.is_empty())
            && self.message_id.as_ref().is_some_and(|s| !s.is_empty());
        let valid = match self.kind.as_str() {
            "message" => has_message,
            "output" => {
                has_message
                    && self.output_id.as_ref().is_some_and(|s| !s.is_empty())
                    && self.version.is_some_and(|v| v > 0)
            }
            "review" => has_message && self.review_id.as_ref().is_some_and(|s| !s.is_empty()),
            "file" => has_message && self.attachment_id.as_ref().is_some_and(|s| !s.is_empty()),
            "url" => self.url.as_ref().is_some_and(|s| {
                reqwest::Url::parse(s)
                    .is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some())
            }),
            _ => false,
        };
        if !valid {
            return Err(format!("Invalid {} result reference", self.kind));
        }
        Ok(())
    }
}

fn request_link(request: &HandoffRequest) -> ResultLink {
    ResultLink {
        kind: "message".into(),
        label: "Delegated request".into(),
        chat_id: Some(request.target_chat_id.clone()),
        message_id: Some(request.trigger_message_id.clone()),
        attachment_id: None,
        url: None,
        output_id: None,
        version: None,
        review_id: None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HandoffReport {
    pub status: HandoffStatus,
    pub summary: String,
    #[serde(default)]
    pub result_links: Vec<ResultLink>,
    /// Bot claims and observed turn/tool outcomes; the linked rows provide the underlying record.
    #[serde(default)]
    pub evidence: Vec<String>,
    pub created_at: f64,
    /// The last row before execution, so automatic reporting captures only this turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_after: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ResultDelivery {
    #[default]
    Pending,
    Started,
    Finished,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HandoffAttempt {
    pub request: HandoffRequest,
    pub delivery: String,
    pub report: Option<HandoffReport>,
    /// A requester cancellation is independent of Runner progress and wins a concurrent finish.
    pub cancellation: Option<HandoffReport>,
    #[serde(default)]
    result_delivery: ResultDelivery,
}

impl HandoffAttempt {
    pub fn outcome(&self) -> Option<&HandoffReport> {
        self.cancellation.as_ref().or(self.report.as_ref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Handoff {
    pub id: String,
    pub attempts: Vec<HandoffAttempt>,
}

impl Handoff {
    pub fn current(&self) -> &HandoffAttempt {
        self.attempts.last().expect("a handoff has an attempt")
    }
    pub fn view(&self) -> Value {
        let attempt = self.current();
        json!({ "handoff_id": self.id, "job_id": attempt.request.job_id, "request": attempt.request,
            "target_runner_id": attempt.request.target_runner_id, "delivery": match attempt.outcome() {
                Some(report) if report.status == HandoffStatus::Running => "running",
                Some(_) => "finished",
                None => &attempt.delivery,
            },
            "status": attempt.outcome().map(|r| r.status), "report": attempt.outcome(), "attempts": self.attempts })
    }
}

/// Each role has its own relay slot, so origin admission/cancellation cannot replace Runner
/// reports. Every update carries its request and can arrive before the request's own blob.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HandoffUpdate {
    Request {
        request: HandoffRequest,
        delivery: String,
    },
    Report {
        request: HandoffRequest,
        report: HandoffReport,
    },
    Cancel {
        request: HandoffRequest,
        report: HandoffReport,
    },
}

impl HandoffUpdate {
    fn request(&self) -> &HandoffRequest {
        match self {
            Self::Request { request, .. }
            | Self::Report { request, .. }
            | Self::Cancel { request, .. } => request,
        }
    }
    fn slot(&self) -> Slot {
        let kind = match self {
            Self::Request { .. } => "request",
            Self::Report { .. } => "report",
            Self::Cancel { .. } => "cancel",
        };
        let request = self.request();
        Slot::latest(format!("{}-{}-{kind}", request.handoff_id, request.attempt))
    }
}

fn key(app: &App) -> Result<[u8; 32], String> {
    app.dek().ok_or_else(|| "No account key".into())
}

pub fn get(app: &App, id: &str) -> Result<Handoff, String> {
    let bytes = app
        .store
        .handoff(id)
        .map_err(|e| e.to_string())?
        .ok_or("Unknown handoff")?;
    crate::crypto::decrypt_json(&key(app)?, "handoff_state", &bytes).map_err(|e| e.to_string())
}

pub fn list(app: &App) -> Result<Vec<Handoff>, String> {
    let dek = key(app)?;
    let mut records: Vec<Handoff> = app
        .store
        .handoffs()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|bytes| {
            crate::crypto::decrypt_json(&dek, "handoff_state", bytes).map_err(|e| e.to_string())
        })
        .collect::<Result<_, _>>()?;
    records.sort_by(|a, b| {
        a.current()
            .request
            .created_at
            .total_cmp(&b.current().request.created_at)
    });
    Ok(records)
}

fn save(
    app: &App,
    record: &Handoff,
    update: Option<&HandoffUpdate>,
    mut outbox: Vec<OutboxItem>,
) -> Result<(), String> {
    let dek = key(app)?;
    let bytes =
        crate::crypto::encrypt_json(&dek, "handoff_state", record).map_err(|e| e.to_string())?;
    if let Some(update) = update {
        outbox.insert(
            0,
            OutboxItem {
                id: uuid::Uuid::new_v4().to_string(),
                kind: "handoff".into(),
                recipient: None,
                ciphertext: crate::crypto::encrypt_json(&dek, "handoff", update)
                    .map_err(|e| e.to_string())?,
                slot: Some(update.slot()),
                group: None,
            },
        );
    }
    app.store
        .save_handoff(&record.id, &bytes, &outbox)
        .map_err(|e| e.to_string())?;
    if !outbox.is_empty() {
        app.outbox_notify.notify_waiters();
    }
    Ok(())
}

fn merge(app: &App, update: &HandoffUpdate) -> Result<Handoff, String> {
    let request = update.request();
    if request.attempt == 0 || request.hops > MAX_BOT_HOPS {
        return Err("Invalid handoff attempt".into());
    }
    let mut record = match app
        .store
        .handoff(&request.handoff_id)
        .map_err(|e| e.to_string())?
    {
        Some(bytes) => crate::crypto::decrypt_json::<Handoff>(&key(app)?, "handoff_state", &bytes)
            .map_err(|e| e.to_string())?,
        None => Handoff {
            id: request.handoff_id.clone(),
            attempts: Vec::new(),
        },
    };
    if let Some(first) = record.attempts.first() {
        let original = &first.request;
        if original.from_bot_id != request.from_bot_id
            || original.source_chat_id != request.source_chat_id
            || original.target_bot_id != request.target_bot_id
            || original.task_id != request.task_id
        {
            return Err("Handoff ownership and destination cannot change".into());
        }
    }
    let index = if let Some(index) = record
        .attempts
        .iter()
        .position(|a| a.request.attempt == request.attempt)
    {
        if record.attempts[index].request != *request {
            return Err("Conflicting handoff attempt".into());
        }
        index
    } else {
        record.attempts.push(HandoffAttempt {
            request: request.clone(),
            delivery: "queued".into(),
            report: None,
            cancellation: None,
            result_delivery: ResultDelivery::Pending,
        });
        record.attempts.sort_by_key(|a| a.request.attempt);
        record
            .attempts
            .iter()
            .position(|a| a.request.attempt == request.attempt)
            .unwrap()
    };
    let attempt = &mut record.attempts[index];
    match update {
        HandoffUpdate::Request { delivery, .. } => attempt.delivery = delivery.clone(),
        HandoffUpdate::Report { report, .. } => {
            // Terminal reports never regress to running or get replaced by another finish.
            if !attempt.report.as_ref().is_some_and(|r| r.status.terminal()) {
                attempt.report = Some(report.clone());
            }
        }
        HandoffUpdate::Cancel { report, .. } => {
            attempt.cancellation = Some(report.clone());
        }
    }
    Ok(record)
}

pub fn apply_update(app: &Arc<App>, update: HandoffUpdate) -> Result<(), String> {
    {
        let _guard = app.handoff_lock.lock().unwrap();
        let record = merge(app, &update)?;
        save(app, &record, None, Vec::new())?;
    }
    if let HandoffUpdate::Cancel { request, .. } = &update {
        app.cancel_job(&request.job_id);
    }
    route_report(app, &update.request().handoff_id)
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DelegateInput {
    pub bot_id: String,
    pub message: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub expected_output: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub task_id: Option<String>,
}

pub fn delegate(
    app: &Arc<App>,
    from_bot_id: &str,
    chat_id: &str,
    hops: u32,
    input: DelegateInput,
) -> Result<Value, String> {
    let from = app.bot(from_bot_id).ok_or("Requesting bot is gone")?;
    require_runner(app, &from.runner_id)?;
    let target = app.bot(input.bot_id.trim()).ok_or("Unknown target bot")?;
    let chat = app.chat(chat_id).ok_or("Chat is gone")?;
    if !chat.meta.bot_ids.contains(&from.id) {
        return Err("Requesting bot is not in this chat".into());
    }
    if target.id == from.id {
        return Err("You cannot message yourself".into());
    }
    if chat.meta.is_group() && chat.meta.bot_ids.contains(&target.id) {
        return Err(format!(
            "{} is in this chat and reads it. Say it here instead.",
            target.name
        ));
    }
    if hops >= MAX_BOT_HOPS {
        return Err(
            "Bots have passed this along eight times without the user. Answer the user instead."
                .into(),
        );
    }
    if input.message.trim().is_empty() {
        return Err("message is required".into());
    }
    if input.message.len()
        + input.context.len()
        + input.expected_output.len()
        + input
            .acceptance_criteria
            .iter()
            .map(String::len)
            .sum::<usize>()
        > 64 * 1024
    {
        return Err("A handoff contract is limited to 64 KiB".into());
    }
    let dm = app.dm_with(&target.id, None).map_err(|e| e.to_string())?;
    let request = HandoffRequest {
        handoff_id: format!("handoff-{}", uuid::Uuid::new_v4()),
        attempt: 1,
        job_id: format!("job-{}", uuid::Uuid::new_v4()),
        from_bot_id: from.id,
        source_chat_id: chat_id.into(),
        source_runner_id: from.runner_id,
        task_id: input.task_id,
        target_bot_id: target.id,
        target_chat_id: dm.meta.id,
        target_runner_id: target.runner_id,
        trigger_message_id: format!("msg-{}", uuid::Uuid::new_v4()),
        message: input.message.trim().into(),
        context: input.context,
        expected_output: input.expected_output,
        acceptance_criteria: input.acceptance_criteria,
        hops: hops + 1,
        created_at: now_secs(),
    };
    admit(app, request, None)
}

fn require_runner(app: &App, runner_id: &str) -> Result<(), String> {
    if app.this_device_id().as_deref() != Some(runner_id) {
        return Err("This operation belongs to the bot's assigned Runner".into());
    }
    Ok(())
}

fn request_job(request: &HandoffRequest) -> Job {
    Job {
        id: request.job_id.clone(),
        chat_id: request.target_chat_id.clone(),
        bot_id: request.target_bot_id.clone(),
        kind: "message".into(),
        trigger_message_id: request.trigger_message_id.clone(),
        task_id: request.task_id.clone(),
        handoff: Some(HandoffJob::Request {
            request: request.clone(),
        }),
        routine_id: None,
        check: None,
        requested_by: request.source_runner_id.clone(),
        from_bot_id: Some(request.from_bot_id.clone()),
        hops: request.hops,
        round: 0,
        is_winding_down: false,
        setup: None,
        created_at: request.created_at,
    }
}

fn incoming(app: &App, request: &HandoffRequest) {
    if app
        .message(&request.target_chat_id, &request.trigger_message_id)
        .is_some()
    {
        return;
    }
    let mut marker = Message::new(
        &request.target_chat_id,
        Author::Bot {
            bot_id: request.from_bot_id.clone(),
        },
        Body::Handoff {
            from: request.from_bot_id.clone(),
            to: request.target_bot_id.clone(),
            reason: contract_text(request),
        },
    );
    marker.id = request.trigger_message_id.clone();
    marker.created_at = request.created_at;
    app.upsert_message(marker, true);
}

pub fn contract_text(request: &HandoffRequest) -> String {
    let mut text = format!(
        "{}\n\nHandoff: {} (attempt {})\nReturn reports to bot {} in chat {}.",
        request.message,
        request.handoff_id,
        request.attempt,
        request.from_bot_id,
        request.source_chat_id
    );
    if let Some(task) = &request.task_id {
        text.push_str(&format!("\nParent task: {task}"));
    }
    if !request.context.is_empty() {
        text.push_str(&format!("\n\nSupplied context:\n{}", request.context));
    }
    if !request.expected_output.is_empty() {
        text.push_str(&format!(
            "\n\nExpected output:\n{}",
            request.expected_output
        ));
    }
    if !request.acceptance_criteria.is_empty() {
        text.push_str(&format!(
            "\n\nAcceptance criteria:\n{}",
            request
                .acceptance_criteria
                .iter()
                .map(|s| format!("- {s}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    text
}

fn admit(app: &Arc<App>, request: HandoffRequest, replaces: Option<&str>) -> Result<Value, String> {
    let local = app.this_device_id().as_deref() == Some(&request.target_runner_id);
    let mut jobs = Vec::new();
    let delivery = if local {
        "queued_local"
    } else {
        let runner = app
            .device(&request.target_runner_id)
            .filter(|d| d.is_runner() && !d.box_pubkey.is_empty())
            .ok_or("Target Runner is unknown or has no encryption key")?;
        let job = request_job(&request);
        jobs.push(OutboxItem {
            id: request.job_id.clone(),
            kind: "job".into(),
            recipient: Some(runner.id.clone()),
            ciphertext: crate::crypto::seal_json(&runner.box_pubkey, &job)
                .map_err(|e| e.to_string())?,
            slot: None,
            group: None,
        });
        if app.relay_url().is_none() {
            "waiting_for_relay"
        } else if app.device_is_online(&runner.id) {
            "queued_relay"
        } else {
            "waiting_for_runner"
        }
    }
    .to_string();
    let update = HandoffUpdate::Request {
        request: request.clone(),
        delivery,
    };
    let record = {
        let _guard = app.handoff_lock.lock().unwrap();
        if let Some(previous) = replaces {
            let current = get(app, &request.handoff_id)?;
            if current.current().request.job_id != previous {
                return Err("Handoff changed; inspect it and use its current job_id".into());
            }
        }
        let record = merge(app, &update)?;
        save(app, &record, Some(&update), jobs)?;
        record
    };
    incoming(app, &request);
    if local {
        crate::runtime::spawn_local_job(app.clone(), request_job(&request), None);
    }
    Ok(record.view())
}

/// Claims execution before effects start. A duplicate job or a cancelled/superseded attempt
/// never starts another turn; a running claim from a previous process is recovered as failure.
pub fn begin_job(app: &Arc<App>, job: &Job) -> Result<bool, String> {
    let Some(binding) = &job.handoff else {
        return Ok(true);
    };
    let _guard = app.handoff_lock.lock().unwrap();
    match binding {
        HandoffJob::Request { request } => {
            require_runner(app, &request.target_runner_id)?;
            if app
                .bot(&request.target_bot_id)
                .is_some_and(|b| b.runner_id != request.target_runner_id)
            {
                return Err(
                    "The recipient moved to another Runner; inspect and create a new handoff"
                        .into(),
                );
            }
            let update = HandoffUpdate::Request {
                request: request.clone(),
                delivery: "queued_local".into(),
            };
            let mut record = merge(app, &update)?;
            if record.current().request.job_id != job.id || record.current().outcome().is_some() {
                return Ok(false);
            }
            incoming(app, request);
            let started_after = app
                .store
                .page(&job.chat_id, None, 1)
                .map_err(|e| e.to_string())?
                .0
                .last()
                .map(|m| m.id.clone());
            let report = HandoffReport {
                status: HandoffStatus::Running,
                summary: "Runner started the delegated turn".into(),
                result_links: Vec::new(),
                evidence: Vec::new(),
                created_at: now_secs(),
                started_after,
            };
            record.attempts.last_mut().unwrap().report = Some(report.clone());
            save(
                app,
                &record,
                Some(&HandoffUpdate::Report {
                    request: request.clone(),
                    report,
                }),
                Vec::new(),
            )?;
        }
        HandoffJob::Result {
            handoff_id,
            request_job_id,
        } => {
            let mut record = get(app, handoff_id)?;
            let attempt = record
                .attempts
                .iter_mut()
                .find(|a| &a.request.job_id == request_job_id)
                .ok_or("Unknown handoff attempt")?;
            if attempt.result_delivery != ResultDelivery::Pending {
                return Ok(false);
            }
            attempt.result_delivery = ResultDelivery::Started;
            save(app, &record, None, Vec::new())?;
        }
    }
    Ok(true)
}

/// Persist a queued recipient job before it waits for the chat lock. The relay may replay
/// the envelope after process loss; the encrypted claim decides whether it can run again.
pub fn stage_job(app: &App, job: &Job) -> Result<(), String> {
    if let Some(HandoffJob::Request { request }) = &job.handoff {
        let _guard = app.handoff_lock.lock().unwrap();
        let update = HandoffUpdate::Request {
            request: request.clone(),
            delivery: "queued_local".into(),
        };
        let record = merge(app, &update)?;
        save(app, &record, None, Vec::new())?;
    }
    Ok(())
}

pub fn finish_job(
    app: &Arc<App>,
    job: &Job,
    outcome: TurnOutcome,
    cancelled: bool,
) -> Result<(), String> {
    match &job.handoff {
        Some(HandoffJob::Request { request }) => {
            let record = get(app, &request.handoff_id)?;
            let attempt = record
                .attempts
                .iter()
                .find(|a| a.request.job_id == job.id)
                .ok_or("Unknown handoff attempt")?;
            if attempt.outcome().is_some_and(|r| r.status.terminal()) {
                return route_report(app, &record.id);
            }
            let (links, evidence, said, failure) = turn_evidence(app, attempt);
            let status = if cancelled {
                HandoffStatus::Cancelled
            } else if outcome == TurnOutcome::Skipped || failure.is_some() {
                HandoffStatus::Failed
            } else if outcome == TurnOutcome::Sent {
                HandoffStatus::Completed
            } else {
                HandoffStatus::Blocked
            };
            let summary = match status {
                HandoffStatus::Cancelled => "Delegated turn was cancelled".into(),
                HandoffStatus::Failed => failure.unwrap_or_else(|| "Delegated turn could not finish; inspect the recipient chat".into()),
                HandoffStatus::Blocked => "Delegated turn ended without an output or a completion report; follow up with the recipient".into(),
                _ => said.unwrap_or_else(|| "Delegated turn finished".into()),
            };
            let report = HandoffReport {
                status,
                summary,
                result_links: links,
                evidence,
                created_at: now_secs(),
                started_after: attempt
                    .report
                    .as_ref()
                    .and_then(|r| r.started_after.clone()),
            };
            publish_report(app, request.clone(), report)
        }
        Some(HandoffJob::Result {
            handoff_id,
            request_job_id,
        }) => {
            let _guard = app.handoff_lock.lock().unwrap();
            let mut record = get(app, handoff_id)?;
            if let Some(attempt) = record
                .attempts
                .iter_mut()
                .find(|a| a.request.job_id == *request_job_id)
            {
                attempt.result_delivery = ResultDelivery::Finished;
                save(app, &record, None, Vec::new())?;
            }
            Ok(())
        }
        None => Ok(()),
    }
}

fn turn_evidence(
    app: &App,
    attempt: &HandoffAttempt,
) -> (Vec<ResultLink>, Vec<String>, Option<String>, Option<String>) {
    let request = &attempt.request;
    let after = attempt
        .report
        .as_ref()
        .and_then(|r| r.started_after.as_deref())
        .unwrap_or(&request.trigger_message_id);
    let messages = app
        .store
        .messages_after(&request.target_chat_id, after)
        .unwrap_or_default();
    let (mut links, mut evidence, mut said, mut failure) = (Vec::new(), Vec::new(), None, None);
    for message in messages {
        if let (Author::System, Body::Notice { text, .. }) = (&message.author, &message.body) {
            failure = Some(text.clone());
            links.push(ResultLink::message(&message, "Runner notice".into()));
        }
        if message.author
            != (Author::Bot {
                bot_id: request.target_bot_id.clone(),
            })
        {
            continue;
        }
        if let MessageState::Failed { error } = &message.state {
            failure = Some(error.clone());
        }
        match &message.body {
            Body::Text { text, .. } if !text.trim().is_empty() && message.is_complete() => {
                said = Some(text.chars().take(8000).collect::<String>());
                links.push(ResultLink::message(&message, "Recipient response".into()));
            }
            Body::Tool {
                name,
                summary,
                is_running: false,
                is_error,
                ..
            } => {
                evidence.push(format!(
                    "{}: {} ({}, message {})",
                    name,
                    summary,
                    if *is_error { "error" } else { "returned" },
                    message.id
                ));
            }
            _ => {}
        }
        // #80's additive Message.output metadata supplies immutable version references once
        // that module is present. Reading its wire shape keeps this module independently usable.
        if let Ok(value) = serde_json::to_value(&message) {
            if let Some(output) = value.get("output").filter(|o| o.is_object()) {
                let mut link = ResultLink::message(
                    &message,
                    output["name"].as_str().unwrap_or("Output").into(),
                );
                link.kind = "output".into();
                link.output_id = output["id"].as_str().map(str::to_string);
                link.version = output["version"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok());
                if link.validate().is_ok() {
                    links.push(link);
                }
            }
        }
    }
    if links.is_empty() {
        links.push(request_link(request));
    }
    links.truncate(40);
    evidence.truncate(40);
    if evidence.is_empty() {
        evidence.push("The Runner observed the delegated turn's outcome; completion is a bot claim, not an independent acceptance review.".into());
    }
    (links, evidence, said, failure)
}

fn publish_report(
    app: &Arc<App>,
    request: HandoffRequest,
    report: HandoffReport,
) -> Result<(), String> {
    {
        let _guard = app.handoff_lock.lock().unwrap();
        let update = HandoffUpdate::Report {
            request: request.clone(),
            report,
        };
        let existing = get(app, &request.handoff_id)?;
        if !existing
            .attempts
            .iter()
            .find(|a| a.request.job_id == request.job_id)
            .and_then(|a| a.report.as_ref())
            .is_some_and(|r| r.status.terminal())
        {
            let record = merge(app, &update)?;
            save(app, &record, Some(&update), Vec::new())?;
        }
    }
    route_report(app, &request.handoff_id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportInput {
    pub status: HandoffStatus,
    pub summary: String,
    #[serde(default)]
    pub result_links: Vec<ResultLink>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

pub fn report(
    app: &Arc<App>,
    bot_id: &str,
    handoff_id: &str,
    job_id: &str,
    input: ReportInput,
) -> Result<Value, String> {
    if serde_json::to_vec(&input).map_err(|e| e.to_string())?.len() > 64 * 1024 {
        return Err("Handoff report is limited to 64 KiB".into());
    }
    let record = get(app, handoff_id)?;
    let attempt = record.current();
    let request = &attempt.request;
    if request.target_bot_id != bot_id || request.job_id != job_id {
        return Err("Only the recipient's active attempt can report this handoff".into());
    }
    require_runner(app, &request.target_runner_id)?;
    if !input.status.terminal() || input.summary.trim().is_empty() {
        return Err("A terminal status and summary are required".into());
    }
    if attempt.outcome().is_some_and(|r| r.status.terminal()) {
        return Err("This attempt already has a terminal report".into());
    }
    if input.summary.len() + input.evidence.iter().map(String::len).sum::<usize>() > 32 * 1024
        || input.result_links.len() > 40
    {
        return Err("Handoff report is too large".into());
    }
    for link in &input.result_links {
        link.validate()?;
    }
    let (automatic, observed, _, _) = turn_evidence(app, attempt);
    let mut links = input.result_links;
    for link in automatic {
        if !links.contains(&link) && links.len() < 40 {
            links.push(link);
        }
    }
    let mut evidence = input.evidence;
    evidence.extend(observed);
    if input.status == HandoffStatus::Completed
        && links
            .iter()
            .all(|link| link.message_id.as_deref() == Some(request.trigger_message_id.as_str()))
    {
        return Err(
            "Completion needs a result link or a response/output produced in this turn".into(),
        );
    }
    publish_report(
        app,
        request.clone(),
        HandoffReport {
            status: input.status,
            summary: input.summary.trim().into(),
            result_links: links,
            evidence,
            created_at: now_secs(),
            started_after: attempt
                .report
                .as_ref()
                .and_then(|r| r.started_after.clone()),
        },
    )?;
    Ok(get(app, handoff_id)?.view())
}

fn result_message(request: &HandoffRequest, report: &HandoffReport) -> Message {
    let status = serde_json::to_value(report.status)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    let mut text = format!(
        "Handoff {} · {status}\n\n{}",
        request.handoff_id, report.summary
    );
    if let Some(task) = &request.task_id {
        text.push_str(&format!("\n\nTask: {task}"));
    }
    for link in &report.result_links {
        let target = match (&link.chat_id, &link.message_id, &link.url) {
            (Some(chat), Some(message), _) => {
                let mut url = reqwest::Url::parse("lorca://message").unwrap();
                url.query_pairs_mut()
                    .append_pair("chat_id", chat)
                    .append_pair("message_id", message);
                Some(url.to_string().replace('+', "%20"))
            }
            (_, _, Some(url)) => Some(url.clone()),
            _ => None,
        };
        if let Some(target) = target {
            text.push_str(&format!(
                "\n- [{}]({target})",
                link.label.replace(['[', ']'], "")
            ));
        }
    }
    if !report.evidence.is_empty() {
        text.push_str(&format!(
            "\n\nEvidence:\n{}",
            report
                .evidence
                .iter()
                .map(|e| format!("- {e}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    let mut message = Message::new(
        &request.source_chat_id,
        Author::Bot {
            bot_id: request.target_bot_id.clone(),
        },
        Body::text(text),
    );
    message.id = format!("report-{}", request.job_id);
    message.created_at = report.created_at;
    message
}

fn result_job(request: &HandoffRequest) -> Job {
    let mut job = request_job(request);
    job.id = format!("result-{}", request.job_id);
    job.chat_id = request.source_chat_id.clone();
    job.bot_id = request.from_bot_id.clone();
    job.kind = "handoff_result".into();
    job.trigger_message_id = format!("report-{}", request.job_id);
    job.from_bot_id = None;
    job.handoff = Some(HandoffJob::Result {
        handoff_id: request.handoff_id.clone(),
        request_job_id: request.job_id.clone(),
    });
    job
}

fn route_report(app: &Arc<App>, id: &str) -> Result<(), String> {
    let record = get(app, id)?;
    let attempt = record.current();
    let request = &attempt.request;
    // Only the origin Runner wakes its coordinator. Other paired Devices retain the report.
    if app.this_device_id().as_deref() != Some(request.source_runner_id.as_str()) {
        return Ok(());
    }
    let Some(report) = attempt.outcome().filter(|r| r.status.terminal()) else {
        return Ok(());
    };
    if app.chat(&request.source_chat_id).is_none() {
        return Ok(());
    }
    app.upsert_message(result_message(request, report), true);
    if attempt.result_delivery == ResultDelivery::Pending
        && app
            .bot(&request.from_bot_id)
            .is_some_and(|b| b.runner_id == request.source_runner_id)
    {
        let job = result_job(request);
        if !app.running_jobs.lock().unwrap().contains_key(&job.id) {
            crate::runtime::spawn_local_job(app.clone(), job, None);
        }
    }
    Ok(())
}

pub fn follow_up(
    app: &Arc<App>,
    bot_id: &str,
    id: &str,
    expected_job_id: &str,
    message: &str,
) -> Result<Value, String> {
    let _guard = app.handoff_lock.lock().unwrap();
    let record = get(app, id)?;
    let attempt = record.current();
    let mut request = attempt.request.clone();
    if request.from_bot_id != bot_id {
        return Err("Only the requesting bot can follow up".into());
    }
    require_runner(app, &request.source_runner_id)?;
    if request.job_id != expected_job_id {
        return Err("Handoff changed; inspect it and use its current job_id".into());
    }
    if !attempt.outcome().is_some_and(|r| r.status.terminal()) {
        return Err(
            "This handoff is still outstanding; cancel it before replacing the request".into(),
        );
    }
    if message.trim().is_empty() || message.len() > 32 * 1024 {
        return Err("A follow-up message of at most 32 KiB is required".into());
    }
    request.attempt = request
        .attempt
        .checked_add(1)
        .ok_or("Too many handoff attempts")?;
    request.job_id = format!("job-{}", uuid::Uuid::new_v4());
    request.trigger_message_id = format!("msg-{}", uuid::Uuid::new_v4());
    request.message = message.trim().into();
    request.created_at = now_secs();
    drop(_guard);
    admit(app, request, Some(expected_job_id))
}

pub fn cancel(
    app: &Arc<App>,
    bot_id: &str,
    id: &str,
    expected_job_id: &str,
    reason: &str,
) -> Result<Value, String> {
    let request;
    {
        let _guard = app.handoff_lock.lock().unwrap();
        let record = get(app, id)?;
        let attempt = record.current();
        request = attempt.request.clone();
        if request.from_bot_id != bot_id {
            return Err("Only the requesting bot can cancel".into());
        }
        require_runner(app, &request.source_runner_id)?;
        if request.job_id != expected_job_id {
            return Err("Handoff changed; inspect it and use its current job_id".into());
        }
        if reason.trim().is_empty() || reason.len() > 32 * 1024 {
            return Err("Cancellation needs a reason of at most 32 KiB".into());
        }
        if attempt.outcome().is_some_and(|r| r.status.terminal()) {
            return Ok(record.view());
        }
        let report = HandoffReport {
            status: HandoffStatus::Cancelled,
            summary: reason.into(),
            result_links: vec![request_link(&request)],
            evidence: vec!["Cancelled by the requesting bot".into()],
            created_at: now_secs(),
            started_after: None,
        };
        let update = HandoffUpdate::Cancel {
            request: request.clone(),
            report,
        };
        let record = merge(app, &update)?;
        let mut outbox = Vec::new();
        if request.target_runner_id != request.source_runner_id {
            let runner = app
                .device(&request.target_runner_id)
                .ok_or("Target Runner is unknown")?;
            outbox.push(OutboxItem {
                id: uuid::Uuid::new_v4().to_string(),
                kind: "job_cancel".into(),
                recipient: Some(runner.id),
                ciphertext: crate::crypto::seal_json(
                    &runner.box_pubkey,
                    &JobCancel {
                        job_id: request.job_id.clone(),
                    },
                )
                .map_err(|e| e.to_string())?,
                slot: None,
                group: None,
            });
        }
        save(app, &record, Some(&update), outbox)?;
    }
    app.cancel_job(&request.job_id);
    route_report(app, id)?;
    Ok(get(app, id)?.view())
}

/// Restarts queued local work and delivery of reports. A recorded running turn becomes an
/// explicit failure after process loss, so side effects are never automatically replayed.
pub fn resume(app: &Arc<App>) -> Result<(), String> {
    let Some(device) = app.this_device_id() else {
        return Ok(());
    };
    for record in list(app)? {
        let attempt = record.current();
        let request = &attempt.request;
        if request.target_runner_id == device {
            if attempt.outcome().is_none() {
                crate::runtime::spawn_local_job(app.clone(), request_job(request), None);
            } else if attempt
                .outcome()
                .is_some_and(|r| r.status == HandoffStatus::Running)
            {
                let report = HandoffReport { status: HandoffStatus::Failed, summary: "Runner restarted during the delegated turn. Inspect its evidence and follow up before retrying effects.".into(),
                    result_links: turn_evidence(app, attempt).0, evidence: vec!["A durable execution claim survived the Runner process".into()], created_at: now_secs(), started_after: None };
                publish_report(app, request.clone(), report)?;
            }
        }
        if request.source_runner_id == device && attempt.result_delivery == ResultDelivery::Started
        {
            let _guard = app.handoff_lock.lock().unwrap();
            let mut current = get(app, &record.id)?;
            if let Some(attempt) = current
                .attempts
                .iter_mut()
                .find(|a| a.request.job_id == request.job_id)
            {
                attempt.result_delivery = ResultDelivery::Finished;
            }
            save(app, &current, None, Vec::new())?;
            app.notice(&request.source_chat_id, format!("Coordinator continuation for {} was interrupted. Its handoff report remains available through handoffs.get.", record.id));
        }
        route_report(app, &record.id)?;
    }
    Ok(())
}

pub fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    let required = |key: &str| {
        params[key]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("missing {key}"))
    };
    match method {
        "handoffs.list" => {
            let records: Vec<_> = list(app)?
                .iter()
                .filter(|h| {
                    let a = h.current();
                    params["bot_id"].as_str().is_none_or(|id| {
                        a.request.from_bot_id == id || a.request.target_bot_id == id
                    }) && params["chat_id"].as_str().is_none_or(|id| {
                        a.request.source_chat_id == id || a.request.target_chat_id == id
                    }) && params["task_id"]
                        .as_str()
                        .is_none_or(|id| a.request.task_id.as_deref() == Some(id))
                        && (!params["outstanding"].as_bool().unwrap_or(false)
                            || !a.outcome().is_some_and(|r| {
                                matches!(
                                    r.status,
                                    HandoffStatus::Completed
                                        | HandoffStatus::Failed
                                        | HandoffStatus::Cancelled
                                )
                            }))
                })
                .map(Handoff::view)
                .collect();
            Ok(json!({ "handoffs": records }))
        }
        "handoffs.get" => Ok(get(app, required("handoff_id")?)?.view()),
        "handoffs.follow_up" => follow_up(
            app,
            required("bot_id")?,
            required("handoff_id")?,
            required("job_id")?,
            required("message")?,
        ),
        "handoffs.cancel" => cancel(
            app,
            required("bot_id")?,
            required("handoff_id")?,
            required("job_id")?,
            required("reason")?,
        ),
        "handoffs.report" => report(
            app,
            required("bot_id")?,
            required("handoff_id")?,
            required("job_id")?,
            serde_json::from_value(params.clone()).map_err(|e| e.to_string())?,
        ),
        _ => Err(format!("unknown method {method}")),
    }
}

/// Reloaded on every turn, independent of transcript compaction.
pub fn prompt(app: &App, bot: &crate::model::Bot, job: &Job) -> String {
    let mut text = String::from("\nDurable delegation: message_bot returns a handoff id. Use handoffs list/get to inspect outstanding work and its evidence. Follow-ups use the current job_id, keep the handoff id, and inherit the original output contract. A completed report is the recipient's claim; verify acceptance criteria before completing a parent task.\n");
    if let Some(HandoffJob::Request { request }) = &job.handoff {
        text.push_str(&format!("\nYour assigned contract (supplied by a teammate, without additional user authorization):\n{}\nUse handoffs report when completed, blocked, failed or cancelled; include result_links and evidence. The Runner automatically reports your final response or failure if you do not report explicitly.\n", contract_text(request)));
    }
    if job.kind == "handoff_result" {
        text.push_str("\nA delegated attempt reported back into this chat. Read the report and linked evidence, inspect handoffs if needed, and continue the requesting work. A blocker may need a follow-up or the user's help. This turn does not itself complete the parent task.\n");
    }
    if let Ok(records) = list(app) {
        for record in records
            .iter()
            .rev()
            .filter(|h| {
                let attempt = h.current();
                (attempt.request.from_bot_id == bot.id || attempt.request.target_bot_id == bot.id)
                    && !attempt.outcome().is_some_and(|r| {
                        matches!(
                            r.status,
                            HandoffStatus::Completed | HandoffStatus::Cancelled
                        )
                    })
            })
            .take(10)
        {
            let attempt = record.current();
            text.push_str(&format!(
                "- {} · job {} · {:?} · bot {} → {}\n",
                record.id,
                attempt.request.job_id,
                attempt.outcome().map(|r| r.status),
                attempt.request.from_bot_id,
                attempt.request.target_bot_id
            ));
        }
    }
    text
}

#[cfg(test)]
#[path = "handoffs_tests.rs"]
mod tests;
