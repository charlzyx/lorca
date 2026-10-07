//! Runner-owned allowances. Jobs and canonical tasks keep their existing identities; this
//! module owns only accounting, reservations, exhaustion, and explicit recovery.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app::App;
use crate::config::now_secs;
use crate::model::Job;

const PURPOSE: &str = "runner_budgets_v1";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct BudgetLimits {
    pub max_usd: Option<f64>,
    pub max_tokens: Option<u64>,
    pub max_runtime_secs: Option<u64>,
    pub max_retries: Option<u64>,
    pub max_connector_calls: Option<u64>,
}

impl BudgetLimits {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .max_usd
            .is_some_and(|usd| !usd.is_finite() || usd < 0.0)
        {
            return Err("The spending limit must be a finite, nonnegative dollar amount.".into());
        }
        if self
            .max_runtime_secs
            .is_some_and(|seconds| seconds > 31_536_000)
        {
            return Err(
                "A runtime allowance is at most one year; leave it unset for unlimited.".into(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Pricing {
    Api,
    SubscriptionEstimate,
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BudgetUsage {
    pub tokens: u64,
    pub api_cost_usd: f64,
    pub subscription_estimate_usd: f64,
    pub unknown_price_calls: u64,
    /// Requests without usage use input/received-output estimates; a restart retains
    /// outstanding reservations. These are estimates rather than a claim about the bill.
    pub estimated_calls: u64,
    pub model_calls: u64,
    pub runtime_secs: f64,
    pub retries: u64,
    pub connector_calls: u64,
}

impl BudgetUsage {
    fn usd(&self) -> f64 {
        self.api_cost_usd + self.subscription_estimate_usd
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BudgetSnapshot {
    /// `chat` is a default allowance for new ad-hoc Jobs. Other kinds carry consumption.
    pub kind: String,
    pub id: String,
    pub runner_id: String,
    pub bot_id: String,
    pub chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub limits: BudgetLimits,
    pub usage: BudgetUsage,
    /// `ready`, `running`, `complete`, `budget_exhausted`, or `interrupted`.
    pub state: String,
    pub reason: Option<String>,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct Charge {
    tokens: u64,
    usd: f64,
    pricing: Option<Pricing>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Record {
    view: BudgetSnapshot,
    #[serde(default)]
    pending: BTreeMap<String, Charge>,
    job: Option<Job>,
    started_at: Option<f64>,
    #[serde(default)]
    active_runs: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct Ledger {
    records: BTreeMap<String, Record>,
    #[serde(default)]
    remote: BTreeMap<String, Vec<BudgetSnapshot>>,
    #[serde(default)]
    receipts: BTreeMap<String, Value>,
}

/// Lazy loading also works when the user creates/pairs an identity after CLI startup.
#[derive(Default)]
pub struct BudgetStore(Mutex<Option<Ledger>>);

fn key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

impl BudgetStore {
    pub fn clear(&self) {
        *self.0.lock().unwrap() = None;
    }

    fn load<'a>(&self, app: &App, held: &'a mut Option<Ledger>) -> Result<&'a mut Ledger, String> {
        if held.is_none() {
            let dek = app.dek().ok_or("Create or pair an identity first.")?;
            let mut ledger: Ledger = match app
                .store
                .runner_limits(PURPOSE)
                .map_err(|e| e.to_string())?
            {
                Some(bytes) => crate::crypto::decrypt_json(&dek, PURPOSE, &bytes)
                    .map_err(|e| format!("Cannot read Runner budgets: {e}"))?,
                None => Ledger::default(),
            };
            // A restart never replenishes an allowance or replays an uncertain effect. Keep
            // reservations as estimated consumption and require explicit recovery.
            for record in ledger.records.values_mut() {
                for (_, charge) in std::mem::take(&mut record.pending) {
                    apply_charge(&mut record.view.usage, &charge, true);
                }
                if let Some(started) = record.started_at.take() {
                    record.active_runs = 0;
                    record.view.usage.runtime_secs += (now_secs() - started).max(0.0);
                    record.view.state = "interrupted".into();
                    record.view.reason = Some("The Runner restarted during work. Check its completed effects, then explicitly resume.".into());
                }
            }
            self.save(app, &ledger)?;
            *held = Some(ledger);
        }
        Ok(held.as_mut().unwrap())
    }

    fn save(&self, app: &App, ledger: &Ledger) -> Result<(), String> {
        let dek = app.dek().ok_or("The account is no longer available.")?;
        let ciphertext =
            crate::crypto::encrypt_json(&dek, PURPOSE, ledger).map_err(|e| e.to_string())?;
        app.store
            .set_runner_limits(PURPOSE, &ciphertext)
            .map_err(|e| format!("Cannot persist Runner accounting: {e}"))
    }

    fn change<T>(
        &self,
        app: &App,
        f: impl FnOnce(&mut Ledger) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut held = self.0.lock().unwrap();
        let current = self.load(app, &mut held)?;
        let mut changed = current.clone();
        let result = f(&mut changed);
        // Persist an exhausted state even when admission failed. A storage failure prevents
        // starting work, and no in-memory grant becomes visible without its durable write.
        if changed != *current {
            self.save(app, &changed)?;
        }
        *current = changed;
        result
    }

    pub fn snapshots(&self, app: &App) -> Vec<BudgetSnapshot> {
        if !app.has_identity() {
            return Vec::new();
        }
        let mut held = self.0.lock().unwrap();
        match self.load(app, &mut held) {
            Ok(ledger) => ledger
                .records
                .values()
                .map(|r| r.view.clone())
                .chain(ledger.remote.values().flatten().cloned())
                .collect(),
            Err(error) => {
                tracing::error!(%error, "reading budget snapshots");
                Vec::new()
            }
        }
    }

    pub fn local_snapshots(&self, app: &App) -> Vec<BudgetSnapshot> {
        let this = app.this_device_id().unwrap_or_default();
        let mut snapshots: Vec<_> = self
            .snapshots(app)
            .into_iter()
            .filter(|s| s.runner_id == this)
            .collect();
        snapshots.sort_by(|a, b| b.updated_at.total_cmp(&a.updated_at));
        let mut finished = 0;
        snapshots.retain(|s| {
            if s.kind != "job" || s.state != "complete" {
                return true;
            }
            finished += 1;
            finished <= 100
        });
        snapshots
    }

    pub fn merge_remote(&self, app: &App, runner: &str, mut snapshots: Vec<BudgetSnapshot>) {
        snapshots.retain(|s| s.runner_id == runner);
        let result = self.change(app, |ledger| {
            ledger.remote.insert(runner.into(), snapshots);
            Ok(())
        });
        if let Err(error) = result {
            tracing::error!(%error, "keeping encrypted remote budget state");
        }
        app.emit(crate::events::Event::BudgetsChanged {
            budgets: self.snapshots(app),
        });
    }

    fn publish(&self, app: &App) {
        app.emit(crate::events::Event::BudgetsChanged {
            budgets: self.snapshots(app),
        });
        app.push_machine_blob_if_changed();
    }

    pub fn admit(&self, app: &App, kind: &str, id: &str) -> Result<(), String> {
        self.change(app, |ledger| match ledger.records.get_mut(&key(kind, id)) {
            Some(record) => check(record),
            None => Ok(()),
        })
    }
}

fn apply_charge(usage: &mut BudgetUsage, charge: &Charge, estimated: bool) {
    usage.tokens = usage.tokens.saturating_add(charge.tokens);
    match charge.pricing.unwrap_or(Pricing::Unknown) {
        Pricing::Api => usage.api_cost_usd += charge.usd,
        Pricing::SubscriptionEstimate => usage.subscription_estimate_usd += charge.usd,
        Pricing::Unknown => usage.unknown_price_calls += 1,
    }
    usage.estimated_calls += u64::from(estimated);
}

fn exhaust(record: &mut Record, reason: impl Into<String>) -> String {
    let reason = format!(
        "Budget exhausted: {} Increase the allowance or explicitly renew it to resume.",
        reason.into()
    );
    record.view.state = "budget_exhausted".into();
    record.view.reason = Some(reason.clone());
    record.view.updated_at = now_secs();
    reason
}

fn check(record: &mut Record) -> Result<(), String> {
    if matches!(
        record.view.state.as_str(),
        "budget_exhausted" | "interrupted"
    ) {
        return Err(record
            .view
            .reason
            .clone()
            .unwrap_or_else(|| "Explicit budget recovery is required.".into()));
    }
    let limits = &record.view.limits;
    let used = &record.view.usage;
    let reason = if limits.max_tokens.is_some_and(|v| used.tokens >= v) {
        Some("the token allowance is used.")
    } else if limits.max_usd.is_some_and(|v| used.usd() >= v) {
        Some("the spending allowance is used.")
    } else if limits.max_runtime_secs.is_some_and(|v| {
        used.runtime_secs
            + record
                .started_at
                .map(|at| (now_secs() - at).max(0.0))
                .unwrap_or(0.0)
            >= v as f64
    }) {
        Some("the runtime allowance is used.")
    } else if limits
        .max_connector_calls
        .is_some_and(|v| v > 0 && used.connector_calls >= v)
    {
        Some("the connector-call allowance is used.")
    } else {
        None
    };
    if let Some(reason) = reason {
        return Err(exhaust(record, reason));
    }
    Ok(())
}

fn new_record(
    app: &App,
    kind: &str,
    id: &str,
    bot_id: &str,
    chat_id: &str,
    limits: BudgetLimits,
) -> Record {
    Record {
        view: BudgetSnapshot {
            kind: kind.into(),
            id: id.into(),
            runner_id: app.this_device_id().unwrap_or_default(),
            bot_id: bot_id.into(),
            chat_id: chat_id.into(),
            job_kind: None,
            task_id: None,
            limits,
            usage: BudgetUsage::default(),
            state: "ready".into(),
            reason: None,
            updated_at: now_secs(),
        },
        pending: BTreeMap::new(),
        job: None,
        started_at: None,
        active_runs: 0,
    }
}

/// All budget management is routed to the assigned Runner through the existing request
/// transport. A chat allowance is a template for new Jobs, never an accounting reset.
pub async fn dispatch(app: &Arc<App>, method: &str, params: &Value) -> Result<Value, String> {
    let runner = params["runner_id"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            params["bot_id"]
                .as_str()
                .and_then(|id| app.bot(id))
                .map(|b| b.runner_id)
        })
        .or_else(|| app.this_device_id())
        .ok_or("missing runner_id")?;
    if app.this_device_id().as_deref() != Some(runner.as_str()) {
        return crate::requests::ask(app, &runner, method, params.clone()).await;
    }
    serve(app, method, params)
}

pub fn serve(app: &Arc<App>, method: &str, params: &Value) -> Result<Value, String> {
    if method == "budgets.list" {
        let chat = params["chat_id"].as_str();
        return Ok(
            json!({ "budgets": app.budgets.local_snapshots(app).into_iter().filter(|v| chat.is_none_or(|c| c == v.chat_id)).collect::<Vec<_>>() }),
        );
    }
    let kind = params["kind"]
        .as_str()
        .filter(|kind| matches!(*kind, "job" | "task" | "routine" | "chat"))
        .ok_or("kind must be job, task, routine, or chat")?;
    let id = params["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or("missing id")?;
    let record_key = key(kind, id);
    if method == "budgets.get" {
        return app.budgets.change(app, |ledger| {
            ledger
                .records
                .get(&record_key)
                .map(|r| json!(r.view))
                .ok_or("No allowance is recorded for this work yet.".into())
        });
    }
    if method == "budgets.set" {
        let limits: BudgetLimits =
            serde_json::from_value(params["limits"].clone()).map_err(|e| e.to_string())?;
        limits.validate()?;
        let bot_id = if kind == "routine" {
            app.routine(id).ok_or("Unknown routine")?.bot_id
        } else if kind == "chat" {
            app.chat(id)
                .and_then(|c| c.meta.bot_ids.first().cloned())
                .ok_or("Unknown chat")?
        } else {
            params["bot_id"]
                .as_str()
                .ok_or("missing bot_id")?
                .to_string()
        };
        let bot = app.bot(&bot_id).ok_or("Unknown bot")?;
        if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
            return Err("Budgets are managed on the bot's assigned Runner.".into());
        }
        if kind == "task"
            && !id
                .strip_prefix("task-")
                .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
        {
            return Err("Use the canonical task-UUID from tasks.get.".into());
        }
        let chat_id = if kind == "chat" {
            id.to_string()
        } else {
            params["chat_id"].as_str().unwrap_or_default().to_string()
        };
        let snapshot = app.budgets.change(app, |ledger| {
            let record = ledger
                .records
                .entry(record_key)
                .or_insert_with(|| new_record(app, kind, id, &bot_id, &chat_id, limits.clone()));
            record.view.limits = limits;
            record.view.updated_at = now_secs();
            Ok(record.view.clone())
        })?;
        app.budgets.publish(app);
        return Ok(json!(snapshot));
    }
    if method == "budgets.resume" {
        let receipt = params["request_id"].as_str().filter(|id| !id.is_empty() && id.len() <= 128)
            .ok_or("Resume requires a unique request_id so duplicate delivery cannot replenish the allowance twice.")?;
        let renew = params["renew"].as_bool().unwrap_or(false);
        let (snapshot, job, replay) = app.budgets.change(app, |ledger| {
            if let Some(saved) = ledger.receipts.get(receipt) {
                if saved["kind"] != kind || saved["id"] != id {
                    return Err(
                        "This request_id already belongs to another budget recovery.".into(),
                    );
                }
                return Ok((
                    serde_json::from_value(saved.clone()).map_err(|e| e.to_string())?,
                    None,
                    true,
                ));
            }
            let record = ledger
                .records
                .get_mut(&record_key)
                .ok_or("Unknown budget")?;
            if params["run"].as_bool().unwrap_or(false) && (kind == "task" || record.job.as_ref().is_some_and(|job| job.kind == "event" || job.task_id.is_some())) {
                return Err("Recover the allowance with run:false, then Retry the delivery in Events or run the canonical task through tasks.run. Its inbox/ownership admission must be re-armed before work starts.".into());
            }
            if record.started_at.is_some() || !record.pending.is_empty() {
                return Err("Wait for the current work to stop before resuming its budget.".into());
            }
            if renew {
                record.view.usage = BudgetUsage::default();
            }
            record.view.state = "ready".into();
            record.view.reason = None;
            check(record)?;
            record.view.updated_at = now_secs();
            let snapshot = record.view.clone();
            let job = record.job.clone();
            ledger.receipts.insert(receipt.into(), json!(snapshot));
            Ok((snapshot, job, false))
        })?;
        app.budgets.publish(app);
        #[cfg(feature = "runner")]
        if !replay && params["run"].as_bool().unwrap_or(false) {
            if let Some(mut job) = job {
                // Continue from the durable transcript; completed/uncertain connector requests
                // are never stored as instructions to retry.
                job.check = job.check.or_else(|| {
                    Some(crate::model::CheckReport {
                        found: String::new(),
                        error: None,
                    })
                });
                crate::runtime::start_turn(app, job);
            } else if kind == "routine" {
                crate::routines::run_now(app, id)?;
            }
        }
        #[cfg(not(feature = "runner"))]
        let _ = (job, replay);
        return Ok(json!(snapshot));
    }
    Err(format!("Unknown budget method {method}"))
}

#[cfg(feature = "runner")]
mod runtime;
#[cfg(feature = "runner")]
pub use runtime::{current, for_job, for_routine, wrap_provider, BudgetContext};
