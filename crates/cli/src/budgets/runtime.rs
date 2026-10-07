use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use lorca_agent::provider::AssistantEvent;
use lorca_agent::{AssistantEventStream, ModelRequest, Provider, RequestHooks};
use tokio_util::sync::CancellationToken;

use super::*;

tokio::task_local! { static CURRENT: BudgetContext; }

/// Shared across inference, review, script host functions, plugin calls, and housekeeping.
/// Constructors that escape the current task capture this handle explicitly.
#[derive(Clone)]
pub struct BudgetContext {
    app: Arc<App>,
    keys: Vec<String>,
}

pub fn current() -> Option<BudgetContext> {
    CURRENT.try_with(Clone::clone).ok()
}

pub fn for_job(app: &Arc<App>, job: &Job) -> Result<BudgetContext, String> {
    let bot = app.bot(&job.bot_id).ok_or("Unknown bot")?;
    if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
        return Err("Work is admitted only on its assigned Runner.".into());
    }
    let mut scopes = vec![("job", job.id.as_str())];
    if let Some(task) = &job.task_id {
        scopes.push(("task", task));
    }
    if let Some(routine) = &job.routine_id {
        scopes.push(("routine", routine));
    }
    let keys = app.budgets.change(app, |ledger| {
        let default = ledger
            .records
            .get(&key("chat", &job.chat_id))
            .map(|r| r.view.limits.clone())
            .unwrap_or_default();
        let mut keys = Vec::new();
        for (kind, id) in &scopes {
            let k = key(kind, id);
            let limits = if (*kind == "job" && job.task_id.is_none() && job.routine_id.is_none())
                || *kind == "task"
            {
                default.clone()
            } else {
                BudgetLimits::default()
            };
            let record = ledger
                .records
                .entry(k.clone())
                .or_insert_with(|| new_record(app, kind, id, &job.bot_id, &job.chat_id, limits));
            if record.view.kind == "job" || record.view.chat_id.is_empty() {
                record.view.chat_id = job.chat_id.clone();
            }
            record.view.job_kind = Some(job.kind.clone());
            record.view.task_id = job.task_id.clone();
            record.job = Some(job.clone());
            keys.push(k);
        }
        check_scopes(ledger, &keys)?;
        Ok(keys)
    });
    app.budgets.publish(app);
    Ok(BudgetContext {
        app: app.clone(),
        keys: keys?,
    })
}

/// A check has the same routine allowance as the model run it may start. When saved/run
/// inside a turn it also retains the task/Job scopes already in force.
pub fn for_routine(
    app: &Arc<App>,
    routine: &crate::model::Routine,
    chat_id: &str,
) -> Result<BudgetContext, String> {
    let bot = app.bot(&routine.bot_id).ok_or("Unknown bot")?;
    if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
        return Err("Checks are admitted only on their assigned Runner.".into());
    }
    let k = key("routine", &routine.id);
    let mut keys = current().map(|c| c.keys).unwrap_or_default();
    if !keys.contains(&k) {
        keys.push(k.clone());
    }
    app.budgets.change(app, |ledger| {
        ledger.records.entry(k).or_insert_with(|| {
            new_record(
                app,
                "routine",
                &routine.id,
                &routine.bot_id,
                chat_id,
                BudgetLimits::default(),
            )
        });
        check_scopes(ledger, &keys)
    })?;
    Ok(BudgetContext {
        app: app.clone(),
        keys,
    })
}

fn propagate_exhaustion(ledger: &mut Ledger, keys: &[String], reason: &str) {
    let state = keys
        .iter()
        .filter_map(|key| ledger.records.get(key))
        .find(|record| {
            matches!(
                record.view.state.as_str(),
                "budget_exhausted" | "interrupted"
            ) && record.view.reason.as_deref() == Some(reason)
        })
        .map(|record| record.view.state.clone());
    if let Some(state) = state {
        for key in keys {
            if let Some(record) = ledger.records.get_mut(key) {
                record.view.state = state.clone();
                record.view.reason = Some(reason.into());
                record.view.updated_at = now_secs();
            }
        }
    }
}

fn check_scopes(ledger: &mut Ledger, keys: &[String]) -> Result<(), String> {
    for key in keys {
        if let Err(reason) = check(ledger.records.get_mut(key).ok_or("Unknown allowance")?) {
            propagate_exhaustion(ledger, keys, &reason);
            return Err(reason);
        }
    }
    Ok(())
}

impl BudgetContext {
    fn change<T>(&self, f: impl FnOnce(&mut Ledger) -> Result<T, String>) -> Result<T, String> {
        self.app.budgets.change(&self.app, |ledger| {
            let result = f(ledger);
            if let Err(reason) = &result {
                propagate_exhaustion(ledger, &self.keys, reason);
            }
            result
        })
    }
    pub async fn scope<T>(&self, future: impl Future<Output = T>) -> T {
        CURRENT.scope(self.clone(), future).await
    }

    pub fn check(&self) -> Result<(), String> {
        self.change(|ledger| {
            for k in &self.keys {
                check(ledger.records.get_mut(k).ok_or("Unknown allowance")?)?;
            }
            Ok(())
        })
    }

    fn start_runtime(&self) -> Result<Option<Duration>, String> {
        self.change(|ledger| {
            let now = now_secs();
            let mut remaining: Option<f64> = None;
            for k in &self.keys {
                let record = ledger.records.get_mut(k).ok_or("Unknown allowance")?;
                check(record)?;
                if let Some(limit) = record.view.limits.max_runtime_secs {
                    let elapsed = record.started_at.map(|t| (now - t).max(0.0)).unwrap_or(0.0);
                    let left = limit as f64 - record.view.usage.runtime_secs - elapsed;
                    if left <= 0.0 {
                        return Err(exhaust(record, "the runtime allowance is used."));
                    }
                    remaining = Some(remaining.map(|v| v.min(left)).unwrap_or(left));
                }
            }
            for k in &self.keys {
                let record = ledger.records.get_mut(k).unwrap();
                record.started_at.get_or_insert(now);
                record.active_runs += 1;
                record.view.state = "running".into();
                record.view.updated_at = now;
            }
            Ok(remaining.map(Duration::from_secs_f64))
        })
    }

    fn end_runtime(&self, deadline: bool) {
        let result = self.change(|ledger| {
            let now = now_secs();
            for k in &self.keys {
                let record = ledger.records.get_mut(k).ok_or("Unknown allowance")?;
                record.active_runs = record.active_runs.saturating_sub(1);
                if record.active_runs == 0 {
                    if let Some(started) = record.started_at.take() {
                        record.view.usage.runtime_secs += (now - started).max(0.0);
                    }
                }
                if deadline
                    && record
                        .view
                        .limits
                        .max_runtime_secs
                        .is_some_and(|v| record.view.usage.runtime_secs >= v as f64)
                {
                    exhaust(record, "the runtime allowance is used.");
                } else if record.view.state == "running" && record.active_runs == 0 {
                    record.view.state = if record.view.kind == "job" {
                        "complete"
                    } else {
                        "ready"
                    }
                    .into();
                    let _ = check(record);
                }
                record.view.updated_at = now;
            }
            let _ = check_scopes(ledger, &self.keys);
            Ok(())
        });
        if let Err(error) = result {
            tracing::error!(%error, "recording runtime consumption");
        }
        self.app.budgets.publish(&self.app);
    }

    /// Runtime includes checks, connection waits, retries, review, and user questions. The
    /// same cancellation token reaches provider streams, tools, and MCP cancellation.
    pub async fn run<T>(
        &self,
        cancel: &CancellationToken,
        future: impl Future<Output = T>,
    ) -> Result<T, String> {
        let deadline = self.start_runtime()?;
        let future = self.scope(future);
        tokio::pin!(future);
        let (answer, timed_out) = if let Some(deadline) = deadline {
            tokio::select! {
                answer = &mut future => (answer, false),
                _ = tokio::time::sleep(deadline) => {
                    cancel.cancel();
                    (future.await, true)
                }
            }
        } else {
            (future.await, false)
        };
        self.end_runtime(timed_out);
        if timed_out {
            Err("Budget exhausted: the runtime allowance is used. Increase or renew the allowance, then resume.".into())
        } else {
            Ok(answer)
        }
    }

    pub fn connector_call(&self) -> Result<(), String> {
        self.change(|ledger| {
            for k in &self.keys {
                let record = ledger.records.get_mut(k).ok_or("Unknown allowance")?;
                check(record)?;
                if record
                    .view
                    .limits
                    .max_connector_calls
                    .is_some_and(|v| record.view.usage.connector_calls >= v)
                {
                    return Err(exhaust(record, "the connector-call allowance is used."));
                }
            }
            for k in &self.keys {
                let record = ledger.records.get_mut(k).unwrap();
                record.view.usage.connector_calls += 1;
                record.view.updated_at = now_secs();
            }
            Ok(())
        })
    }

    fn retry(&self) -> Result<(), String> {
        self.change(|ledger| {
            for k in &self.keys {
                let record = ledger.records.get_mut(k).ok_or("Unknown allowance")?;
                check(record)?;
                if record
                    .view
                    .limits
                    .max_retries
                    .is_some_and(|v| record.view.usage.retries >= v)
                {
                    return Err(exhaust(record, "the retry allowance is used."));
                }
            }
            for k in &self.keys {
                ledger.records.get_mut(k).unwrap().view.usage.retries += 1;
            }
            Ok(())
        })
    }

    fn reserve(
        &self,
        input: u64,
        requested_output: u64,
        rates: Option<(f64, f64)>,
        pricing: Pricing,
    ) -> Result<(String, u64, Charge), String> {
        let result = self.change(|ledger| {
            let mut output = requested_output;
            for k in &self.keys {
                let record = ledger.records.get_mut(k).ok_or("Unknown allowance")?;
                check(record)?;
                let pending_tokens: u64 = record.pending.values().map(|c| c.tokens).sum();
                let pending_usd: f64 = record.pending.values().map(|c| c.usd).sum();
                if let Some(limit) = record.view.limits.max_tokens {
                    let remaining = limit
                        .saturating_sub(record.view.usage.tokens.saturating_add(pending_tokens));
                    if remaining <= input {
                        return Err(exhaust(
                            record,
                            "the next request does not fit the remaining token allowance.",
                        ));
                    }
                    output = output.min(remaining - input);
                }
                if let Some(limit) = record.view.limits.max_usd {
                    let Some((input_rate, output_rate)) = rates else {
                        // Monetary caps cannot admit unpriced work as free. Supply another
                        // measurable bound and use it for this model instead.
                        if record.view.limits.max_tokens.is_some()
                            || record.view.limits.max_runtime_secs.is_some()
                        {
                            continue;
                        }
                        return Err(exhaust(
                            record,
                            "this provider has unknown pricing; set a token or runtime limit.",
                        ));
                    };
                    let left = limit
                        - record.view.usage.usd()
                        - pending_usd
                        - input_rate * input as f64 / 1_000_000.0;
                    if left < 0.0 {
                        return Err(exhaust(
                            record,
                            "the next request does not fit the remaining spending allowance.",
                        ));
                    }
                    if output_rate > 0.0 {
                        output =
                            output.min((left * 1_000_000.0 / output_rate).floor().max(0.0) as u64);
                    }
                    if output == 0 {
                        return Err(exhaust(
                            record,
                            "the next reply does not fit the remaining spending allowance.",
                        ));
                    }
                }
            }
            let id = uuid::Uuid::new_v4().to_string();
            let usd = rates
                .map(|(i, o)| (i * input as f64 + o * output as f64) / 1_000_000.0)
                .unwrap_or(0.0);
            let charge = Charge {
                tokens: input.saturating_add(output),
                usd,
                pricing: Some(pricing),
            };
            for k in &self.keys {
                let record = ledger.records.get_mut(k).unwrap();
                record.pending.insert(id.clone(), charge.clone());
                record.view.usage.model_calls += 1;
            }
            Ok((id, output, charge))
        });
        if result.is_err() {
            self.app.budgets.publish(&self.app);
        }
        result
    }

    fn settle(&self, id: &str, usage: Option<&lorca_agent::Usage>) -> Result<(), String> {
        self.settle_usage(id, usage, usage.is_none())
    }

    fn settle_usage(
        &self,
        id: &str,
        usage: Option<&lorca_agent::Usage>,
        estimated: bool,
    ) -> Result<(), String> {
        self.change(|ledger| {
            for k in &self.keys {
                let record = ledger.records.get_mut(k).ok_or("Unknown allowance")?;
                let Some(mut charge) = record.pending.remove(id) else {
                    continue;
                };
                if let Some(usage) = usage {
                    charge.tokens = lorca_agent::estimate::context_tokens(usage);
                    charge.usd = usage.cost.total;
                }
                apply_charge(&mut record.view.usage, &charge, estimated);
                record.view.updated_at = now_secs();
            }
            let _ = check_scopes(ledger, &self.keys);
            Ok(())
        })?;
        self.app.budgets.publish(&self.app);
        Ok(())
    }
}

/// Labels pricing on every CLI model call, even without a configured allowance.
pub fn wrap_provider(app: &Arc<App>, inner: Arc<dyn Provider>) -> Arc<dyn Provider> {
    Arc::new(BudgetProvider {
        app: app.clone(),
        inner,
        context: current(),
        failed: Arc::new(Mutex::new(false)),
    })
}

struct BudgetProvider {
    app: Arc<App>,
    inner: Arc<dyn Provider>,
    context: Option<BudgetContext>,
    failed: Arc<Mutex<bool>>,
}

fn pricing(provider: &dyn Provider) -> Pricing {
    if provider.provider_id().starts_with("custom:") || provider.model_info().is_none() {
        Pricing::Unknown
    } else if matches!(provider.provider_id(), "chatgpt" | "grok" | "opencode-go") {
        Pricing::SubscriptionEstimate
    } else {
        Pricing::Api
    }
}

#[async_trait]
impl Provider for BudgetProvider {
    fn provider_id(&self) -> &str {
        self.inner.provider_id()
    }
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }
    fn supports_images(&self) -> bool {
        self.inner.supports_images()
    }
    fn model_info(&self) -> Option<&'static lorca_agent::models::ModelInfo> {
        self.inner.model_info()
    }

    async fn stream(
        &self,
        mut request: ModelRequest,
        cancel: CancellationToken,
    ) -> AssistantEventStream {
        let pricing = pricing(self.inner.as_ref());
        let chat_id = request.options.session_id.clone();
        let mut permit = None;
        if let Some(context) = &self.context {
            let prepared = (|| {
                if *self.failed.lock().unwrap() {
                    context.retry()?;
                }
                let input = lorca_agent::estimate::estimate_text_tokens(&request.system_prompt)
                    + request
                        .messages
                        .iter()
                        .map(|message| {
                            lorca_agent::estimate::estimate_message_tokens(
                                &lorca_agent::AgentMessage::from(message.clone()),
                            ) + 8
                        })
                        .sum::<u64>()
                    + serde_json::to_vec(&request.tools)
                        .unwrap_or_default()
                        .len()
                        .div_ceil(4) as u64;
                let requested_output = request
                    .max_tokens
                    .or_else(|| self.model_info().map(|i| i.max_output))
                    .unwrap_or(8192)
                    .max(1);
                // Use the highest published tier, including cache-write rates, for admission.
                let rates = (pricing != Pricing::Unknown)
                    .then(|| self.model_info())
                    .flatten()
                    .map(|i| {
                        std::iter::once(i.rates)
                            .chain(i.tiers.iter().map(|t| t.rates))
                            .fold((0f64, 0f64), |(input, output), r| {
                                (
                                    input.max(r.input).max(r.cache_read).max(r.cache_write),
                                    output.max(r.output),
                                )
                            })
                    });
                let (id, output, charge) =
                    context.reserve(input, requested_output, rates, pricing)?;
                request.max_tokens = Some(output);
                let _ = charge;
                Ok::<_, String>(Arc::new(ModelPermit {
                    context: context.clone(),
                    id: Mutex::new(Some(id)),
                    input,
                    output,
                    rates,
                    pricing,
                    output_chars: std::sync::atomic::AtomicU64::new(0),
                    outer: request.options.hooks.take(),
                }))
            })();
            match prepared {
                Ok(p) => {
                    request.options.hooks = Some(p.clone());
                    permit = Some(p);
                }
                Err(message) => {
                    return Box::pin(futures::stream::once(async {
                        AssistantEvent::Error {
                            message,
                            aborted: false,
                        }
                    }))
                }
            }
        }
        let stream = self.inner.stream(request, cancel).await;
        let app = self.app.clone();
        let failed = self.failed.clone();
        Box::pin(futures::stream::unfold(
            (stream, permit, false),
            move |(mut stream, permit, mut done)| {
                let app = app.clone();
                let failed = failed.clone();
                let chat_id = chat_id.clone();
                async move {
                    let event = stream.next().await?;
                    match &event {
                        AssistantEvent::TextDelta { delta, .. }
                        | AssistantEvent::ThinkingDelta { delta, .. }
                        | AssistantEvent::ToolCallDelta { delta, .. } => {
                            if let Some(permit) = &permit {
                                permit.output_chars.fetch_add(
                                    delta.len() as u64,
                                    std::sync::atomic::Ordering::Relaxed,
                                );
                            }
                        }
                        AssistantEvent::Done { usage, .. } if !done => {
                            if let Some(permit) = &permit {
                                permit.finish(Some(usage));
                            }
                            if let Some(chat_id) = &chat_id {
                                app.record_pricing(chat_id, pricing, usage.cost.total);
                            }
                            *failed.lock().unwrap() = false;
                            done = true;
                        }
                        AssistantEvent::Error { .. } if !done => {
                            if let Some(permit) = &permit {
                                permit.finish(None);
                            }
                            *failed.lock().unwrap() = true;
                            done = true;
                        }
                        _ => {}
                    }
                    Some((event, (stream, permit, done)))
                }
            },
        ))
    }
}

struct ModelPermit {
    context: BudgetContext,
    id: Mutex<Option<String>>,
    input: u64,
    output: u64,
    rates: Option<(f64, f64)>,
    pricing: Pricing,
    output_chars: std::sync::atomic::AtomicU64,
    outer: Option<Arc<dyn RequestHooks>>,
}

impl ModelPermit {
    fn finish(&self, usage: Option<&lorca_agent::Usage>) {
        if let Some(id) = self.id.lock().unwrap().take() {
            let mut estimated = lorca_agent::Usage::default();
            estimated.input = self.input;
            estimated.output = self
                .output_chars
                .swap(0, std::sync::atomic::Ordering::Relaxed)
                .div_ceil(4)
                .min(self.output);
            estimated.cost.total = self
                .rates
                .map(|(input, output)| {
                    (input * estimated.input as f64 + output * estimated.output as f64)
                        / 1_000_000.0
                })
                .unwrap_or(0.0);
            if let Err(error) =
                self.context
                    .settle_usage(&id, Some(usage.unwrap_or(&estimated)), usage.is_none())
            {
                tracing::error!(%error, "settling model usage");
            }
        }
    }
}

impl Drop for ModelPermit {
    fn drop(&mut self) {
        self.finish(None);
    }
}

#[async_trait]
impl RequestHooks for ModelPermit {
    async fn api_key(&self) -> Option<String> {
        match &self.outer {
            Some(h) => h.api_key().await,
            None => None,
        }
    }
    fn before_payload(&self, payload: &mut Value) {
        if let Some(h) = &self.outer {
            h.before_payload(payload);
        }
    }
    fn after_response(&self, response: &lorca_agent::request::ResponseInfo) {
        if let Some(h) = &self.outer {
            h.after_response(response);
        }
    }
    async fn before_request(&self, retry: bool, cancel: &CancellationToken) -> Result<(), String> {
        if let Some(h) = &self.outer {
            h.before_request(retry, cancel).await?;
        }
        if cancel.is_cancelled() {
            return Err("Stopped".into());
        }
        self.context.check()?;
        if retry {
            self.finish(None);
            self.context.retry()?;
            let (id, output, _) =
                self.context
                    .reserve(self.input, self.output, self.rates, self.pricing)?;
            // The already-built HTTP body cannot change its output cap on a retry. Refuse
            // rather than start a request bigger than its newly available reservation.
            if output != self.output {
                self.context
                    .settle(&id, Some(&lorca_agent::Usage::default()))?;
                return Err(
                    "Budget exhausted: the retry no longer fits the remaining allowance.".into(),
                );
            }
            *self.id.lock().unwrap() = Some(id);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::runtime::command_job;

    struct Scratch(Arc<App>, std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }
    fn app() -> (Scratch, Job) {
        let home = std::env::temp_dir().join(format!("lorca-budget-{}", uuid::Uuid::new_v4()));
        let app = App::load(Config {
            home: home.clone(),
            port: 0,
        })
        .unwrap();
        crate::identity::create(&app, Some("Budget Runner".into())).unwrap();
        let bot = app.state.lock().unwrap().bots[0].clone();
        let dm = app.dm_with(&bot.id, None).unwrap();
        let mut job = command_job(&app, &dm.meta.id, &bot.id, "card");
        job.kind = "turn".into();
        (Scratch(app, home), job)
    }
    fn configure(app: &Arc<App>, job: &Job, limits: Value) {
        serve(
            app,
            "budgets.set",
            &json!({"kind":"chat", "id":job.chat_id, "limits":limits}),
        )
        .unwrap();
    }
    fn view(app: &App, kind: &str, id: &str) -> BudgetSnapshot {
        app.budgets
            .snapshots(app)
            .into_iter()
            .find(|v| v.kind == kind && v.id == id)
            .unwrap()
    }

    #[tokio::test]
    async fn event_recovery_rearms_its_allowance_without_bypassing_the_inbox() {
        let (scratch, mut job) = app();
        let app = &scratch.0;
        job.kind = "event".into();
        configure(app, &job, json!({"max_tokens":0}));
        assert!(for_job(app, &job).is_err());
        serve(
            app,
            "budgets.set",
            &json!({"kind":"job", "id":job.id, "bot_id":job.bot_id, "limits":{"max_tokens":100}}),
        )
        .unwrap();
        let blocked = json!({"kind":"job", "id":job.id, "request_id":"event-recovery", "run":true});
        assert!(serve(app, "budgets.resume", &blocked)
            .unwrap_err()
            .contains("run:false"));
        assert_eq!(view(app, "job", &job.id).state, "budget_exhausted");
        let resumed =
            json!({"kind":"job", "id":job.id, "request_id":"event-recovery", "run":false});
        serve(app, "budgets.resume", &resumed).unwrap();
        let snapshot = view(app, "job", &job.id);
        assert_eq!(snapshot.state, "ready");
        assert_eq!(snapshot.job_kind.as_deref(), Some("event"));
        assert!(app.running_jobs.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_lower_run_limit_exposes_task_exhaustion_and_new_runs_keep_the_task_allowance() {
        let (scratch, mut job) = app();
        let app = &scratch.0;
        job.kind = "task".into();
        job.task_id = Some(format!("task-{}", uuid::Uuid::new_v4()));
        configure(app, &job, json!({"max_tokens":100}));
        let context = for_job(app, &job).unwrap();
        assert_eq!(
            view(app, "task", job.task_id.as_deref().unwrap())
                .limits
                .max_tokens,
            Some(100)
        );
        assert_eq!(view(app, "job", &job.id).limits.max_tokens, None);
        serve(
            app,
            "budgets.set",
            &json!({"kind":"job","id":job.id,"bot_id":job.bot_id,"limits":{"max_tokens":20}}),
        )
        .unwrap();
        let (id, _, _) = context.reserve(10, 10, None, Pricing::Unknown).unwrap();
        context.settle(&id, None).unwrap();
        let task = view(app, "task", job.task_id.as_deref().unwrap());
        assert_eq!(
            task.state, "budget_exhausted",
            "a partial reply cannot hide a lower run cap from the task lifecycle"
        );
        assert_eq!(task.usage.tokens, 20);
        serve(
            app,
            "budgets.resume",
            &json!({"kind":"task","id":task.id,"request_id":"task-recovery","run":false}),
        )
        .unwrap();
        job.id = format!("job-{}", uuid::Uuid::new_v4());
        assert!(for_job(app, &job).is_ok());
        assert_eq!(
            view(app, "task", &task.id).usage.tokens,
            20,
            "a fresh tasks.run does not replenish the task's counters"
        );
    }

    #[tokio::test]
    async fn reservations_share_capacity_and_recovery_does_not_erase_usage() {
        let (scratch, job) = app();
        let app = &scratch.0;
        configure(app, &job, json!({"max_tokens":100}));
        let context = for_job(app, &job).unwrap();
        let (id, output, _) = context.reserve(10, 500, None, Pricing::Unknown).unwrap();
        assert_eq!(
            output, 90,
            "the provider receives only the remaining output allowance"
        );
        assert!(context
            .reserve(10, 50, None, Pricing::Unknown)
            .unwrap_err()
            .contains("Budget exhausted"));
        let mut used = lorca_agent::Usage::default();
        used.input = 10;
        used.output = 10;
        context.settle(&id, Some(&used)).unwrap();
        assert_eq!(view(app, "job", &job.id).usage.tokens, 20);
        assert!(
            context.check().is_err(),
            "a smaller actual response does not silently restart blocked work"
        );
        serve(
            app,
            "budgets.set",
            &json!({"kind":"job", "id":job.id, "bot_id":job.bot_id, "limits":{"max_tokens":200}}),
        )
        .unwrap();
        assert!(
            context.check().is_err(),
            "changing a limit alone does not resume a held Job"
        );
        serve(
            app,
            "budgets.resume",
            &json!({"request_id": uuid::Uuid::new_v4().to_string(), "kind":"job", "id":job.id}),
        )
        .unwrap();
        assert!(context.check().is_ok());
        assert_eq!(view(app, "job", &job.id).usage.tokens, 20);
    }

    #[tokio::test]
    async fn checks_and_turns_share_the_routine_and_canonical_task_scope() {
        let (scratch, mut job) = app();
        let app = &scratch.0;
        let routine = crate::routines::create(
            app,
            &job.bot_id,
            "Check",
            "every 1h",
            "Look",
            Some("return false"),
            true,
        )
        .unwrap();
        job.routine_id = Some(routine.id.clone());
        job.task_id = Some(format!("task-{}", uuid::Uuid::new_v4()));
        let context = for_job(app, &job).unwrap();
        context
            .scope(async {
                let check = for_routine(app, &routine, &job.chat_id).unwrap();
                assert_eq!(
                    check.keys, context.keys,
                    "the nested check retains its parent's scopes without duplicating them"
                );
                check.connector_call().unwrap();
            })
            .await;
        for (kind, id) in [
            ("job", job.id.as_str()),
            ("task", job.task_id.as_deref().unwrap()),
            ("routine", routine.id.as_str()),
        ] {
            assert_eq!(view(app, kind, id).usage.connector_calls, 1);
        }
    }

    #[tokio::test]
    async fn runtime_cancels_work_and_zero_retry_limit_refuses_before_another_request() {
        let (scratch, job) = app();
        let app = &scratch.0;
        configure(
            app,
            &job,
            json!({"max_runtime_secs":1,"max_retries":0,"max_tokens":100}),
        );
        let context = for_job(app, &job).unwrap();
        let (id, output, _) = context.reserve(8, 8, None, Pricing::Unknown).unwrap();
        let permit = ModelPermit {
            context: context.clone(),
            id: Mutex::new(Some(id)),
            input: 8,
            output,
            rates: None,
            pricing: Pricing::Unknown,
            output_chars: std::sync::atomic::AtomicU64::new(0),
            outer: None,
        };
        let cancel = CancellationToken::new();
        permit.before_request(false, &cancel).await.unwrap();
        assert!(permit
            .before_request(true, &cancel)
            .await
            .unwrap_err()
            .contains("retry allowance"));
        assert_eq!(view(app, "job", &job.id).usage.model_calls, 1);
        assert_eq!(view(app, "job", &job.id).usage.estimated_calls, 1);
        serve(app, "budgets.resume", &json!({"request_id": uuid::Uuid::new_v4().to_string(), "kind":"job", "id":job.id, "renew":true})).unwrap();
        let result = context
            .run(&cancel, async {
                cancel.cancelled().await;
            })
            .await;
        assert!(result.unwrap_err().contains("runtime allowance"));
        assert!(cancel.is_cancelled());
        assert_eq!(view(app, "job", &job.id).state, "budget_exhausted");
    }

    #[tokio::test]
    async fn unknown_prices_need_a_measurable_bound_and_restart_keeps_encrypted_reservations() {
        let (scratch, job) = app();
        let app = &scratch.0;
        configure(app, &job, json!({"max_usd":5.0}));
        let context = for_job(app, &job).unwrap();
        assert!(context
            .reserve(10, 20, None, Pricing::Unknown)
            .unwrap_err()
            .contains("unknown pricing"));
        serve(app, "budgets.set", &json!({"kind":"job", "id":job.id, "bot_id":job.bot_id, "limits":{"max_usd":5.0,"max_tokens":100}})).unwrap();
        serve(
            app,
            "budgets.resume",
            &json!({"request_id": uuid::Uuid::new_v4().to_string(), "kind":"job", "id":job.id}),
        )
        .unwrap();
        context.reserve(10, 20, None, Pricing::Unknown).unwrap();
        context.start_runtime().unwrap();
        let bytes = app.store.runner_limits(PURPOSE).unwrap().unwrap();
        assert!(!bytes.windows(job.id.len()).any(|w| w == job.id.as_bytes()));
        assert!(serde_json::from_slice::<Ledger>(&bytes).is_err());
        app.budgets.clear();
        let held = view(app, "job", &job.id);
        assert_eq!(held.usage.tokens, 30);
        assert_eq!(held.usage.unknown_price_calls, 1);
        assert_eq!(held.state, "interrupted");
        assert!(
            for_job(app, &job).is_err(),
            "restart needs explicit recovery instead of granting a fresh allowance"
        );
    }

    #[tokio::test]
    async fn repeated_recovery_delivery_never_renews_consumption_twice() {
        let (scratch, job) = app();
        let app = &scratch.0;
        configure(app, &job, json!({"max_tokens":100}));
        let context = for_job(app, &job).unwrap();
        let (id, _, _) = context.reserve(10, 10, None, Pricing::Unknown).unwrap();
        context.settle(&id, None).unwrap();
        let resume = json!({"request_id":"same-delivery", "kind":"job", "id":job.id, "renew":true});
        let answer = serve(app, "budgets.resume", &resume).unwrap();
        let (id, _, _) = context.reserve(10, 10, None, Pricing::Unknown).unwrap();
        context.settle(&id, None).unwrap();
        assert_eq!(serve(app, "budgets.resume", &resume).unwrap(), answer);
        assert_eq!(view(app, "job", &job.id).usage.tokens, 20);
    }

    struct Answer {
        id: &'static str,
        usage: lorca_agent::Usage,
        cap: Arc<std::sync::atomic::AtomicU64>,
    }
    #[async_trait]
    impl Provider for Answer {
        fn provider_id(&self) -> &str {
            self.id
        }
        fn model_id(&self) -> &str {
            "m"
        }
        fn model_info(&self) -> Option<&'static lorca_agent::models::ModelInfo> {
            lorca_agent::models::find("deepseek", crate::providers::default_model("deepseek"))
        }
        async fn stream(
            &self,
            request: ModelRequest,
            _: CancellationToken,
        ) -> AssistantEventStream {
            self.cap.store(
                request.max_tokens.unwrap_or(0),
                std::sync::atomic::Ordering::SeqCst,
            );
            Box::pin(futures::stream::iter(vec![AssistantEvent::Done {
                stop_reason: lorca_agent::StopReason::Stop,
                usage: self.usage.clone(),
            }]))
        }
    }

    #[tokio::test]
    async fn inference_and_side_model_calls_settle_once_and_keep_price_labels_distinct() {
        let (scratch, job) = app();
        let app = &scratch.0;
        configure(app, &job, json!({"max_tokens":1000,"max_usd":1.0}));
        let context = for_job(app, &job).unwrap();
        context
            .scope(async {
                for id in ["deepseek", "chatgpt", "custom:local"] {
                    let mut usage = lorca_agent::Usage::default();
                    usage.input = 10;
                    usage.output = 10;
                    usage.cost.total = 0.01;
                    let cap = Arc::new(std::sync::atomic::AtomicU64::new(0));
                    let provider = wrap_provider(
                        app,
                        Arc::new(Answer {
                            id,
                            usage,
                            cap: cap.clone(),
                        }),
                    );
                    let request = ModelRequest {
                        system_prompt: String::new(),
                        messages: Vec::new(),
                        tools: Vec::new(),
                        cache_points: Vec::new(),
                        max_tokens: Some(20),
                        options: lorca_agent::RequestOptions::default()
                            .with_session_id(&job.chat_id),
                    };
                    provider
                        .stream(request, CancellationToken::new())
                        .await
                        .collect::<Vec<_>>()
                        .await;
                    assert_eq!(cap.load(std::sync::atomic::Ordering::SeqCst), 20);
                }
            })
            .await;
        let usage = view(app, "job", &job.id).usage;
        assert_eq!(usage.tokens, 60);
        assert_eq!(usage.api_cost_usd, 0.01);
        assert_eq!(usage.subscription_estimate_usd, 0.01);
        assert_eq!(usage.unknown_price_calls, 1);
        let chat = app.chat(&job.chat_id).unwrap().usage.unwrap();
        assert_eq!(
            (
                chat.api_cost_usd,
                chat.subscription_estimate_usd,
                chat.unknown_price_calls
            ),
            (0.01, 0.01, 1)
        );
    }
}
