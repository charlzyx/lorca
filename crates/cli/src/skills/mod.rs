use crate::app::App;
use gray_matter::{Matter, engine::YAML};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use unicode_normalization::UnicodeNormalization;
use walkdir::WalkDir;
mod sources;
#[cfg(feature = "runner")]
mod tools;
#[cfg(feature = "runner")]
pub use tools::{prompt, tools};

const MAX_FILE: u64 = 16 * 1024 * 1024;
const MAX_PACKAGE: u64 = 128 * 1024 * 1024;
const MAX_FILES: usize = 10000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Scope {
    pub kind: String,
    #[serde(default)]
    pub id: String,
}
impl Scope {
    fn runner() -> Self {
        Self {
            kind: "runner".into(),
            id: String::new(),
        }
    }
    fn key(&self) -> String {
        format!("{}:{}", self.kind, self.id)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub kind: String,
    pub location: String,
    #[serde(default)]
    pub reference: Option<String>,
    #[serde(default)]
    pub subdir: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Installed {
    scope: Scope,
    name: String,
    source: Source,
    revision: String,
    digest: String,
}
#[derive(Default, Serialize, Deserialize)]
struct Registry {
    #[serde(default)]
    installed: BTreeMap<String, Installed>,
    #[serde(default)]
    selections: BTreeMap<String, BTreeSet<String>>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Skill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub scope: Scope,
    pub entry: String,
    #[serde(skip_serializing)]
    pub metadata: Value,
    pub digest: String,
    pub source: Option<Source>,
    pub revision: Option<String>,
    #[serde(skip)]
    root: PathBuf,
}
fn registry(app: &App) -> Result<Registry, String> {
    let path = app.config.home.join("skills-index.json");
    if !path.exists() {
        return Ok(Registry::default());
    }
    serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Skill index is invalid: {e}"))
}
fn save_registry(app: &App, index: &Registry) -> Result<(), String> {
    crate::config::write_json_private(&app.config.home.join("skills-index.json"), index)
        .map_err(|e| e.to_string())
}
fn current_runner(app: &App) -> Result<String, String> {
    app.this_device_id()
        .filter(|id| {
            app.device(id)
                .is_some_and(|d| matches!(d.os.as_str(), "macos" | "linux" | "windows"))
        })
        .ok_or("Skills are managed on a Runner".into())
}
pub fn scope_root(app: &App, scope: &Scope) -> Result<PathBuf, String> {
    let runner = current_runner(app)?;
    match scope.kind.as_str() {
        "runner" if scope.id.is_empty() || scope.id == runner => Ok(app.config.home.join("skills")),
        "agent" => {
            let bot = app.bot(&scope.id).ok_or("Unknown agent")?;
            if bot.runner_id != runner {
                return Err("Skill scope belongs to another Runner".into());
            }
            Ok(app.config.home.join("agent-skills").join(&bot.id))
        }
        "space" => {
            let space = app
                .state
                .lock()
                .unwrap()
                .spaces
                .iter()
                .find(|s| s.id == scope.id)
                .cloned()
                .ok_or("Unknown Space")?;
            if space.runner_id != runner {
                return Err("Skill scope belongs to another Runner".into());
            }
            let root = space
                .sandbox
                .as_ref()
                .and_then(|s| s.root.as_ref())
                .map(|s| crate::spaces::expand_path(s))
                .unwrap_or_else(|| app.config.home.join("spaces").join(&space.id));
            let skills = root.join(".agents").join("skills");
            if !crate::spaces::allow_path(&root, &skills) {
                return Err("Skills directory leaves the Space root".into());
            }
            Ok(skills)
        }
        _ => Err("Choose runner, space or agent scope".into()),
    }
}
fn safe_relative(value: &str) -> Result<&Path, String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\\')
        || value.contains('\0')
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("Use a relative path inside the skill directory".into());
    }
    Ok(path)
}
fn id_for(runner: &str, scope: &Scope, name: &str) -> String {
    format!(
        "skill-{:x}",
        Sha256::digest(format!("{runner}:{}:{name}", scope.key()).as_bytes())
    )
}
fn package_files(root: &Path) -> Result<Vec<(PathBuf, u64)>, String> {
    let canonical = root.canonicalize().map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    let mut size = 0;
    for entry in WalkDir::new(&canonical)
        .follow_links(true)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| !matches!(e.file_name().to_str(), Some(".git" | ".hg" | ".svn")))
    {
        let entry = entry.map_err(|e| e.to_string())?;
        let real = entry.path().canonicalize().map_err(|e| e.to_string())?;
        if !real.starts_with(&canonical) {
            return Err("Skill symlink leaves the package; nothing was installed".into());
        }
        if entry.file_type().is_dir() {
            continue;
        }
        if !entry.file_type().is_file() {
            return Err("Skills may contain only files and directories".into());
        }
        let len = entry.metadata().map_err(|e| e.to_string())?.len();
        size += len;
        if len > MAX_FILE || size > MAX_PACKAGE || files.len() >= MAX_FILES {
            return Err("Skill exceeds the Runner's import budget (16 MiB/file, 128 MiB/package, 10000 files); nothing was truncated".into());
        }
        files.push((
            entry
                .path()
                .strip_prefix(&canonical)
                .map_err(|e| e.to_string())?
                .to_path_buf(),
            len,
        ));
    }
    Ok(files)
}
fn digest(root: &Path) -> Result<String, String> {
    let mut hash = Sha256::new();
    for (path, _) in package_files(root)? {
        hash.update(path.to_string_lossy().as_bytes());
        hash.update([0]);
        hash.update(std::fs::read(root.join(path)).map_err(|e| e.to_string())?);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn metadata(root: &Path) -> Result<Value, String> {
    let file = root.join("SKILL.md");
    if !file
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(root.canonicalize().map_err(|e| e.to_string())?)
    {
        return Err("SKILL.md leaves its skill directory".into());
    }
    if std::fs::metadata(&file).map_err(|e| e.to_string())?.len() > MAX_FILE {
        return Err("SKILL.md exceeds the read budget; it was not truncated".into());
    }
    let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let parsed = Matter::<YAML>::new()
        .parse::<Value>(&text)
        .map_err(|e| format!("Invalid SKILL.md YAML: {e}"))?;
    let mut data = parsed
        .data
        .filter(Value::is_object)
        .ok_or("SKILL.md requires YAML frontmatter")?;
    let allowed = [
        "name",
        "description",
        "license",
        "compatibility",
        "metadata",
        "allowed-tools",
    ];
    if data
        .as_object()
        .unwrap()
        .keys()
        .any(|key| !allowed.contains(&key.as_str()))
    {
        return Err("Unsupported frontmatter fields; keep extension fields under metadata. Original files were not changed.".into());
    }
    let name: String = data["name"]
        .as_str()
        .ok_or("Skill name is required")?
        .trim()
        .nfkc()
        .collect();
    let directory: String = root
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Invalid directory name")?
        .nfkc()
        .collect();
    if name.is_empty()
        || name.chars().count() > 64
        || name.starts_with('-')
        || name.ends_with('-')
        || name.contains("--")
        || !name.chars().all(|c| c == '-' || c.is_alphanumeric())
        || name != name.to_lowercase()
        || directory != name
    {
        return Err("Skill name must match its directory and contain 1–64 lowercase letters, numbers or non-consecutive hyphens".into());
    }
    data["name"] = json!(name);
    let description = data["description"]
        .as_str()
        .ok_or("Skill description is required")?;
    if description.trim().is_empty() || description.chars().count() > 1024 {
        return Err("Skill description requires 1–1024 characters".into());
    }
    if let Some(value) = data.get("compatibility") {
        if !value
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.chars().count() <= 500)
        {
            return Err("compatibility requires 1–500 characters".into());
        }
    }
    for key in ["license", "allowed-tools"] {
        if data.get(key).is_some_and(|v| !v.is_string()) {
            return Err(format!("{key} must be a string"));
        }
    }
    if let Some(value) = data.get("metadata") {
        if !value
            .as_object()
            .is_some_and(|v| v.values().all(Value::is_string))
        {
            return Err("metadata must map strings to strings".into());
        }
    }
    Ok(data)
}
fn discover_scope(
    app: &App,
    scope: &Scope,
    index: &Registry,
    verify_package: bool,
) -> Result<(Vec<Skill>, Vec<Value>), String> {
    let root = scope_root(app, scope)?;
    if !root.exists() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut items = Vec::new();
    let mut errors = Vec::new();
    let allowed_root = root.canonicalize().map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let directory = entry.path();
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if !directory.join("SKILL.md").is_file() {
            continue;
        }
        if !directory
            .canonicalize()
            .is_ok_and(|p| p.starts_with(&allowed_root))
        {
            errors.push(json!({"directory":entry.file_name(),"error":"Skill directory leaves its scope root"}));
            continue;
        }
        match metadata(&directory).and_then(|m| {
            let id = id_for(&current_runner(app)?, scope, m["name"].as_str().unwrap());
            let hash = if verify_package {
                digest(&directory)?
            } else {
                index
                    .installed
                    .get(&id)
                    .map(|entry| entry.digest.clone())
                    .unwrap_or_default()
            };
            Ok((m, hash))
        }) {
            Ok((m, hash)) => {
                let name = m["name"].as_str().unwrap().to_owned();
                let id = id_for(&current_runner(app)?, scope, &name);
                let installed = index.installed.get(&id);
                items.push(Skill {
                    entry: format!("skill://{id}/SKILL.md"),
                    id,
                    name,
                    description: m["description"].as_str().unwrap().into(),
                    metadata: m,
                    scope: scope.clone(),
                    digest: hash,
                    root: directory,
                    source: installed.map(|i| i.source.clone()),
                    revision: installed.map(|i| i.revision.clone()),
                });
            }
            Err(error) => errors.push(json!({"directory":directory.file_name(),"error":error})),
        }
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok((items, errors))
}
pub fn visible(app: &App, bot_id: &str) -> Result<Vec<Skill>, String> {
    let bot = app.bot(bot_id).ok_or("Unknown agent")?;
    if Some(bot.runner_id.as_str()) != app.this_device_id().as_deref() {
        return Err("Skills belong to the agent's Runner".into());
    }
    let index = registry(app)?;
    let mut scopes = vec![Scope::runner()];
    if let Some(id) = &bot.space_id {
        scopes.push(Scope {
            kind: "space".into(),
            id: id.clone(),
        });
    }
    scopes.push(Scope {
        kind: "agent".into(),
        id: bot.id.clone(),
    });
    let mut result = Vec::new();
    for scope in scopes {
        result.extend(discover_scope(app, &scope, &index, false)?.0);
    }
    if let Some(ids) = index.selections.get(&format!("agent:{bot_id}")) {
        result.retain(|s| ids.contains(&s.id));
    }
    if let Some(ids) = bot
        .space_id
        .as_ref()
        .and_then(|id| index.selections.get(&format!("space:{id}")))
    {
        result.retain(|s| s.scope.kind == "agent" || ids.contains(&s.id));
    }
    Ok(result)
}
pub fn read(skill: &Skill, relative: &str, offset: usize, limit: usize) -> Result<Value, String> {
    let path = if relative == "." {
        skill.root.clone()
    } else {
        skill.root.join(safe_relative(relative)?)
    }
    .canonicalize()
    .map_err(|e| e.to_string())?;
    if !path.starts_with(skill.root.canonicalize().map_err(|e| e.to_string())?) {
        return Err("Resource leaves skill directory".into());
    }
    if path.is_dir() {
        let mut entries = std::fs::read_dir(&path).map_err(|e|e.to_string())?.map(|entry| {
            let entry = entry.map_err(|e|e.to_string())?;
            Ok(json!({"name":entry.file_name(),"directory":entry.file_type().map_err(|e|e.to_string())?.is_dir()}))
        }).collect::<Result<Vec<Value>,String>>()?;
        entries.sort_by_key(|entry| entry["name"].as_str().unwrap_or("").to_owned());
        let total = entries.len();
        let end = offset.saturating_add(limit.clamp(1, 500)).min(total);
        return Ok(
            json!({"path":relative,"entries":entries.into_iter().skip(offset).take(end.saturating_sub(offset)).collect::<Vec<_>>(),"next_offset":(end<total).then_some(end),"total_entries":total}),
        );
    }
    if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > MAX_FILE {
        return Err("Resource exceeds read budget".into());
    }
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let limit = limit.clamp(1, 262144);
    if let Ok(text) = std::str::from_utf8(&bytes) {
        let count = text.chars().count();
        let end = offset.saturating_add(limit).min(count);
        Ok(
            json!({"path":relative,"text":text.chars().skip(offset).take(limit).collect::<String>(),"offset":offset,"next_offset":(end<count).then_some(end),"total_characters":count,"root":skill.root,"execution":"Reading never executes scripts; use ordinary file and shell tools with existing permissions and Space boundaries."}),
        )
    } else {
        use base64::Engine;
        let end = offset.saturating_add(limit).min(bytes.len());
        if offset > bytes.len() {
            return Err("Offset exceeds resource length".into());
        }
        Ok(
            json!({"path":relative,"encoding":"base64","data":base64::engine::general_purpose::STANDARD.encode(&bytes[offset..end]),"offset":offset,"next_offset":(end<bytes.len()).then_some(end),"total_bytes":bytes.len()}),
        )
    }
}
fn copy_package(from: &Path, to: &Path) -> Result<(), String> {
    let files = package_files(from)?;
    std::fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for (relative, _) in files {
        let target = to.join(&relative);
        std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::copy(from.join(relative), &target).map_err(|e| e.to_string())?;
    }
    Ok(())
}
fn scope(params: &Value) -> Result<Scope, String> {
    serde_json::from_value(
        params
            .get("scope")
            .cloned()
            .unwrap_or(json!({"kind":"runner"})),
    )
    .map_err(|e| e.to_string())
}
fn str_param<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v[key].as_str().ok_or_else(|| format!("{key} is required"))
}
fn get_skill(app: &App, params: &Value) -> Result<Skill, String> {
    let index = registry(app)?;
    let requested = scope(params)?;
    discover_scope(app, &requested, &index, true)?
        .0
        .into_iter()
        .find(|s| s.id == params["id"])
        .ok_or("Unknown skill in this Runner/scope".into())
}

pub async fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    if let Some(runner) = params["runner_id"]
        .as_str()
        .filter(|id| Some(*id) != app.this_device_id().as_deref())
    {
        return crate::requests::ask_within(
            app,
            runner,
            method,
            params.clone(),
            if matches!(method, "skills.install" | "skills.update") {
                std::time::Duration::from_secs(180)
            } else {
                crate::requests::REQUEST_TIMEOUT
            },
        )
        .await;
    }
    serve(app, method, &params).await
}
pub async fn serve(app: &Arc<App>, method: &str, params: &Value) -> Result<Value, String> {
    current_runner(app)?;
    match method {
        "skills.list" | "skills.search" => {
            let index = registry(app)?;
            let (mut items, errors) = if let Some(bot) = params["bot_id"].as_str() {
                (visible(app, bot)?, vec![])
            } else {
                discover_scope(app, &scope(params)?, &index, true)?
            };
            let q = params["query"].as_str().unwrap_or("").to_lowercase();
            items.retain(|s| {
                format!("{} {}", s.name, s.description)
                    .to_lowercase()
                    .contains(&q)
            });
            Ok(json!({"runner_id":app.this_device_id(),"items":items,"errors":errors}))
        }
        "skills.read" => {
            let skill = get_skill(app, params)?;
            read(
                &skill,
                params["path"].as_str().unwrap_or("SKILL.md"),
                params["offset"].as_u64().unwrap_or(0) as usize,
                params["limit"].as_u64().unwrap_or(65536) as usize,
            )
        }
        "skills.install" | "skills.update" => {
            sources::install(app, params, method == "skills.update").await
        }
        "skills.set_selection" => {
            let _lock = app.skills_lock.lock().await;
            let target = scope(params)?;
            scope_root(app, &target)?;
            if !matches!(target.kind.as_str(), "agent" | "space") {
                return Err("Selections belong to an agent or Space".into());
            }
            let ids: BTreeSet<String> =
                serde_json::from_value(params["ids"].clone()).map_err(|e| e.to_string())?;
            let mut index = registry(app)?;
            index.selections.insert(target.key(), ids);
            save_registry(app, &index)?;
            Ok(json!({"saved":true}))
        }
        "skills.remove" => {
            let _lock = app.skills_lock.lock().await;
            let skill = get_skill(app, params)?;
            let mut index = registry(app)?;
            if !index.installed.contains_key(&skill.id) {
                return Err("This directory is externally managed; remove it at its source".into());
            }
            if params["expected_digest"].as_str() != Some(&skill.digest) {
                return Err("Skill changed; read it again before removal".into());
            }
            let trash = skill
                .root
                .parent()
                .ok_or("Invalid skill root")?
                .join(format!(".skill-trash-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&trash).map_err(|e| e.to_string())?;
            std::fs::rename(&skill.root, trash.join(&skill.name)).map_err(|e| e.to_string())?;
            index.installed.remove(&skill.id);
            if let Err(error) = save_registry(app, &index) {
                let _ = std::fs::rename(trash.join(&skill.name), &skill.root);
                return Err(error);
            }
            Ok(json!({"removed":true,"recovery_directory":trash}))
        }
        "skills.create" => sources::create(app, params, None).await,
        "skills.import_playbook" => {
            let playbook_scope: crate::playbooks::Scope =
                serde_json::from_value(params["playbook_scope"].clone())
                    .map_err(|e| e.to_string())?;
            let exported =
                crate::playbooks::export(app, &playbook_scope, str_param(params, "playbook_id")?)?;
            let files: BTreeMap<String, String> = exported["files"]
                .as_array()
                .ok_or("Invalid playbook export")?
                .iter()
                .map(|f| Ok((str_param(f, "path")?.into(), str_param(f, "text")?.into())))
                .collect::<Result<_, String>>()?;
            sources::create(app,&json!({"scope":scope(params)?,"name":exported["content"]["name"],"files":files,"origin":"playbook-export"}), None).await
        }
        _ => Err("Unknown skills operation".into()),
    }
}
