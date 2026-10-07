//! User-owned capability policies, checked before review and again at execution.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::model::Bot;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Read,
    Draft,
    Write,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Read => "read",
            Self::Draft => "draft",
            Self::Write => "write",
        })
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemAccess {
    None,
    Read,
    #[default]
    Write,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConnectionPermissions {
    /// Independent grants. An empty set denies the connection entirely.
    #[serde(default)]
    pub capabilities: BTreeSet<Capability>,
    /// Original MCP tool names, scoped to this connection instance. None permits all names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<BTreeSet<String>>,
}

/// None on a Bot means it has the account's existing access. Explicit empty allowlists deny
/// everything in their scope; unknown tools never inherit a listed tool's grant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BotPermissions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connections: Option<BTreeMap<String, ConnectionPermissions>>,
    /// Local CLI tool names. Plugin names are selected under their connection instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<BTreeSet<String>>,
    #[serde(default)]
    pub filesystem: FilesystemAccess,
    #[serde(default = "enabled")]
    pub shell: bool,
}

fn enabled() -> bool {
    true
}

impl Default for BotPermissions {
    fn default() -> Self {
        Self { connections: None, tools: None, filesystem: FilesystemAccess::Write, shell: true }
    }
}

impl BotPermissions {
    pub fn validate(&self) -> Result<(), String> {
        let valid = |name: &str| !name.trim().is_empty() && name.len() <= 256 && name == name.trim();
        if self.tools.as_ref().is_some_and(|tools| tools.len() > 1000 || tools.iter().any(|name| !valid(name))) {
            return Err("Tool allowlists need at most 1000 non-empty exact names, each under 256 bytes.".into());
        }
        if let Some(connections) = &self.connections {
            if connections.len() > 1000 || connections.keys().any(|id| !valid(id)) {
                return Err("Connection allowlists need at most 1000 non-empty instance IDs.".into());
            }
            if connections.values().any(|grant| grant.tools.as_ref().is_some_and(|tools| tools.len() > 1000 || tools.iter().any(|name| !valid(name)))) {
                return Err("Connection tool allowlists need at most 1000 non-empty exact names.".into());
            }
        }
        Ok(())
    }

    fn local_denial(&self, tool: &str) -> Option<String> {
        if self.tools.as_ref().is_some_and(|tools| !tools.contains(tool)) {
            return Some(format!("the local tool {tool} is excluded from its tool allowlist"));
        }
        if matches!(tool, "bash" | "bash_input" | "bash_output") && !self.shell {
            return Some("shell access is disabled".into());
        }
        if matches!(tool, "read" | "grep" | "find" | "ls") && self.filesystem == FilesystemAccess::None {
            return Some("filesystem access is disabled".into());
        }
        if matches!(tool, "write" | "edit") && self.filesystem != FilesystemAccess::Write {
            return Some("filesystem writes are disabled".into());
        }
        None
    }

    fn connection_denial(&self, connection: &str, tool: &str, capability: Option<Capability>) -> Option<String> {
        let connections = self.connections.as_ref()?;
        let Some(grant) = connections.get(connection) else {
            return Some(format!("connection {connection} is excluded from its connection allowlist"));
        };
        if grant.tools.as_ref().is_some_and(|tools| !tools.contains(tool)) {
            return Some(format!("tool {tool} is excluded from the allowlist for connection {connection}"));
        }
        if grant.capabilities.is_empty() || capability.is_some_and(|capability| !grant.capabilities.contains(&capability)) {
            return Some(format!("{} access to connection {connection} is disabled", capability.map(|c| c.to_string()).unwrap_or_else(|| "all".into())));
        }
        None
    }
}

#[derive(Debug, Clone)]
pub struct AccessDenied {
    pub tool: String,
    pub connection_id: Option<String>,
    pub capability: Option<Capability>,
    pub reason: String,
}

impl fmt::Display for AccessDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Access refused for {}: {}. The user can change this bot's Access settings in its profile. Auto-review and Always allow cannot override this restriction.", self.tool, self.reason)
    }
}

/// Every call reads the current profile, so a restriction takes effect during an active turn.
pub fn check_tool(app: &Arc<App>, bot: &Bot, tool: &str) -> Result<(), AccessDenied> {
    let current = app.bot(&bot.id);
    let reason = match current {
        Some(current) if current.runner_id != bot.runner_id => Some("the bot was reassigned to another Runner".into()),
        Some(bot) => bot.permissions.as_ref().and_then(|policy| policy.local_denial(tool)),
        None => Some("the bot no longer exists".into()),
    };
    match reason {
        Some(reason) => Err(AccessDenied { tool: tool.into(), connection_id: None, capability: None, reason }),
        None => Ok(()),
    }
}

pub fn check_connection(app: &Arc<App>, bot: &Bot, connection: &str, tool: &str, capability: Capability) -> Result<(), AccessDenied> {
    check_connection_inner(app, bot, connection, tool, Some(capability))
}

/// Checks instance and tool selection before starting a server to inspect its annotations.
pub fn check_connection_tool(app: &Arc<App>, bot: &Bot, connection: &str, tool: &str) -> Result<(), AccessDenied> {
    check_connection_inner(app, bot, connection, tool, None)
}

/// A stable hash of the effective policy and Runner assignment for staged execution. It does
/// not replace a fresh authorization check, nor include unrelated name/look/model changes.
pub fn policy_fingerprint(app: &Arc<App>, bot: &Bot) -> Result<String, AccessDenied> {
    use sha2::{Digest, Sha256};
    let bot = app.bot(&bot.id).ok_or_else(|| AccessDenied {
        tool: "policy".into(),
        connection_id: None,
        capability: None,
        reason: "the bot no longer exists".into(),
    })?;
    let value = serde_json::json!({ "v": 1, "bot_id": bot.id, "runner_id": bot.runner_id, "permissions": bot.permissions.unwrap_or_default() });
    let digest = Sha256::digest(serde_json::to_vec(&value).expect("a permission policy serializes"));
    Ok(data_encoding::HEXLOWER.encode(&digest))
}

fn check_connection_inner(app: &Arc<App>, bot: &Bot, connection: &str, tool: &str, capability: Option<Capability>) -> Result<(), AccessDenied> {
    let reason = match app.bot(&bot.id) {
        Some(current) if current.runner_id != bot.runner_id => Some("the bot was reassigned to another Runner".into()),
        Some(bot) => bot.permissions.as_ref().and_then(|policy| policy.connection_denial(connection, tool, capability)),
        None => Some("the bot no longer exists".into()),
    };
    match reason {
        Some(reason) => Err(AccessDenied { tool: tool.into(), connection_id: Some(connection.into()), capability, reason }),
        None => Ok(()),
    }
}

/// A rolling-upgrade roster that omits policies cannot silently remove restrictions. Users
/// restore full access by writing an explicit default policy, never by dropping the field.
pub fn keep_policies(current: &[Bot], incoming: &mut [Bot]) -> bool {
    let mut kept = false;
    for bot in incoming {
        if bot.permissions.is_none() {
            if let Some(policy) = current.iter().find(|old| old.id == bot.id).and_then(|old| old.permissions.clone()) {
                bot.permissions = Some(policy);
                kept = true;
            }
        }
    }
    kept
}

/// Editing access acknowledges the outstanding requests; it never resumes refused calls.
pub fn dismiss_requests(app: &Arc<App>, bot_id: &str) {
    use crate::model::{Author, Body};
    let chats: Vec<String> =
        app.state.lock().unwrap().chats.iter().filter(|chat| chat.meta.bot_ids.iter().any(|id| id == bot_id)).map(|chat| chat.meta.id.clone()).collect();
    for chat_id in chats {
        for mut message in app.store.page(&chat_id, None, 200).map(|(messages, _)| messages).unwrap_or_default() {
            if message.author != (Author::Bot { bot_id: bot_id.into() }) {
                continue;
            }
            if let Body::Permission { tool, decision, .. } = &mut message.body {
                if tool == "access" && decision == "pending" {
                    *decision = "dismissed".into();
                    app.upsert_message(message, true);
                }
            }
        }
    }
}

#[cfg(feature = "runner")]
pub fn refuse(app: &Arc<App>, chat_id: &str, bot: &Bot, denied: AccessDenied) -> lorca_agent::BeforeToolCallResult {
    use crate::model::{Author, Body, Message};
    let reason = denied.to_string();
    // One waiting request per missing grant; repeated calls never flood the user's chat.
    let arguments = serde_json::json!({ "connection_id": denied.connection_id, "requested_tool": denied.tool, "capability": denied.capability });
    let recent = app.store.page(chat_id, None, 200).map(|(messages, _)| messages).unwrap_or_default();
    let duplicate = recent.iter().any(|message| {
        message.author == (Author::Bot { bot_id: bot.id.clone() })
            && matches!(&message.body, Body::Permission { tool, decision, arguments: existing, .. } if tool == "access" && decision == "pending" && *existing == arguments)
    });
    if !duplicate {
        let message = Message::new(
            chat_id,
            Author::Bot { bot_id: bot.id.clone() },
            Body::Permission {
                plugin_id: denied.connection_id.unwrap_or_else(|| "computer".into()),
                plugin_name: "Bot access".into(),
                tool: "access".into(),
                summary: format!("{} needs access to {}", bot.name, denied.tool),
                arguments,
                decision: "pending".into(),
                reason: Some(reason.clone()),
                command: None,
                rule: None,
                code: None,
                link: None,
            },
        );
        app.upsert_message(message.clone(), true);
        crate::push::permission(app, &message);
    }
    crate::local_review::blocked(reason)
}

/// The available local names shown by the profile editor. Extensions can supply other exact
/// names through the same allowlist and check_tool entry point.
pub const LOCAL_TOOLS: &[&str] = &[
    "codemode",
    "read",
    "write",
    "edit",
    "grep",
    "find",
    "ls",
    "bash",
    "bash_input",
    "bash_output",
    "memory_update",
    "memory_log",
    "recall",
    "list_teammates",
    "message_bot",
    "create_bot",
    "edit_bot",
    "routines",
    "stage_review",
    "search_plugins",
    "install_plugin",
    "connect_plugin",
];

#[cfg(feature = "runner")]
pub(crate) mod guarded {
    use super::*;
    use async_trait::async_trait;
    use lorca_agent::{Tool, ToolError, ToolExecutionMode, ToolResult, ToolRunner, ToolUpdateFn};
    use serde_json::Value;
    use tokio_util::sync::CancellationToken;

    struct GuardedTool {
        app: Arc<App>,
        bot: Bot,
        chat_id: String,
        tool: Arc<dyn Tool>,
    }

    pub fn tools(app: &Arc<App>, bot: &Bot, chat_id: &str, tools: Vec<Arc<dyn Tool>>) -> Vec<Arc<dyn Tool>> {
        tools.into_iter().map(|tool| Arc::new(GuardedTool { app: app.clone(), bot: bot.clone(), chat_id: chat_id.into(), tool }) as Arc<dyn Tool>).collect()
    }

    impl GuardedTool {
        fn check(&self) -> Result<(), ToolError> {
            check_tool(&self.app, &self.bot, self.name()).map_err(|denied| {
                let result = refuse(&self.app, &self.chat_id, &self.bot, denied);
                ToolError(result.reason.unwrap_or_default())
            })
        }
    }

    #[async_trait]
    impl Tool for GuardedTool {
        fn name(&self) -> &str {
            self.tool.name()
        }
        fn label(&self) -> &str {
            self.tool.label()
        }
        fn description(&self) -> &str {
            self.tool.description()
        }
        fn parameters(&self) -> Value {
            self.tool.parameters()
        }
        fn output_schema(&self) -> Option<Value> {
            self.tool.output_schema()
        }
        fn execution_mode(&self) -> Option<ToolExecutionMode> {
            self.tool.execution_mode()
        }
        fn prepare_arguments(&self, args: Value) -> Value {
            self.tool.prepare_arguments(args)
        }
        async fn execute(&self, id: &str, args: Value, cancel: CancellationToken, update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
            self.check()?;
            self.tool.execute(id, args, cancel, update).await
        }
        async fn execute_with(
            &self,
            id: &str,
            args: Value,
            cancel: CancellationToken,
            update: ToolUpdateFn,
            runner: &dyn ToolRunner,
        ) -> Result<ToolResult, ToolError> {
            self.check()?;
            self.tool.execute_with(id, args, cancel, update, runner).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Scratch(Arc<App>, std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }
    fn scratch() -> (Scratch, Bot, String) {
        let home = std::env::temp_dir().join(format!("lorca-permissions-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        crate::identity::create(&app, Some("Policy Runner".into())).unwrap();
        let bot = app.state.lock().unwrap().bots[0].clone();
        let chat_id = app.dm_with(&bot.id, None).unwrap().meta.id;
        (Scratch(app, home), bot, chat_id)
    }

    fn inbox_policy() -> BotPermissions {
        serde_json::from_value(json!({
            "connections": {"gmail-work": {"capabilities": ["read", "draft"], "tools": ["list_messages", "create_draft"]}},
            "filesystem": "read", "shell": false
        }))
        .unwrap()
    }

    #[test]
    fn connection_instances_tools_and_capabilities_are_independent_grants() {
        let policy = inbox_policy();
        assert_eq!(policy.connection_denial("gmail-work", "list_messages", Some(Capability::Read)), None);
        assert_eq!(policy.connection_denial("gmail-work", "create_draft", Some(Capability::Draft)), None);
        assert!(policy.connection_denial("gmail-work", "create_draft", Some(Capability::Write)).is_some());
        assert!(policy.connection_denial("gmail-personal", "list_messages", Some(Capability::Read)).is_some());
        assert!(policy.connection_denial("gmail-work", "send_message", Some(Capability::Draft)).is_some());
        let mut only_write = inbox_policy();
        only_write.connections.as_mut().unwrap().get_mut("gmail-work").unwrap().capabilities = BTreeSet::from([Capability::Write]);
        assert!(only_write.connection_denial("gmail-work", "list_messages", Some(Capability::Read)).is_some(), "write does not imply read");
    }

    #[test]
    fn shell_and_filesystem_gates_apply_even_to_readonly_commands() {
        let policy = inbox_policy();
        for tool in ["bash", "bash_input", "bash_output", "write", "edit"] {
            assert!(policy.local_denial(tool).is_some(), "{tool}");
        }
        for tool in ["read", "grep", "find", "ls"] {
            assert_eq!(policy.local_denial(tool), None, "{tool}");
        }
        let none = BotPermissions { filesystem: FilesystemAccess::None, tools: Some(BTreeSet::new()), ..Default::default() };
        assert!(none.local_denial("codemode").is_some());
        assert!(none.local_denial("read").is_some());
        let staging = BotPermissions {
            tools: Some(BTreeSet::from(["stage_review".into()])),
            connections: Some(BTreeMap::new()),
            filesystem: FilesystemAccess::None,
            shell: false,
        };
        assert_eq!(staging.local_denial("stage_review"), None);
        assert!(staging.local_denial("bash").is_some(), "permission to stage a review does not authorize execution");
        assert!(staging.connection_denial("mail-work", "send_message", Some(Capability::Write)).is_some());
        assert!(serde_json::from_value::<BotPermissions>(json!({"filesystem": "sandbox"})).is_err());
        assert!(serde_json::from_value::<BotPermissions>(json!({"shelll": true})).is_err());
        assert!(serde_json::from_value::<BotPermissions>(json!({"connections": {"mail": {"capabilities": ["admin"]}}})).is_err());
    }

    #[test]
    fn a_running_turn_reads_revocation_and_fingerprints_effective_access() {
        let (scratch, snapshot, _) = scratch();
        let app = &scratch.0;
        assert!(check_tool(app, &snapshot, "bash").is_ok());
        let initial = policy_fingerprint(app, &snapshot).unwrap();
        app.update_bot(&snapshot.id, |bot| {
            bot.permissions = Some(BotPermissions::default());
            bot.name = "Renamed".into();
        })
        .unwrap();
        assert_eq!(policy_fingerprint(app, &snapshot).unwrap(), initial);
        app.update_bot(&snapshot.id, |bot| bot.permissions = Some(inbox_policy())).unwrap();
        assert!(check_tool(app, &snapshot, "bash").is_err());
        assert!(check_connection(app, &snapshot, "gmail-personal", "list_messages", Capability::Read).is_err());
        assert_ne!(policy_fingerprint(app, &snapshot).unwrap(), initial);
        let reopened = App::load(crate::config::Config { home: scratch.1.clone(), port: 0 }).unwrap();
        assert_eq!(reopened.bot(&snapshot.id).unwrap().permissions, Some(inbox_policy()), "the persisted profile retains policy");
        app.state.lock().unwrap().bots.iter_mut().find(|bot| bot.id == snapshot.id).unwrap().runner_id = "another-runner".into();
        assert!(check_tool(app, &snapshot, "read").unwrap_err().reason.contains("reassigned"));
    }

    #[test]
    fn an_old_roster_cannot_drop_an_explicit_policy() {
        let (_scratch, mut bot, _) = scratch();
        bot.permissions = Some(inbox_policy());
        let mut old = bot.clone();
        old.permissions = None;
        assert!(keep_policies(&[bot.clone()], std::slice::from_mut(&mut old)));
        assert_eq!(old.permissions, bot.permissions);
        old.permissions = Some(BotPermissions::default());
        assert!(!keep_policies(&[bot], std::slice::from_mut(&mut old)), "a user's explicit full policy stands");
    }

    #[tokio::test]
    async fn api_policy_edits_reject_null_and_invalid_fields_without_changing_access() {
        let (scratch, bot, _) = scratch();
        let app = &scratch.0;
        crate::api::dispatch(app, "bots.update", json!({"id": bot.id, "permissions": inbox_policy()})).await.unwrap();
        for invalid in [json!(null), json!({"connections": []}), json!({"shell": "false"}), json!({"tools": [""]}), json!({"unexpected": true})] {
            assert!(crate::api::dispatch(app, "bots.update", json!({"id": bot.id, "permissions": invalid})).await.is_err());
            assert_eq!(app.bot(&bot.id).unwrap().permissions, Some(inbox_policy()));
        }
        crate::api::dispatch(app, "bots.update", json!({"id": bot.id, "name": "Inbox"})).await.unwrap();
        assert_eq!(app.bot(&bot.id).unwrap().permissions, Some(inbox_policy()));
    }

    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nested_file_writes_and_shell_calls_cannot_execute_after_revocation() {
        use lorca_agent::codemode::{CodemodeOptions, CodemodeTool};
        use lorca_agent::{DirectRunner, ToolUpdateFn};
        use tokio_util::sync::CancellationToken;
        let (scratch, bot, chat_id) = scratch();
        let app = &scratch.0;
        // Build the execution catalog before the user changes the policy, like an active turn.
        let tools = guarded::tools(app, &bot, &chat_id, lorca_agent::tools::coding_tools(scratch.1.clone()));
        let catalog = crate::plugins::mcp::bot_catalog(app, &bot, &chat_id, tools);
        app.update_bot(&bot.id, |bot| bot.permissions = Some(inbox_policy())).unwrap();
        let script =
            "await tools.write({path:'forbidden.txt',content:'write happened'}); await tools.bash({command:'touch forbidden-shell',description:'touch'});";
        let codemode = CodemodeTool::new(catalog, CodemodeOptions::default());
        let run = codemode.run_script("nested", script, CancellationToken::new(), &DirectRunner).await.unwrap();
        assert!(run.result.is_error, "{}", run.result.text_content());
        assert!(run.result.text_content().contains("filesystem writes are disabled"));
        assert!(!scratch.1.join("forbidden.txt").exists());
        assert!(!scratch.1.join("forbidden-shell").exists());
        let tools = guarded::tools(app, &bot, &chat_id, lorca_agent::tools::coding_tools(scratch.1.clone()));
        let bash = tools.iter().find(|tool| tool.name() == "bash").unwrap();
        let update: ToolUpdateFn = Arc::new(|_| {});
        let refused =
            bash.execute("direct", json!({"command": "touch forbidden-shell", "description": "Touch"}), CancellationToken::new(), update).await.unwrap_err();
        assert!(refused.0.contains("shell access is disabled"));
        assert!(!scratch.1.join("forbidden-shell").exists());
    }

    #[cfg(feature = "runner")]
    #[tokio::test]
    async fn access_requests_route_to_the_user_and_never_add_grants_or_always_allow() {
        use crate::model::{AutoReviewRule, Body};
        let (scratch, bot, chat_id) = scratch();
        let app = &scratch.0;
        app.update_bot(&bot.id, |bot| bot.permissions = Some(inbox_policy())).unwrap();
        app.add_auto_review_rule(AutoReviewRule {
            id: "always".into(),
            text: "allow everything".into(),
            behavior: "allow".into(),
            tool: Some("gmail-personal/send_message".into()),
        });
        let denied = check_connection(app, &bot, "gmail-personal", "send_message", Capability::Write).unwrap_err();
        assert!(refuse(app, &chat_id, &bot, denied.clone()).block);
        refuse(app, &chat_id, &bot, denied);
        let requests = app.store.page(&chat_id, None, 200).unwrap().0;
        assert_eq!(requests.len(), 1, "repeated missing grant makes one request");
        let card = &requests[0];
        assert!(matches!(&card.body, Body::Permission { tool, decision, rule: None, .. } if tool == "access" && decision == "pending"));
        for decision in ["allow", "always"] {
            let response = crate::api::dispatch(app, "chats.permission", json!({"chat_id": chat_id, "message_id": card.id, "decision": decision})).await;
            assert!(response.unwrap_err().contains("cannot grant permissions"));
        }
        assert_eq!(app.auto_review().rules.len(), 1);
        assert!(check_connection(app, &bot, "gmail-personal", "send_message", Capability::Write).is_err());
        crate::api::dispatch(app, "chats.permission", json!({"chat_id": chat_id, "message_id": card.id, "decision": "deny"})).await.unwrap();
        assert!(matches!(app.message(&chat_id, &card.id).unwrap().body, Body::Permission { decision, .. } if decision == "denied"));
    }
}
