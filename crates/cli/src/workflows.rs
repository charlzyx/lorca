//! Guided marketplace workflows. Setup records are account-encrypted both in SQLite and
//! inside the roster; integrations remain existing Runner-local plugins.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::app::App;
use crate::config::now_secs;
use crate::marketplace::{self, BotTemplate, Index};
use crate::model::{
    Author, Body, Bot, Job, JobCancel, Message, MessageState, PluginStatus, Routine,
};
use crate::runtime::{self, TurnOutcome};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Question {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub placeholder: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Requirement {
    pub service_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Specialist {
    pub id: String,
    pub template_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackRoutine {
    pub id: String,
    pub specialist_id: String,
    pub name: String,
    pub schedule: String,
    pub prompt: String,
}

/// Optional, additive entries in the v1 index. Service requirements may arrive in a later
/// index; setup explains their absence and remains resumable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pack {
    pub id: String,
    pub name: String,
    pub outcome: String,
    pub description: String,
    #[serde(default = "pack_version")]
    pub version: u64,
    pub questions: Vec<Question>,
    #[serde(default)]
    pub connections: Vec<Requirement>,
    pub specialists: Vec<Specialist>,
    pub routines: Vec<PackRoutine>,
    pub sample_specialist: String,
    pub sample_prompt: String,
}

fn pack_version() -> u64 {
    1
}

impl Pack {
    pub fn parse(value: &Value, index: &Index) -> Result<Self, String> {
        let pack: Self = serde_json::from_value(value.clone())
            .map_err(|e| format!("Not a workflow pack: {e}"))?;
        if pack.version != 1
            || !crate::plugins::is_id(&pack.id)
            || [
                &pack.name,
                &pack.outcome,
                &pack.description,
                &pack.sample_prompt,
            ]
            .iter()
            .any(|s| s.trim().is_empty())
        {
            return Err("A workflow pack needs a supported version, id, name, outcome, description and sample.".into());
        }
        if pack.specialists.is_empty()
            || pack.specialists.len() > 6
            || pack.questions.len() > 12
            || pack.connections.len() > 12
            || pack.routines.len() > 20
        {
            return Err("A workflow pack exceeds its setup limits.".into());
        }
        unique_ids(pack.questions.iter().map(|q| q.id.as_str()))?;
        unique_ids(pack.specialists.iter().map(|s| s.id.as_str()))?;
        unique_ids(pack.connections.iter().map(|c| c.service_id.as_str()))?;
        unique_ids(pack.routines.iter().map(|r| r.id.as_str()))?;
        for question in &pack.questions {
            if question.label.trim().is_empty() {
                return Err("A setup question needs a label.".into());
            }
        }
        for connection in &pack.connections {
            if connection.name.trim().is_empty() {
                return Err("A connection requirement needs a name.".into());
            }
        }
        for specialist in &pack.specialists {
            if index.bot(&specialist.template_id).is_none() {
                return Err(format!(
                    "Unknown specialist template {}",
                    specialist.template_id
                ));
            }
        }
        if !pack
            .specialists
            .iter()
            .any(|s| s.id == pack.sample_specialist)
        {
            return Err("Unknown sample specialist.".into());
        }
        for routine in &pack.routines {
            if !pack
                .specialists
                .iter()
                .any(|s| s.id == routine.specialist_id)
            {
                return Err("Unknown routine specialist.".into());
            }
            if routine.name.trim().is_empty()
                || routine.name.chars().count() > crate::routines::MAX_NAME_CHARS
                || routine.prompt.trim().is_empty()
            {
                return Err("A workflow routine needs a name and prompt.".into());
            }
            crate::schedule::parse(&routine.schedule)?;
        }
        Ok(pack)
    }
}

fn unique_ids<'a>(ids: impl Iterator<Item = &'a str>) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if !crate::plugins::is_id(id) || !seen.insert(id) {
            return Err(format!("Invalid or repeated workflow id {id:?}"));
        }
    }
    Ok(())
}

/// Only ciphertext and merge metadata live in the local setup table and roster extension.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Envelope {
    pub id: String,
    pub updated_at: f64,
    pub ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sample {
    pub job_id: String,
    pub chat_id: String,
    pub bot_id: String,
    pub started_at: f64,
    /// running, ready, failed, reviewed. A cancelled generation is never reviewed.
    pub state: String,
    #[serde(default)]
    pub message_ids: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Setup {
    pub id: String,
    pub runner_id: String,
    /// The chosen pack and profiles are pinned so a marketplace update cannot change a
    /// partially completed setup's questions, instructions or schedules.
    pub pack: Pack,
    pub templates: BTreeMap<String, BotTemplate>,
    pub answers: BTreeMap<String, String>,
    pub bot_ids: BTreeMap<String, String>,
    pub routine_ids: BTreeMap<String, String>,
    pub owned_routine_ids: Vec<String>,
    /// References to Installed only, with explicit selections for named accounts.
    pub connection_ids: BTreeMap<String, String>,
    /// questions, connections, sample, reviewed, enabled, cancelled.
    pub phase: String,
    pub sample: Option<Sample>,
}

fn stable_id(prefix: &str, parts: &[&str]) -> String {
    let hash = Sha256::digest(serde_json::to_vec(parts).unwrap());
    format!(
        "{prefix}-{}",
        hash[..16]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

fn decode(app: &App, envelope: &Envelope) -> Result<Setup, String> {
    let dek = app.dek().ok_or("Create or pair an identity first.")?;
    let bytes = URL_SAFE_NO_PAD
        .decode(&envelope.ciphertext)
        .map_err(|e| e.to_string())?;
    let setup: Setup =
        crate::crypto::decrypt_json(&dek, &format!("workflow:{}", envelope.id), &bytes)
            .map_err(|e| e.to_string())?;
    if setup.id != envelope.id {
        return Err("Workflow setup identity does not match its envelope.".into());
    }
    Ok(setup)
}

fn get(app: &App, id: &str) -> Result<Setup, String> {
    let envelope = app
        .state
        .lock()
        .unwrap()
        .workflows
        .iter()
        .find(|w| w.id == id)
        .cloned()
        .ok_or("Unknown workflow setup.")?;
    decode(app, &envelope)
}

/// Strict persistence: report a failed database write, leaving the previous record available
/// for a retry. The outbox follows the encrypted roster path.
fn save(app: &App, setup: &Setup) -> Result<(), String> {
    persist(app, setup, None)
}

fn persist(app: &App, setup: &Setup, sample_job: Option<&str>) -> Result<(), String> {
    let dek = app.dek().ok_or("Create or pair an identity first.")?;
    let ciphertext = crate::crypto::encrypt_json(&dek, &format!("workflow:{}", setup.id), setup)
        .map_err(|e| e.to_string())?;
    let mut state = app.state.lock().unwrap();
    if let Some(job_id) = sample_job {
        let Some(held) = state.workflows.iter().find(|e| e.id == setup.id) else {
            return Ok(());
        };
        let bytes = URL_SAFE_NO_PAD
            .decode(&held.ciphertext)
            .map_err(|e| e.to_string())?;
        let current: Setup =
            crate::crypto::decrypt_json(&dek, &format!("workflow:{}", held.id), &bytes)
                .map_err(|e| e.to_string())?;
        if current.phase == "cancelled"
            || current
                .sample
                .as_ref()
                .is_none_or(|s| s.job_id != job_id || s.state != "running")
        {
            return Ok(());
        }
    }
    let previous = state.workflows.clone();
    let updated_at = now_secs().max(
        previous
            .iter()
            .find(|e| e.id == setup.id)
            .map_or(0.0, |e| e.updated_at + 0.000001),
    );
    let envelope = Envelope {
        id: setup.id.clone(),
        updated_at,
        ciphertext: URL_SAFE_NO_PAD.encode(ciphertext),
    };
    if let Some(held) = state.workflows.iter_mut().find(|e| e.id == setup.id) {
        *held = envelope;
    } else {
        state.workflows.push(envelope);
    }
    if let Err(error) = app.store.save_state(&state) {
        state.workflows = previous;
        return Err(format!("Could not save workflow progress: {error}"));
    }
    drop(state);
    app.push_roster();
    app.emit(app.roster_summary());
    Ok(())
}

/// Preserve records omitted by an older client; cancellations remain records, so sync cannot
/// resurrect a cancelled setup. Different packs merge independently.
pub fn merge(current: &mut Vec<Envelope>, incoming: Option<Vec<Envelope>>) -> bool {
    let Some(incoming) = incoming else {
        return !current.is_empty();
    };
    let mut republish = false;
    for envelope in &incoming {
        match current.iter_mut().find(|held| held.id == envelope.id) {
            Some(held) if envelope.updated_at > held.updated_at => *held = envelope.clone(),
            Some(held) if envelope.updated_at < held.updated_at => republish = true,
            Some(_) => {}
            None => current.push(envelope.clone()),
        }
    }
    republish
        || current
            .iter()
            .any(|held| !incoming.iter().any(|e| e.id == held.id))
}

fn all(app: &App) -> Result<Vec<Setup>, String> {
    let envelopes = app.state.lock().unwrap().workflows.clone();
    envelopes.iter().map(|e| decode(app, e)).collect()
}

fn str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Missing {key}."))
}

fn running(app: &App, sample: &Sample) -> bool {
    app.running_turns()
        .iter()
        .any(|turn| turn["job_id"].as_str() == Some(&sample.job_id))
}

fn guard_edit(app: &App, setup: &Setup) -> Result<(), String> {
    if setup.sample.as_ref().is_some_and(|s| {
        s.state == "running"
            && (running(app, s)
                || (setup.runner_id != app.this_device_id().unwrap_or_default()
                    && now_secs() - s.started_at < 300.0))
    }) {
        return Err(
            "The sample is running. Cancel setup before changing its answers or accounts.".into(),
        );
    }
    if setup.phase == "enabled" {
        return Err("Cancel setup to pause its schedules before changing it.".into());
    }
    Ok(())
}

pub async fn handle(app: &Arc<App>, method: &str, params: &Value) -> Result<Value, String> {
    // One materialization/install per local CLI; stable IDs also recover interrupted steps.
    let _editing = app.workflow_editing.lock().await;
    if method == "workflows.start" {
        let runner_id = str_param(params, "runner_id")?;
        let runner = app
            .device(runner_id)
            .filter(|d| d.is_runner())
            .ok_or("Choose a Runner for this workflow.")?;
        let pack_id = str_param(params, "pack_id")?;
        let id = stable_id("workflow", &[pack_id, runner_id]);
        if let Ok(mut setup) = get(app, &id) {
            if setup.phase == "cancelled" {
                setup.phase = if setup.bot_ids.is_empty() {
                    "questions"
                } else {
                    "connections"
                }
                .into();
                save(app, &setup)?;
            }
            return view(app, &setup);
        }
        // Existing corrupt progress is an error, not permission to create resources again.
        if app
            .state
            .lock()
            .unwrap()
            .workflows
            .iter()
            .any(|e| e.id == id)
        {
            return Err("Workflow progress could not be decrypted.".into());
        }
        let index = marketplace::index(app).await;
        let pack = index
            .pack(pack_id)
            .cloned()
            .ok_or("This workflow is no longer in the marketplace.")?;
        let templates = pack
            .specialists
            .iter()
            .map(|s| (s.id.clone(), index.bot(&s.template_id).unwrap().clone()))
            .collect();
        let setup = Setup {
            id,
            runner_id: runner.id,
            pack,
            templates,
            answers: BTreeMap::new(),
            bot_ids: BTreeMap::new(),
            routine_ids: BTreeMap::new(),
            owned_routine_ids: Vec::new(),
            connection_ids: BTreeMap::new(),
            phase: "questions".into(),
            sample: None,
        };
        save(app, &setup)?;
        return view(app, &setup);
    }
    let id = str_param(params, "id")?;
    let mut setup = get(app, id)?;
    let mut installed_status: Option<Value> = None;
    match method {
        "workflows.get" => {}
        "workflows.configure" => {
            guard_edit(app, &setup)?;
            let answers: BTreeMap<String, String> =
                serde_json::from_value(params["answers"].clone())
                    .map_err(|_| "Pass the workflow's answers as text fields.")?;
            validate_answers(&setup.pack, &answers)?;
            let selected: BTreeMap<String, String> =
                serde_json::from_value(params.get("bot_ids").cloned().unwrap_or_else(|| json!({})))
                    .map_err(|_| "Pass selected bot IDs by specialist.")?;
            materialize(app, &mut setup, answers, selected)?;
            save(app, &setup)?;
        }
        "workflows.connection" => {
            guard_edit(app, &setup)?;
            if setup.bot_ids.is_empty() {
                return Err("Answer this workflow's questions first.".into());
            }
            let service_id = str_param(params, "service_id")?;
            if !setup
                .pack
                .connections
                .iter()
                .any(|c| c.service_id == service_id)
            {
                return Err("This workflow does not require that integration.".into());
            }
            let choices = choices(app, &setup.runner_id, service_id);
            let explicit = params["plugin_id"].as_str();
            let plugin_id = if let Some(id) = explicit {
                if !choices.iter().any(|p| p.id == id) {
                    return Err(
                        "Select an account for this service on the workflow's Runner.".into(),
                    );
                }
                id.to_string()
            } else if let Some(id) = setup.connection_ids.get(service_id) {
                id.clone()
            } else {
                // A lost installation response or pre-existing account is recoverable by
                // selecting its advertised instance; never infer a default named account.
                if !choices.is_empty() {
                    return Err("Choose an existing account before adding another.".into());
                }
                let index = marketplace::index(app).await;
                let manifest = index.plugin(service_id).cloned().ok_or_else(|| format!("{} is not available in this marketplace yet. Your setup is saved; retry after updating the marketplace.", service_id))?;
                let status = crate::plugins::on_runner(app, &setup.runner_id, "plugins.install", json!({ "manifest": manifest, "source": "marketplace", "account_name": params["account_name"].as_str().filter(|name| !name.trim().is_empty()).unwrap_or(&setup.pack.name) })).await?;
                let id = str_param(&status, "id")?.to_string();
                installed_status = Some(status);
                id
            };
            if setup.connection_ids.get(service_id) != Some(&plugin_id) {
                for id in &setup.owned_routine_ids {
                    if app.routine(id).is_some() {
                        crate::routines::set_enabled(app, id, false)?;
                    }
                }
                setup.sample = None;
                setup.phase = "connections".into();
            }
            setup.connection_ids.insert(service_id.into(), plugin_id);
            save(app, &setup)?;
        }
        "workflows.clear_connection" => {
            guard_edit(app, &setup)?;
            let service_id = str_param(params, "service_id")?;
            if setup.connection_ids.remove(service_id).is_none() {
                return Err("No selected account to clear.".into());
            }
            for id in &setup.owned_routine_ids {
                if app.routine(id).is_some() {
                    crate::routines::set_enabled(app, id, false)?;
                }
            }
            setup.sample = None;
            setup.phase = "connections".into();
            save(app, &setup)?;
        }
        "workflows.sample" => {
            if setup.phase == "cancelled" {
                return Err("Resume this workflow before running a sample.".into());
            }
            if let Some(sample) = &setup.sample {
                if sample.state == "running"
                    && (running(app, sample)
                        || (setup.runner_id != app.this_device_id().unwrap_or_default()
                            && now_secs() - sample.started_at < 300.0))
                {
                    return view(app, &setup);
                }
            }
            ready(app, &setup)?;
            for id in &setup.owned_routine_ids {
                if app.routine(id).is_some() {
                    crate::routines::set_enabled(app, id, false)?;
                }
            }
            let bot_id = setup
                .bot_ids
                .get(&setup.pack.sample_specialist)
                .ok_or("Set up the sample specialist first.")?
                .clone();
            let dm = app.dm_with(&bot_id, None).map_err(|e| e.to_string())?;
            let message = Message::new(&dm.meta.id, Author::You, Body::text(format!("Run a sample of {} for me to review. {}\nKeep its schedules paused. Present a draft in this chat; ask no question about enabling schedules.", setup.pack.name, setup.pack.sample_prompt)));
            let job = Job {
                id: format!("job-{}", uuid::Uuid::new_v4()),
                chat_id: dm.meta.id.clone(),
                bot_id: bot_id.clone(),
                kind: "workflow_sample".into(),
                trigger_message_id: message.id.clone(),
                routine_id: None,
                check: None,
                requested_by: app.this_device_id().unwrap_or_default(),
                from_bot_id: None,
                hops: 0,
                round: 0,
                is_winding_down: false,
                setup: None,
                created_at: now_secs(),
            };
            setup.sample = Some(Sample {
                job_id: job.id.clone(),
                chat_id: dm.meta.id,
                bot_id,
                started_at: job.created_at,
                state: "running".into(),
                message_ids: Vec::new(),
                error: None,
            });
            setup.phase = "sample".into();
            save(app, &setup)?;
            app.upsert_message(message, true);
            runtime::start_turn(app, job);
        }
        "workflows.review" => {
            if setup.phase == "cancelled" {
                return Err("Resume setup and run a sample first.".into());
            }
            ready(app, &setup)?;
            let job_id = str_param(params, "job_id")?;
            let sample = setup.sample.as_mut().ok_or("Run a sample first.")?;
            if sample.job_id != job_id
                || !matches!(sample.state.as_str(), "ready" | "reviewed")
                || sample.message_ids.is_empty()
            {
                return Err("Review the completed result of the current sample first.".into());
            }
            if sample
                .message_ids
                .iter()
                .any(|id| app.message(&sample.chat_id, id).is_none())
            {
                return Err(
                    "The sample result has not reached this Device yet. Retry after sync.",
                )?;
            }
            sample.state = "reviewed".into();
            setup.phase = "reviewed".into();
            save(app, &setup)?;
        }
        "workflows.enable" => {
            ready(app, &setup)?;
            if setup.phase == "cancelled"
                || setup.sample.as_ref().is_none_or(|s| s.state != "reviewed")
            {
                return Err("Run and review a sample before enabling schedules.".into());
            }
            for id in setup.routine_ids.values() {
                crate::routines::set_enabled(app, id, true)?;
            }
            setup.phase = "enabled".into();
            save(app, &setup)?;
        }
        "workflows.cancel" => {
            if let Some(sample) = &setup.sample {
                app.cancel_job(&sample.job_id);
                if setup.runner_id != app.this_device_id().unwrap_or_default() {
                    if let Some(runner) = app.device(&setup.runner_id) {
                        let ciphertext = crate::crypto::seal_json(
                            &runner.box_pubkey,
                            &JobCancel {
                                job_id: sample.job_id.clone(),
                            },
                        )
                        .map_err(|e| e.to_string())?;
                        app.push_blob("job_cancel", Some(runner.id), ciphertext);
                    }
                }
            }
            // Only pack-created routines are paused. A reused user's routine stays as it was.
            for id in &setup.owned_routine_ids {
                if app.routine(id).is_some() {
                    crate::routines::set_enabled(app, id, false)?;
                }
            }
            setup.phase = "cancelled".into();
            setup.sample = None;
            save(app, &setup)?;
        }
        _ => return Err(format!("Unknown workflow method {method}")),
    }
    let mut out = view(app, &setup)?;
    if let Some(status) = installed_status {
        // Return the Runner's acknowledgment immediately. The authoritative machine
        // advertisement can arrive later; this is a transient response, not another store.
        if let Some(connection) = out["connections"]
            .as_array_mut()
            .and_then(|rows| rows.iter_mut().find(|c| c["selected_id"] == status["id"]))
        {
            connection["state"] = status["state"].clone();
            connection["detail"] = status["detail"].clone();
            connection["choices"].as_array_mut().unwrap().push(status);
        }
    }
    Ok(out)
}

fn validate_answers(pack: &Pack, answers: &BTreeMap<String, String>) -> Result<(), String> {
    if answers
        .keys()
        .any(|id| !pack.questions.iter().any(|q| &q.id == id))
    {
        return Err("Pass only answers this workflow asks for.".into());
    }
    for question in &pack.questions {
        let answer = answers
            .get(&question.id)
            .map(String::as_str)
            .unwrap_or("")
            .trim();
        if answer.is_empty() {
            return Err(format!("Answer {}.", question.label));
        }
        if answer.chars().count() > 2000 {
            return Err(format!("Keep {} under 2,000 characters.", question.label));
        }
        if crate::memory::scrub(answer) != answer {
            return Err("Keep credentials in the integration's sign-in or setup fields.")?;
        }
    }
    Ok(())
}

fn materialize(
    app: &Arc<App>,
    setup: &mut Setup,
    answers: BTreeMap<String, String>,
    selected: BTreeMap<String, String>,
) -> Result<(), String> {
    if selected
        .keys()
        .any(|role| !setup.templates.contains_key(role))
    {
        return Err("Unknown workflow specialist.".into());
    }
    // Validate every explicit choice before creating resources.
    for id in selected.values() {
        if app.bot(id).is_none_or(|b| b.runner_id != setup.runner_id) {
            return Err("Reuse a bot on the chosen Runner.")?;
        }
    }
    let changed = setup.answers != answers
        || selected
            .iter()
            .any(|(role, id)| setup.bot_ids.get(role) != Some(id));
    if changed {
        for id in &setup.owned_routine_ids {
            if app.routine(id).is_some() {
                crate::routines::set_enabled(app, id, false)?;
            }
        }
        setup.sample = None;
    }
    setup.answers = answers
        .into_iter()
        .map(|(k, v)| (k, v.trim().to_string()))
        .collect();
    for specialist in &setup.pack.specialists {
        let template = &setup.templates[&specialist.id];
        let stable = stable_id("bot-workflow", &[&setup.id, &specialist.id]);
        let held = selected
            .get(&specialist.id)
            .or_else(|| setup.bot_ids.get(&specialist.id))
            .and_then(|id| app.bot(id))
            .filter(|b| b.runner_id == setup.runner_id);
        let suitable = app
            .state
            .lock()
            .unwrap()
            .bots
            .iter()
            .find(|b| b.runner_id == setup.runner_id && b.description == template.description)
            .cloned();
        let bot = if let Some(bot) = held
            .or_else(|| app.bot(&stable).filter(|b| b.runner_id == setup.runner_id))
            .or(suitable)
        {
            bot
        } else {
            if app.bot(&stable).is_some() {
                return Err("The imported specialist moved to another Runner. Select another bot for this workflow.".into());
            }
            let provider = app
                .credentials
                .lock()
                .unwrap()
                .statuses()
                .iter()
                .find(|p| p.is_connected)
                .map(|p| p.kind.clone())
                .unwrap_or_else(|| "deepseek".into());
            let bot = Bot {
                id: stable.clone(),
                name: template.name.clone(),
                description: template.description.clone(),
                symbol_name: template.symbol_name.clone(),
                accent: template.accent.clone(),
                avatar: None,
                runner_id: setup.runner_id.clone(),
                provider,
                model: None,
                thinking: None,
                legacy_instructions: String::new(),
                workdir: None,
                created_at: 0.0,
            };
            app.create_bot_with_dm(bot, Some(stable_id("chat-workflow", &[&stable])))
                .map_err(|e| e.to_string())?
                .0
        };
        setup.bot_ids.insert(specialist.id.clone(), bot.id);
    }
    for spec in &setup.pack.routines {
        let bot_id = &setup.bot_ids[&spec.specialist_id];
        let stable = stable_id("routine-workflow", &[&setup.id, &spec.id, bot_id]);
        let schedule = crate::schedule::parse(&spec.schedule)?.canonical();
        let held = setup
            .routine_ids
            .get(&spec.id)
            .and_then(|id| app.routine(id))
            .filter(|r| &r.bot_id == bot_id && r.schedule == schedule && r.prompt == spec.prompt);
        let suitable = app
            .routines_of(bot_id)
            .into_iter()
            .find(|r| r.name == spec.name && r.schedule == schedule && r.prompt == spec.prompt);
        let recovered = if let Some(routine) = app.routine(&stable) {
            if routine.prompt != spec.prompt
                || routine.schedule != schedule
                || &routine.bot_id != bot_id
            {
                setup.sample = None;
                Some(
                    app.update_routine(&stable, |r| {
                        r.bot_id = bot_id.clone();
                        r.prompt = spec.prompt.clone();
                        r.schedule = schedule.clone();
                        r.is_enabled = false;
                    })
                    .map_err(|e| e.to_string())?,
                )
            } else {
                Some(routine)
            }
        } else {
            None
        };
        let routine = if let Some(routine) = held.or(recovered).or(suitable) {
            routine
        } else {
            if app.routines_of(bot_id).len() >= crate::routines::MAX_PER_BOT {
                return Err(
                    "The selected bot has no room for another routine. Choose another specialist.",
                )?;
            }
            let now = now_secs();
            let routine = Routine {
                id: stable.clone(),
                bot_id: bot_id.clone(),
                name: spec.name.clone(),
                prompt: spec.prompt.clone(),
                schedule,
                is_enabled: false,
                enabled_at: now,
                last_run_at: None,
                last_outcome: None,
                paused_reason: None,
                check: None,
                created_at: now,
            };
            let routine = app.insert_routine(routine).map_err(|e| e.to_string())?;
            setup.owned_routine_ids.push(routine.id.clone());
            routine
        };
        // Recover a crash between the deterministic insert and the encrypted setup write.
        if routine.id == stable && !setup.owned_routine_ids.contains(&stable) {
            setup.owned_routine_ids.push(stable);
        }
        setup.routine_ids.insert(spec.id.clone(), routine.id);
    }
    setup.phase = "connections".into();
    Ok(())
}

fn choices(app: &App, runner_id: &str, service_id: &str) -> Vec<PluginStatus> {
    app.device(runner_id)
        .map(|d| {
            d.plugins
                .into_iter()
                .filter(|p| {
                    p.source.is_none() && p.service_id.as_deref().unwrap_or(&p.id) == service_id
                })
                .collect()
        })
        .unwrap_or_default()
}

fn ready(app: &App, setup: &Setup) -> Result<(), String> {
    validate_answers(&setup.pack, &setup.answers)?;
    for specialist in &setup.pack.specialists {
        let bot = setup
            .bot_ids
            .get(&specialist.id)
            .and_then(|id| app.bot(id))
            .filter(|b| b.runner_id == setup.runner_id)
            .ok_or("A specialist was removed or moved. Rerun setup to choose its replacement.")?;
        if !app
            .credentials
            .lock()
            .unwrap()
            .statuses()
            .iter()
            .any(|p| p.kind == bot.provider && p.is_connected)
        {
            return Err(format!(
                "Connect {} in Settings for {} before running a sample.",
                bot.provider, bot.name
            ));
        }
    }
    for spec in &setup.pack.routines {
        let routine = setup
            .routine_ids
            .get(&spec.id)
            .and_then(|id| app.routine(id))
            .ok_or("A routine was removed. Rerun setup to restore it.")?;
        if setup.bot_ids.get(&spec.specialist_id) != Some(&routine.bot_id)
            || routine.prompt != spec.prompt
            || routine.schedule != crate::schedule::parse(&spec.schedule)?.canonical()
        {
            return Err(
                "A routine changed since setup. Rerun setup and review another sample.".into(),
            );
        }
    }
    for requirement in &setup.pack.connections {
        let id = setup
            .connection_ids
            .get(&requirement.service_id)
            .ok_or_else(|| format!("Choose a {} account.", requirement.name))?;
        if !choices(app, &setup.runner_id, &requirement.service_id)
            .iter()
            .any(|p| &p.id == id && p.state == "ready")
        {
            return Err(format!(
                "Finish connecting {} on the selected Runner.",
                requirement.name
            ));
        }
    }
    Ok(())
}

fn view(app: &App, setup: &Setup) -> Result<Value, String> {
    let index = marketplace::current(app);
    let connections: Vec<_> = setup.pack.connections.iter().map(|r| {
        let choices = choices(app, &setup.runner_id, &r.service_id);
        let selected_id = setup.connection_ids.get(&r.service_id);
        let status = selected_id.and_then(|id| choices.iter().find(|p| &p.id == id));
        json!({ "service_id": r.service_id, "name": r.name, "selected_id": selected_id, "choices": choices, "available": index.plugin(&r.service_id).is_some(), "state": status.map(|p| p.state.as_str()).unwrap_or("missing"), "detail": status.map(|p| p.detail.as_str()).unwrap_or("Choose or add an account on this Runner.") })
    }).collect();
    let specialists: Vec<_> = setup.pack.specialists.iter().map(|s| {
        let candidates: Vec<Bot> = app.state.lock().unwrap().bots.iter().filter(|b| b.runner_id == setup.runner_id).cloned().collect();
        json!({ "id": s.id, "name": setup.templates[&s.id].name, "selected_id": setup.bot_ids.get(&s.id), "choices": candidates })
    }).collect();
    let routines: Vec<_> = setup
        .routine_ids
        .values()
        .filter_map(|id| app.routine(id))
        .map(|r| app.routine_out(&r))
        .collect();
    let sample_messages: Vec<_> = setup
        .sample
        .as_ref()
        .into_iter()
        .flat_map(|s| {
            s.message_ids
                .iter()
                .filter_map(|id| app.message(&s.chat_id, id))
        })
        .map(|m| m.for_app())
        .collect();
    let blocked = ready(app, setup).err();
    let is_running = setup.sample.as_ref().is_some_and(|s| {
        s.state == "running"
            && (running(app, s)
                || (setup.runner_id != app.this_device_id().unwrap_or_default()
                    && now_secs() - s.started_at < 300.0))
    });
    Ok(
        json!({ "setup": setup, "connections": connections, "specialists": specialists, "routines": routines, "sample_messages": sample_messages, "is_running": is_running, "can_sample": blocked.is_none() && !is_running && setup.phase != "cancelled", "can_enable": blocked.is_none() && setup.phase != "cancelled" && setup.sample.as_ref().is_some_and(|s| s.state == "reviewed"), "blocked_reason": blocked }),
    )
}

/// Set the result boundary only after obtaining the chat lock, so replies from a preceding
/// queued turn cannot become part of this sample's result.
pub fn sample_started(app: &App, job: &Job) {
    if job.kind != "workflow_sample" {
        return;
    }
    let Ok(setups) = all(app) else { return };
    if let Some(mut setup) = setups.into_iter().find(|s| {
        s.sample
            .as_ref()
            .is_some_and(|sample| sample.job_id == job.id)
    }) {
        setup.sample.as_mut().unwrap().started_at = now_secs();
        if let Err(error) = persist(app, &setup, Some(&job.id)) {
            tracing::error!(%error, "saving workflow sample boundary");
        }
    }
}

/// Called on the executing Runner while the chat's turn lock is still held. Read the current
/// generation before recording a result; a cancelled or retried generation cannot activate it.
pub fn sample_finished(app: &App, job: &Job, outcome: TurnOutcome) {
    if job.kind != "workflow_sample" {
        return;
    }
    let Ok(setups) = all(app) else { return };
    let Some(mut setup) = setups.into_iter().find(|s| {
        s.phase != "cancelled"
            && s.sample
                .as_ref()
                .is_some_and(|sample| sample.job_id == job.id && sample.state == "running")
    }) else {
        return;
    };
    let sample = setup.sample.as_mut().unwrap();
    let messages = app
        .store
        .page(&job.chat_id, None, 100)
        .map(|p| p.0)
        .unwrap_or_default();
    sample.message_ids = messages
        .iter()
        .filter(|m| {
            m.created_at >= sample.started_at
                && m.author
                    == Author::Bot {
                        bot_id: job.bot_id.clone(),
                    }
                && matches!(&m.body, Body::Text { text, .. } if !text.trim().is_empty())
                && m.state == MessageState::Complete
        })
        .map(|m| m.id.clone())
        .collect();
    sample.state = if outcome == TurnOutcome::Sent && !sample.message_ids.is_empty() {
        "ready"
    } else {
        "failed"
    }
    .into();
    if sample.state == "failed" {
        sample.error = Some("The sample did not produce a completed result. Review its chat, fix the connection or provider, and try again.".into());
    }
    if let Err(error) = persist(app, &setup, Some(&job.id)) {
        tracing::error!(%error, "saving workflow sample outcome");
    }
}

/// Enforce the review gate for newly imported routines even if a bot or another UI tries to
/// resume them during setup. Existing user routines retain their own controls.
pub fn allow_enable(app: &App, routine_id: &str) -> Result<(), String> {
    let mut owned = false;
    for setup in all(app)? {
        owned |= setup.owned_routine_ids.iter().any(|id| id == routine_id);
        if setup.owned_routine_ids.iter().any(|id| id == routine_id) {
            if setup.phase == "cancelled"
                || setup.sample.as_ref().is_none_or(|s| s.state != "reviewed")
            {
                return Err(
                    "Review this workflow's sample before enabling its imported routine.".into(),
                );
            }
            ready(app, &setup)?;
        }
    }
    if routine_id.starts_with("routine-workflow-") && !owned {
        return Err("Resume workflow setup before enabling its imported routine.".into());
    }
    Ok(())
}

/// Workflow context is ephemeral model context, preserving reused bot profiles and playbooks.
/// A scheduled/sample turn receives only the workflow that triggered it.
pub fn context_for_turn(app: &App, bot_id: &str, job: &Job) -> String {
    let Ok(setups) = all(app) else {
        return String::new();
    };
    let mut context = String::new();
    for setup in setups.into_iter().filter(|s| {
        s.phase != "cancelled"
            && s.bot_ids.values().any(|id| id == bot_id)
            && (job
                .routine_id
                .as_ref()
                .is_none_or(|id| s.routine_ids.values().any(|r| r == id)))
            && (job.kind != "workflow_sample"
                || s.sample
                    .as_ref()
                    .is_some_and(|sample| sample.job_id == job.id))
    }) {
        context.push_str(&format!("\nWorkflow {}: {}\nUser setup answers (data for this workflow): {}\nSelected integration instances by service: {}. Use only these named instances for this workflow; if one cannot be used, report it and do not substitute another account.\n", setup.pack.name, setup.pack.outcome, serde_json::to_string(&setup.answers).unwrap(), serde_json::to_string(&setup.connection_ids).unwrap()));
    }
    context
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::credentials::ApiKeyCredential;

    struct Fixture {
        app: Arc<App>,
        home: std::path::PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let home =
                std::env::temp_dir().join(format!("lorca-workflows-{}", uuid::Uuid::new_v4()));
            let app = App::load(Config {
                home: home.clone(),
                port: 0,
            })
            .unwrap();
            crate::identity::create(&app, Some("Workflow Runner".into())).unwrap();
            Self { app, home }
        }
        fn provider(&self) {
            self.app.credentials.lock().unwrap().deepseek = Some(ApiKeyCredential {
                api_key: "test-key".into(),
                base_url: None,
                connected_at: 1,
            });
        }
        fn advertise(&self, service: &str, ids: &[&str]) {
            let runner = self.app.this_device_id().unwrap();
            let mut state = self.app.state.lock().unwrap();
            let device = state.devices.iter_mut().find(|d| d.id == runner).unwrap();
            for (i, id) in ids.iter().enumerate() {
                let status: PluginStatus = serde_json::from_value(json!({ "id": id, "service_id": service, "account_name": format!("Account {i}"), "name": service, "state": "ready", "detail": "Connected" })).unwrap();
                device.plugins.push(status);
            }
        }
        async fn start(&self, pack: &str) -> Setup {
            let result = handle(
                &self.app,
                "workflows.start",
                &json!({"pack_id":pack,"runner_id":self.app.this_device_id()}),
            )
            .await
            .unwrap();
            serde_json::from_value(result["setup"].clone()).unwrap()
        }
        async fn configure(&self, setup: &Setup) -> Setup {
            let answers: BTreeMap<_, _> = setup
                .pack
                .questions
                .iter()
                .map(|q| (q.id.clone(), format!("scope-{}", q.id)))
                .collect();
            let result = handle(
                &self.app,
                "workflows.configure",
                &json!({"id":setup.id,"answers":answers}),
            )
            .await
            .unwrap();
            serde_json::from_value(result["setup"].clone()).unwrap()
        }
        async fn repository(&self) -> Setup {
            self.provider();
            self.advertise("github", &["github"]);
            let setup = self
                .configure(&self.start("repository-monitoring").await)
                .await;
            let result = handle(
                &self.app,
                "workflows.connection",
                &json!({"id":setup.id,"service_id":"github","plugin_id":"github"}),
            )
            .await
            .unwrap();
            serde_json::from_value(result["setup"].clone()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }

    fn preview(app: &App, setup: &mut Setup, job_id: &str) -> Job {
        let bot_id = setup.bot_ids[&setup.pack.sample_specialist].clone();
        let dm = app.dm_with(&bot_id, None).unwrap();
        let job = Job {
            id: job_id.into(),
            chat_id: dm.meta.id.clone(),
            bot_id: bot_id.clone(),
            kind: "workflow_sample".into(),
            trigger_message_id: "sample-trigger".into(),
            routine_id: None,
            check: None,
            requested_by: app.this_device_id().unwrap(),
            from_bot_id: None,
            hops: 0,
            round: 0,
            is_winding_down: false,
            setup: None,
            created_at: now_secs() - 1.0,
        };
        setup.sample = Some(Sample {
            job_id: job.id.clone(),
            chat_id: dm.meta.id,
            bot_id,
            started_at: job.created_at,
            state: "running".into(),
            message_ids: Vec::new(),
            error: None,
        });
        setup.phase = "sample".into();
        save(app, setup).unwrap();
        job
    }

    #[test]
    fn packs_validate_references_ids_versions_and_schedules() {
        let index = marketplace::bundled();
        assert_eq!(index.packs.len(), 3);
        for pack in &index.packs {
            assert!(Pack::parse(&serde_json::to_value(pack).unwrap(), &index).is_ok());
        }
        let original = serde_json::to_value(&index.packs[0]).unwrap();
        for (field, value) in [
            ("version", json!(2)),
            ("sample_specialist", json!("missing")),
            ("specialists", json!([])),
            (
                "routines",
                json!([{"id":"daily","specialist_id":"preparer","name":"x","schedule":"every 1m","prompt":"p"}]),
            ),
        ] {
            let mut invalid = original.clone();
            invalid[field] = value;
            assert!(Pack::parse(&invalid, &index).is_err(), "{field}");
        }
        let mut invalid = original.clone();
        invalid["questions"][0]["id"] = json!("Bad ID");
        assert!(Pack::parse(&invalid, &index).is_err());
        let mut repeated = original.clone();
        repeated["connections"] =
            json!([{"service_id":"gmail","name":"Gmail"},{"service_id":"gmail","name":"Again"}]);
        assert!(Pack::parse(&repeated, &index).is_err());
    }

    #[tokio::test]
    async fn resume_cancel_and_restart_reuse_resources_and_keep_imports_paused() {
        let fixture = Fixture::new();
        let setup = fixture.start("repository-monitoring").await;
        assert!(setup.bot_ids.is_empty());
        let configured = fixture.configure(&setup).await;
        let again = fixture.configure(&configured).await;
        assert_eq!(configured.bot_ids, again.bot_ids);
        assert_eq!(configured.routine_ids, again.routine_ids);
        assert_eq!(
            fixture.app.state.lock().unwrap().bots.len(),
            2,
            "Chef and one specialist"
        );
        let routine_id = configured.routine_ids.values().next().unwrap();
        assert!(!fixture.app.routine(routine_id).unwrap().is_enabled);
        assert!(crate::routines::set_enabled(&fixture.app, routine_id, true).is_err());
        handle(&fixture.app, "workflows.cancel", &json!({"id":setup.id}))
            .await
            .unwrap();
        let resumed = fixture.start("repository-monitoring").await;
        assert_eq!(resumed.bot_ids, configured.bot_ids);
        assert_eq!(resumed.routine_ids, configured.routine_ids);
        let reloaded = App::load(Config {
            home: fixture.home.clone(),
            port: 0,
        })
        .unwrap();
        assert_eq!(
            get(&reloaded, &setup.id).unwrap().bot_ids,
            configured.bot_ids
        );
        assert!(!reloaded.routine(routine_id).unwrap().is_enabled);
    }

    #[tokio::test]
    async fn suitable_bots_and_routines_are_reused_without_profile_changes() {
        let fixture = Fixture::new();
        let mut setup = fixture.start("repository-monitoring").await;
        let template = setup.templates.values().next().unwrap();
        let bot = Bot {
            id: "existing".into(),
            name: "My Watcher".into(),
            description: template.description.clone(),
            symbol_name: "eye".into(),
            accent: "red".into(),
            avatar: None,
            runner_id: setup.runner_id.clone(),
            provider: "deepseek".into(),
            model: None,
            thinking: None,
            legacy_instructions: String::new(),
            workdir: None,
            created_at: 1.0,
        };
        fixture.app.create_bot_with_dm(bot.clone(), None).unwrap();
        let spec = &setup.pack.routines[0];
        let routine = crate::routines::create(
            &fixture.app,
            &bot.id,
            &spec.name,
            &spec.schedule,
            &spec.prompt,
            None,
            false,
        )
        .unwrap();
        setup = fixture.configure(&setup).await;
        assert_eq!(setup.bot_ids.values().collect::<Vec<_>>(), vec![&bot.id]);
        assert_eq!(
            setup.routine_ids.values().collect::<Vec<_>>(),
            vec![&routine.id]
        );
        assert!(setup.owned_routine_ids.is_empty());
        assert_eq!(fixture.app.bot(&bot.id).unwrap(), bot);
    }

    #[tokio::test]
    async fn accepts_only_required_answers_and_bots_on_the_selected_runner() {
        let fixture = Fixture::new();
        let setup = fixture.start("repository-monitoring").await;
        for answers in [
            json!({}),
            json!({"repositories":"repo","unasked":"x"}),
            json!({"repositories":"API_KEY=secret-value"}),
        ] {
            assert!(handle(
                &fixture.app,
                "workflows.configure",
                &json!({"id":setup.id,"answers":answers})
            )
            .await
            .is_err());
        }
        assert_eq!(fixture.app.state.lock().unwrap().bots.len(), 1);
        assert!(handle(&fixture.app, "workflows.configure", &json!({"id":setup.id,"answers":{"repositories":"repo"},"bot_ids":{"monitor":"missing"}})).await.is_err());
    }

    #[tokio::test]
    async fn named_accounts_require_explicit_selection_and_retries_do_not_install_another() {
        let fixture = Fixture::new();
        let setup = fixture
            .configure(&fixture.start("inbox-triage").await)
            .await;
        fixture.advertise("gmail", &["gmail-work", "gmail-personal"]);
        let params = json!({"id":setup.id,"service_id":"gmail"});
        assert!(handle(&fixture.app, "workflows.connection", &params)
            .await
            .unwrap_err()
            .contains("Choose an existing account"));
        let result = handle(
            &fixture.app,
            "workflows.connection",
            &json!({"id":setup.id,"service_id":"gmail","plugin_id":"gmail-personal"}),
        )
        .await
        .unwrap();
        assert_eq!(result["setup"]["connection_ids"]["gmail"], "gmail-personal");
        let result = handle(&fixture.app, "workflows.connection", &params)
            .await
            .unwrap();
        assert_eq!(result["setup"]["connection_ids"]["gmail"], "gmail-personal");
        assert!(handle(
            &fixture.app,
            "workflows.connection",
            &json!({"id":setup.id,"service_id":"gmail","plugin_id":"slack-work"})
        )
        .await
        .is_err());
        assert_eq!(choices(&fixture.app, &setup.runner_id, "gmail").len(), 2);
    }

    #[tokio::test]
    async fn missing_integrations_preserve_partial_setup() {
        let fixture = Fixture::new();
        let setup = fixture
            .configure(&fixture.start("meeting-preparation").await)
            .await;
        let error = handle(
            &fixture.app,
            "workflows.connection",
            &json!({"id":setup.id,"service_id":"google-calendar"}),
        )
        .await
        .unwrap_err();
        assert!(error.contains("not available"));
        assert_eq!(
            fixture.start("meeting-preparation").await.bot_ids,
            setup.bot_ids
        );
    }

    #[tokio::test]
    async fn setup_answers_and_bindings_are_ciphertext_in_the_local_table_and_roster() {
        let fixture = Fixture::new();
        let setup = fixture.repository().await;
        let envelope = fixture.app.state.lock().unwrap().workflows[0].clone();
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(!json.contains("scope-repositories") && !json.contains("github"));
        let connection = rusqlite::Connection::open(fixture.app.config.database_path()).unwrap();
        let stored: String = connection
            .query_row(
                "SELECT json FROM workflow_setups WHERE id=?1",
                [&setup.id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!stored.contains("scope-repositories"));
        assert_eq!(decode(&fixture.app, &envelope).unwrap(), setup);
        let mut swapped = envelope.clone();
        swapped.id.push_str("other");
        assert!(
            decode(&fixture.app, &swapped).is_err(),
            "ciphertext is bound to its setup id"
        );
        let reloaded = App::load(Config {
            home: fixture.home.clone(),
            port: 0,
        })
        .unwrap();
        assert_eq!(get(&reloaded, &setup.id).unwrap(), setup);
    }

    #[tokio::test]
    async fn only_the_current_completed_sample_can_be_reviewed_before_enable() {
        let fixture = Fixture::new();
        let mut setup = fixture.repository().await;
        assert!(
            handle(&fixture.app, "workflows.enable", &json!({"id":setup.id}))
                .await
                .is_err()
        );
        let job = preview(&fixture.app, &mut setup, "preview-current");
        assert!(handle(
            &fixture.app,
            "workflows.review",
            &json!({"id":setup.id,"job_id":job.id})
        )
        .await
        .is_err());
        let mut partial = Message::new(
            &job.chat_id,
            Author::Bot {
                bot_id: job.bot_id.clone(),
            },
            Body::text("unfinished"),
        );
        partial.state = MessageState::Streaming;
        fixture.app.upsert_message(partial, false);
        sample_finished(&fixture.app, &job, TurnOutcome::Skipped);
        assert_eq!(
            get(&fixture.app, &setup.id).unwrap().sample.unwrap().state,
            "failed"
        );
        let job = preview(&fixture.app, &mut setup, "preview-next");
        let result = Message::new(
            &job.chat_id,
            Author::Bot {
                bot_id: job.bot_id.clone(),
            },
            Body::text("Here is the sample briefing."),
        );
        fixture.app.upsert_message(result.clone(), true);
        sample_finished(&fixture.app, &job, TurnOutcome::Sent);
        assert_eq!(
            get(&fixture.app, &setup.id)
                .unwrap()
                .sample
                .unwrap()
                .message_ids,
            [result.id]
        );
        assert!(handle(
            &fixture.app,
            "workflows.review",
            &json!({"id":setup.id,"job_id":"preview-current"})
        )
        .await
        .is_err());
        handle(
            &fixture.app,
            "workflows.review",
            &json!({"id":setup.id,"job_id":job.id}),
        )
        .await
        .unwrap();
        handle(&fixture.app, "workflows.enable", &json!({"id":setup.id}))
            .await
            .unwrap();
        assert!(setup
            .routine_ids
            .values()
            .all(|id| fixture.app.routine(id).unwrap().is_enabled));
        handle(&fixture.app, "workflows.cancel", &json!({"id":setup.id}))
            .await
            .unwrap();
        assert!(setup
            .routine_ids
            .values()
            .all(|id| !fixture.app.routine(id).unwrap().is_enabled));
        sample_finished(&fixture.app, &job, TurnOutcome::Sent);
        assert!(get(&fixture.app, &setup.id).unwrap().sample.is_none());
        assert!(
            handle(&fixture.app, "workflows.enable", &json!({"id":setup.id}))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn changes_to_answers_accounts_or_routines_require_another_sample() {
        let fixture = Fixture::new();
        let mut setup = fixture.repository().await;
        let job = preview(&fixture.app, &mut setup, "sample");
        fixture.app.upsert_message(
            Message::new(
                &job.chat_id,
                Author::Bot {
                    bot_id: job.bot_id.clone(),
                },
                Body::text("Sample."),
            ),
            false,
        );
        sample_finished(&fixture.app, &job, TurnOutcome::Sent);
        handle(
            &fixture.app,
            "workflows.review",
            &json!({"id":setup.id,"job_id":job.id}),
        )
        .await
        .unwrap();
        let mut answers = setup.answers.clone();
        answers.insert("repositories".into(), "another/repo".into());
        handle(
            &fixture.app,
            "workflows.configure",
            &json!({"id":setup.id,"answers":answers}),
        )
        .await
        .unwrap();
        assert!(get(&fixture.app, &setup.id).unwrap().sample.is_none());
        assert!(
            handle(&fixture.app, "workflows.enable", &json!({"id":setup.id}))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_pending_machine_advertisement_does_not_duplicate_a_bound_install() {
        let fixture = Fixture::new();
        let mut setup = fixture
            .configure(&fixture.start("inbox-triage").await)
            .await;
        setup
            .connection_ids
            .insert("gmail".into(), "gmail-stable-instance".into());
        save(&fixture.app, &setup).unwrap();
        let response = handle(
            &fixture.app,
            "workflows.connection",
            &json!({"id":setup.id,"service_id":"gmail"}),
        )
        .await
        .unwrap();
        assert_eq!(
            response["setup"]["connection_ids"]["gmail"],
            "gmail-stable-instance"
        );
        assert!(choices(&fixture.app, &setup.runner_id, "gmail").is_empty());
        handle(
            &fixture.app,
            "workflows.clear_connection",
            &json!({"id":setup.id,"service_id":"gmail"}),
        )
        .await
        .unwrap();
        assert!(get(&fixture.app, &setup.id)
            .unwrap()
            .connection_ids
            .is_empty());
    }

    #[tokio::test]
    async fn sample_boundaries_and_late_outcomes_keep_the_current_generation() {
        let fixture = Fixture::new();
        let mut setup = fixture.repository().await;
        let job = preview(&fixture.app, &mut setup, "queued-sample");
        fixture.app.upsert_message(
            Message::new(
                &job.chat_id,
                Author::Bot {
                    bot_id: job.bot_id.clone(),
                },
                Body::text("Reply from a preceding queued turn."),
            ),
            false,
        );
        sample_started(&fixture.app, &job);
        let own = Message::new(
            &job.chat_id,
            Author::Bot {
                bot_id: job.bot_id.clone(),
            },
            Body::text("This sample's reply."),
        );
        fixture.app.upsert_message(own.clone(), false);
        sample_finished(&fixture.app, &job, TurnOutcome::Sent);
        assert_eq!(
            get(&fixture.app, &setup.id)
                .unwrap()
                .sample
                .unwrap()
                .message_ids,
            vec![own.id]
        );
        let stale_job = preview(&fixture.app, &mut setup, "cancelled-sample");
        let mut stale_completion = get(&fixture.app, &setup.id).unwrap();
        stale_completion.sample.as_mut().unwrap().state = "ready".into();
        handle(&fixture.app, "workflows.cancel", &json!({"id":setup.id}))
            .await
            .unwrap();
        // Simulates completion already read before cancellation persisted.
        persist(&fixture.app, &stale_completion, Some(&stale_job.id)).unwrap();
        assert_eq!(get(&fixture.app, &setup.id).unwrap().phase, "cancelled");
        assert!(get(&fixture.app, &setup.id).unwrap().sample.is_none());
    }

    #[cfg(all(feature = "runner", feature = "server"))]
    #[tokio::test]
    async fn a_real_sample_job_streams_a_result_and_then_allows_review_and_activation() {
        use crate::credentials::{CustomApi, CustomModel, CustomProvider};
        use axum::{routing::post, Router};
        let seen = Arc::new(std::sync::Mutex::new(None::<Value>));
        let capture = seen.clone();
        let router = Router::new().route("/chat/completions", post(move |axum::Json(body): axum::Json<Value>| {
            let capture = capture.clone();
            async move {
                *capture.lock().unwrap() = Some(body);
                ([ ("content-type", "text/event-stream") ], "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"Sample: review the two open pull requests.\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let fixture = Fixture::new();
        let setup = fixture.repository().await;
        let model: CustomModel =
            serde_json::from_value(json!({"id":"sample-model","context_window":128000})).unwrap();
        fixture.app.credentials.lock().unwrap().custom.insert(
            "custom:sample".into(),
            CustomProvider {
                name: "Sample server".into(),
                api: CustomApi::ChatCompletions,
                base_url,
                api_key: String::new(),
                models: vec![model],
                created_at: 1,
            },
        );
        for id in setup.bot_ids.values() {
            fixture
                .app
                .update_bot(id, |b| b.provider = "custom:sample".into())
                .unwrap();
        }
        // Install a fixture record through the same persisted Store projection the local
        // Runner advertises. No external MCP or OAuth server is contacted by this sample.
        crate::config::write_json_private(&fixture.app.config.plugins_dir().join("installed.json"), &json!({
            "plugins":[{"manifest":{"id":"github","name":"Test GitHub","servers":{"fixture":{"type":"http","url":"http://127.0.0.1:9/mcp"}}},"source":"inline","installed_at":1,"variables":{}}]
        })).unwrap();
        *fixture.app.plugins.lock().unwrap() = crate::plugins::Store::load(&fixture.app.config);
        let mut events = fixture.app.events.subscribe();
        let initial = handle(&fixture.app, "workflows.sample", &json!({"id":setup.id}))
            .await
            .unwrap();
        let job_id = initial["setup"]["sample"]["job_id"].as_str().unwrap();
        let replay = handle(&fixture.app, "workflows.sample", &json!({"id":setup.id}))
            .await
            .unwrap();
        assert_eq!(
            initial["setup"]["sample"]["job_id"],
            replay["setup"]["sample"]["job_id"]
        );
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while get(&fixture.app, &setup.id).unwrap().sample.unwrap().state == "running" {
                let _ = events.recv().await;
            }
        })
        .await
        .unwrap();
        let completed = handle(&fixture.app, "workflows.get", &json!({"id":setup.id}))
            .await
            .unwrap();
        assert_eq!(completed["setup"]["sample"]["state"], "ready");
        assert_eq!(
            completed["sample_messages"][0]["body"]["text"],
            "Sample: review the two open pull requests."
        );
        assert!(!completed["can_enable"].as_bool().unwrap());
        let request = seen.lock().unwrap().take().unwrap();
        assert!(request["messages"]
            .to_string()
            .contains("scope-repositories"));
        assert!(request["messages"]
            .to_string()
            .contains("Selected integration instances"));
        let tools = request["tools"].as_array().unwrap();
        assert!(!tools.iter().any(|t| matches!(
            t["function"]["name"].as_str(),
            Some("routines" | "create_bot" | "edit_bot" | "install_plugin" | "connect_plugin")
        )));
        handle(
            &fixture.app,
            "workflows.review",
            &json!({"id":setup.id,"job_id":job_id}),
        )
        .await
        .unwrap();
        handle(&fixture.app, "workflows.enable", &json!({"id":setup.id}))
            .await
            .unwrap();
        assert!(setup
            .routine_ids
            .values()
            .all(|id| fixture.app.routine(id).unwrap().is_enabled));
        server.abort();
    }

    #[test]
    fn rolling_upgrade_and_independent_pack_updates_preserve_progress() {
        let first = Envelope {
            id: "first".into(),
            updated_at: 1.0,
            ciphertext: "a".into(),
        };
        let mut current = vec![first.clone()];
        assert!(merge(&mut current, None));
        assert!(merge(&mut current, Some(vec![])));
        let second = Envelope {
            id: "second".into(),
            updated_at: 1.0,
            ciphertext: "b".into(),
        };
        assert!(merge(&mut current, Some(vec![second.clone()])));
        let newer = Envelope {
            updated_at: 2.0,
            ciphertext: "new".into(),
            ..first
        };
        assert!(!merge(&mut current, Some(vec![newer.clone(), second])));
        assert_eq!(current[0], newer);
    }
}
