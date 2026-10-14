use super::*;
#[cfg(not(feature = "runner"))]
use tokio::process::Command;

struct Staging(PathBuf);
impl Staging {
    fn new(parent: &Path) -> Result<Self, String> {
        let root = parent.join(format!(".skill-stage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        Ok(Self(root))
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
async fn command(program: &str, args: &[String], cwd: &Path) -> Result<String, String> {
    #[cfg(feature = "runner")]
    let mut cmd = lorca_agent::login_shell::command(program).await;
    #[cfg(not(feature = "runner"))]
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::null());
    let result = tokio::time::timeout(std::time::Duration::from_secs(120), cmd.output())
        .await
        .map_err(|_| format!("{program} timed out"))?
        .map_err(|e| format!("Unable to run {program}: {e}"))?;
    if !result.status.success() {
        return Err(format!(
            "{program} failed ({}); no skill was installed",
            result.status
        ));
    }
    String::from_utf8(result.stdout).map_err(|e| e.to_string())
}
async fn acquire(source: &Source, temp: &Path) -> Result<(PathBuf, String), String> {
    let (root, revision) = match source.kind.as_str() {
        "local" => {
            let path = PathBuf::from(&source.location);
            if !path.is_absolute() {
                return Err(
                    "Local source must be an absolute directory on the selected Runner".into(),
                );
            }
            (path, "local".into())
        }
        "github" => {
            let url = reqwest::Url::parse(&source.location).map_err(|e| e.to_string())?;
            if url.scheme() != "https"
                || url.host_str() != Some("github.com")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err("Use a public HTTPS GitHub repository URL without credentials; choose ref and subdir separately".into());
            }
            if url
                .path_segments()
                .map(|v| v.filter(|s| !s.is_empty()).count())
                != Some(2)
            {
                return Err("GitHub source must name owner/repository".into());
            }
            let root = temp.join("repository");
            command(
                "git",
                &[
                    "-c".into(),
                    "core.hooksPath=/dev/null".into(),
                    "clone".into(),
                    "--quiet".into(),
                    "--no-checkout".into(),
                    "--".into(),
                    source.location.clone(),
                    root.to_string_lossy().into_owned(),
                ],
                temp,
            )
            .await?;
            let reference = source.reference.as_deref().unwrap_or("HEAD");
            if reference.starts_with('-') || reference.contains('\0') {
                return Err("Invalid Git reference".into());
            }
            command(
                "git",
                &[
                    "-c".into(),
                    "core.hooksPath=/dev/null".into(),
                    "checkout".into(),
                    "--quiet".into(),
                    "--detach".into(),
                    reference.into(),
                ],
                &root,
            )
            .await?;
            let revision = command("git", &["rev-parse".into(), "HEAD".into()], &root)
                .await?
                .trim()
                .to_owned();
            (root, revision)
        }
        "npm" => {
            let valid =
                regex::Regex::new(r"^(?:@[a-z0-9._~-]+/)?[a-z0-9._~-]+(?:@[a-zA-Z0-9.*^~+_-]+)?$")
                    .unwrap();
            if !valid.is_match(&source.location)
                || source.location.starts_with('-')
                || source.location.starts_with('.')
            {
                return Err("Use an npm registry package name with an optional version".into());
            }
            let result = command(
                "npm",
                &[
                    "pack".into(),
                    "--ignore-scripts".into(),
                    "--json".into(),
                    "--pack-destination".into(),
                    temp.to_string_lossy().into_owned(),
                    "--".into(),
                    source.location.clone(),
                ],
                temp,
            )
            .await?;
            let info: Value = serde_json::from_str(&result).map_err(|e| e.to_string())?;
            let filename = str_param(&info[0], "filename")?;
            let archive = temp.join(safe_relative(filename)?);
            let root = temp.join("package-source");
            std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
            #[cfg(feature = "cli")]
            {
                let reader = flate2::read::GzDecoder::new(
                    std::fs::File::open(archive).map_err(|e| e.to_string())?,
                );
                let mut archive = tar::Archive::new(reader);
                let mut bytes = 0u64;
                let mut count = 0;
                for entry in archive.entries().map_err(|e| e.to_string())? {
                    let mut entry = entry.map_err(|e| e.to_string())?;
                    let kind = entry.header().entry_type();
                    if !kind.is_file() && !kind.is_dir() {
                        return Err(
                            "npm skill archives must contain regular files and directories".into(),
                        );
                    }
                    bytes = bytes.saturating_add(entry.size());
                    count += 1;
                    if bytes > MAX_PACKAGE || entry.size() > MAX_FILE || count > MAX_FILES {
                        return Err(
                            "npm package exceeds import budget; nothing was truncated".into()
                        );
                    }
                    if !entry.unpack_in(&root).map_err(|e| e.to_string())? {
                        return Err("npm archive path leaves the package".into());
                    }
                }
            }
            #[cfg(not(feature = "cli"))]
            {
                let _ = archive;
                return Err("npm installation requires the full Runner build".into());
            }
            (
                root.join("package"),
                info[0]["integrity"]
                    .as_str()
                    .or(info[0]["version"].as_str())
                    .unwrap_or("npm")
                    .to_owned(),
            )
        }
        _ => return Err("Choose local, github or npm source".into()),
    };
    let root = if let Some(subdir) = &source.subdir {
        root.join(safe_relative(subdir)?)
    } else {
        root
    };
    Ok((root, revision))
}
fn candidates(root: &Path) -> Result<Vec<PathBuf>, String> {
    let canonical = root.canonicalize().map_err(|e| e.to_string())?;
    let mut found = Vec::new();
    for entry in WalkDir::new(&canonical)
        .max_depth(32)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !matches!(e.file_name().to_str(), Some(".git" | "node_modules")))
    {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name() == "SKILL.md" && entry.file_type().is_file() {
            found.push(entry.path().parent().unwrap().to_path_buf());
        }
        if found.len() > 1000 {
            return Err("Package contains too many skill candidates".into());
        }
    }
    if found.is_empty() {
        return Err("The source has no standard SKILL.md skill directory".into());
    }
    Ok(found)
}
fn commit(
    app: &App,
    scope: &Scope,
    from: &Path,
    source: Source,
    revision: String,
    expected: Option<&str>,
) -> Result<Value, String> {
    let m = metadata(from)?;
    let name = m["name"].as_str().unwrap();
    let id = id_for(&current_runner(app)?, scope, name);
    let parent = scope_root(app, scope)?;
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let destination = parent.join(name);
    let mut index = registry(app)?;
    if destination.exists() {
        if !index.installed.contains_key(&id) {
            return Err("An externally managed skill already uses this name".into());
        }
        if expected != Some(digest(&destination)?.as_str()) {
            return Err("Skill changed; reload before updating".into());
        }
    } else if expected.is_some() {
        return Err("Skill was removed while updating".into());
    }
    let staging = Staging::new(&parent)?;
    let ready = staging.0.join(name);
    copy_package(from, &ready)?;
    let hash = digest(&ready)?;
    let previous = parent
        .join(format!(".skill-backup-{}", uuid::Uuid::new_v4()))
        .join(name);
    let existed = destination.exists();
    if existed {
        std::fs::create_dir_all(previous.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::rename(&destination, &previous).map_err(|e| e.to_string())?;
    }
    if let Err(error) = std::fs::rename(&ready, &destination) {
        if existed {
            let _ = std::fs::rename(&previous, &destination);
        }
        return Err(error.to_string());
    }
    index.installed.insert(
        id.clone(),
        Installed {
            scope: scope.clone(),
            name: name.into(),
            source,
            revision,
            digest: hash,
        },
    );
    if let Err(error) = save_registry(app, &index) {
        let _ = std::fs::rename(&destination, &ready);
        if existed {
            let _ = std::fs::rename(&previous, &destination);
        }
        return Err(error);
    }
    Ok(
        json!({"id":id,"name":name,"scope":scope,"digest":digest(&destination)?,"previous_directory":existed.then_some(previous)}),
    )
}
pub async fn install(app: &Arc<App>, params: &Value, update: bool) -> Result<Value, String> {
    let _lock = app.skills_lock.lock().await;
    let scope = scope(params)?;
    let runner = current_runner(app)?;
    scope_root(app, &scope)?;
    let old = if update {
        Some(get_skill(app, params)?)
    } else {
        None
    };
    let mut source: Source = if let Some(old) = &old {
        registry(app)?
            .installed
            .get(&old.id)
            .ok_or("Externally managed skill; update its source directory")?
            .source
            .clone()
    } else {
        serde_json::from_value(params["source"].clone()).map_err(|e| e.to_string())?
    };
    if update {
        if let Some(reference) = params["reference"].as_str() {
            if source.kind != "github" {
                return Err("A revision override applies only to GitHub sources".into());
            }
            source.reference = Some(reference.into());
        }
    }
    let temp = Staging::new(&app.config.home)?;
    let (root, revision) = acquire(&source, &temp.0).await?;
    let candidates = candidates(&root)?;
    let wanted = old
        .as_ref()
        .map(|s| s.name.as_str())
        .or(params["name"].as_str());
    let selected = if let Some(name) = wanted {
        candidates
            .iter()
            .find(|p| metadata(p).is_ok_and(|m| m["name"].as_str() == Some(name)))
            .ok_or("Requested skill is not in this source")?
    } else if candidates.len() == 1 {
        &candidates[0]
    } else {
        return Ok(
            json!({"selection_required":true,"candidates":candidates.iter().map(|p|json!({"directory":p.strip_prefix(&root).unwrap_or(p),"metadata":metadata(p).ok()})).collect::<Vec<_>>()}),
        );
    };
    let expected = if update {
        Some(str_param(params, "expected_digest")?)
    } else {
        None
    };
    if current_runner(app)? != runner {
        return Err("Runner identity changed during installation".into());
    }
    commit(app, &scope, selected, source, revision, expected)
}
pub async fn create(
    app: &Arc<App>,
    params: &Value,
    actor: Option<crate::model::Bot>,
) -> Result<Value, String> {
    let _lock = app.skills_lock.lock().await;
    let scope = scope(params)?;
    let check_actor = || -> Result<(), String> {
        #[cfg(feature = "runner")]
        if let Some(actor) = &actor {
            let current = app.bot(&actor.id).ok_or("Agent no longer exists")?;
            if current.runner_id != actor.runner_id || current.space_id != actor.space_id {
                return Err("Agent moved while creating the skill".into());
            }
            crate::permissions::check_tool(app, &current, "write").map_err(|e| e.reason)?;
            let target = scope_root(app, &scope)?;
            if let crate::spaces::Isolation::Boundary { root, .. } =
                crate::spaces::isolation_for(app, &current)
            {
                if !crate::spaces::allow_path(&root, &target) {
                    return Err("Skill creation leaves the current Space boundary".into());
                }
            }
        }
        #[cfg(not(feature = "runner"))]
        let _ = &actor;
        Ok(())
    };
    check_actor()?;
    let name = str_param(params, "name")?;
    safe_relative(name)?;
    if Path::new(name).components().count() != 1 {
        return Err("Skill name is a directory name".into());
    }
    let files: BTreeMap<String, String> =
        serde_json::from_value(params["files"].clone()).map_err(|e| e.to_string())?;
    let temp = Staging::new(&app.config.home)?;
    let root = temp.0.join(name);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    for (path, text) in files {
        let target = root.join(safe_relative(&path)?);
        std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(target, text).map_err(|e| e.to_string())?;
    }
    check_actor()?;
    commit(
        app,
        &scope,
        &root,
        Source {
            kind: "authored".into(),
            location: params["origin"].as_str().unwrap_or("conversation").into(),
            reference: None,
            subdir: None,
        },
        "authored".into(),
        None,
    )
}
