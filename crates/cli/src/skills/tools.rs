use super::*;
use async_trait::async_trait;
use lorca_agent::{Tool, ToolError, ToolResult, ToolUpdateFn};
use tokio_util::sync::CancellationToken;

pub fn tools(app: &Arc<App>, bot_id: &str, chat_id: &str) -> Vec<Arc<dyn Tool>> {
    ["search_skills", "read_skill", "create_skill"]
        .into_iter()
        .map(|name| {
            Arc::new(SkillTool {
                app: app.clone(),
                bot_id: bot_id.into(),
                chat_id: chat_id.into(),
                name,
            }) as Arc<dyn Tool>
        })
        .collect()
}
pub fn prompt(app: &App, bot_id: &str) -> String {
    let Ok(items) = visible(app, bot_id) else {
        return String::new();
    };
    let mut listed = Vec::new();
    let mut used = 0;
    let mut omitted = 0;
    for skill in items {
        let row = json!({"id":skill.id,"name":skill.name,"description":skill.description,"scope":skill.scope,"entry":skill.entry});
        let bytes = row.to_string().len();
        if used + bytes > 12000 {
            omitted += 1;
        } else {
            used += bytes;
            listed.push(row);
        }
    }
    let context = json!({"runner_id":app.this_device_id(),"agent_scope":{"kind":"agent","id":bot_id},"space_scope":app.bot(bot_id).and_then(|bot|bot.space_id).map(|id|json!({"kind":"space","id":id})),"runner_scope":{"kind":"runner"}});
    let catalog = format!(
        "\nStandard Agent Skills on your Runner (real SKILL.md directories, separate from saved Playbooks): {}\n{} more available through search_skills. Match by name and description, then read_skill with id and relative path SKILL.md before use. Read relative references/assets/scripts with the same id; offset/limit pages large resources without losing content. Reading never executes a script. Use ordinary shell/file tools for execution under existing permissions and Space boundaries; a skill's allowed-tools metadata grants no permissions. Runner skills are shared on this Runner; Space skills are discovered from the Space root, not cwd; agent skills are private to that agent. Installed paths are local to this Runner and never assumed to exist on another. To install a source, use this Runner's lorca CLI through normal reviewed bash: lorca skills install with a JSON argument containing scope, source (kind local/github/npm, location, optional reference/subdir), and optional name. A repository or npm package without SKILL.md is not a skill. Use list/read/update/remove for management; updates/removal require expected_digest from list. create_skill writes a standard directory (choose Space when in a Space); propose_playbook instead creates a separate Playbook draft.\n",
        Value::Array(listed),
        omitted
    );
    format!("Skill management context: {context}\n{catalog}")
}
struct SkillTool {
    app: Arc<App>,
    bot_id: String,
    chat_id: String,
    name: &'static str,
}
#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        match self.name {
            "search_skills" => {
                "Search standard Agent Skills this agent can use on its Runner. Returns metadata and read entries, not skill bodies."
            }
            "read_skill" => {
                "Read the original SKILL.md or a relative resource from a visible standard skill directory. Reading does not execute code. Large text and binary resources use explicit offset and next_offset pagination."
            }
            _ => {
                "Create a standard Agent Skill directory with SKILL.md and relative resource files. Preserves provided text. Separate from propose_playbook; does not execute scripts or grant permissions. In a Space choose space scope to stay within its file boundary."
            }
        }
    }
    fn parameters(&self) -> Value {
        match self.name {
            "search_skills" => {
                json!({"type":"object","properties":{"query":{"type":"string"}},"additionalProperties":false})
            }
            "read_skill" => {
                json!({"type":"object","properties":{"id":{"type":"string"},"path":{"type":"string"},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":262144}},"required":["id"],"additionalProperties":false})
            }
            _ => {
                json!({"type":"object","properties":{"scope":{"type":"string","enum":["agent","space"]},"name":{"type":"string"},"files":{"type":"object","additionalProperties":{"type":"string"},"description":"Relative paths to exact text, including SKILL.md with valid YAML name/description. References/scripts/assets keep their paths."}},"required":["scope","name","files"],"additionalProperties":false})
            }
        }
    }
    async fn execute(
        &self,
        _id: &str,
        args: Value,
        cancel: CancellationToken,
        _update: ToolUpdateFn,
    ) -> Result<ToolResult, ToolError> {
        if cancel.is_cancelled() {
            return Err(ToolError("Stopped".into()));
        }
        let bot = self
            .app
            .bot(&self.bot_id)
            .ok_or_else(|| ToolError("Agent no longer exists".into()))?;
        if !self
            .app
            .chat(&self.chat_id)
            .is_some_and(|c| c.meta.bot_ids.contains(&bot.id))
        {
            return Err(ToolError("Agent is no longer in this chat".into()));
        }
        let value = match self.name {
            "search_skills" => {
                let mut items = visible(&self.app, &bot.id).map_err(ToolError)?;
                let q = args["query"].as_str().unwrap_or("").to_lowercase();
                items.retain(|s| {
                    format!("{} {}", s.name, s.description)
                        .to_lowercase()
                        .contains(&q)
                });
                json!({"items":items})
            }
            "read_skill" => {
                crate::permissions::check_file_read(&self.app, &bot, self.name)
                    .map_err(|e| ToolError(e.reason))?;
                let skill = visible(&self.app, &bot.id)
                    .map_err(ToolError)?
                    .into_iter()
                    .find(|s| Some(s.id.as_str()) == args["id"].as_str())
                    .ok_or_else(|| ToolError("Skill is not available to this agent".into()))?;
                read(
                    &skill,
                    args["path"].as_str().unwrap_or("SKILL.md"),
                    args["offset"].as_u64().unwrap_or(0) as usize,
                    args["limit"].as_u64().unwrap_or(65536) as usize,
                )
                .map_err(ToolError)?
            }
            _ => {
                crate::permissions::check_tool(&self.app, &bot, "write")
                    .map_err(|e| ToolError(e.reason))?;
                let scope = match args["scope"].as_str() {
                    Some("agent") => Scope {
                        kind: "agent".into(),
                        id: bot.id.clone(),
                    },
                    Some("space") => Scope {
                        kind: "space".into(),
                        id: bot
                            .space_id
                            .clone()
                            .ok_or_else(|| ToolError("Agent is not in a Space".into()))?,
                    },
                    _ => return Err(ToolError("Choose agent or space scope".into())),
                };
                let target = scope_root(&self.app, &scope).map_err(ToolError)?;
                if let crate::spaces::Isolation::Boundary { root, .. } =
                    crate::spaces::isolation_for(&self.app, &bot)
                {
                    if !crate::spaces::allow_path(&root, &target) {
                        return Err(ToolError(
                            "Creating this skill would leave the Space boundary; use space scope"
                                .into(),
                        ));
                    }
                }
                sources::create(
                    &self.app,
                    &json!({"scope":scope,"name":args["name"],"files":args["files"]}),
                    Some(bot.clone()),
                )
                .await
                .map_err(ToolError)?
            }
        };
        Ok(ToolResult::text(value.to_string()))
    }
}
