//! Spaces in the roster, and the Runner's file-boundary hook.
//!
//! A Space names a set of bots on one Runner. Its sandbox is an optional file
//! boundary. Approval `never` is stored on the bot and does not require that
//! boundary, whether the bot is in a Space or solo. Skills stay playbooks on a
//! bot or a group; a Space does not hold them.
//!
//! The relay sees Spaces only inside the encrypted roster. A build that does not
//! know the field leaves it out, and [`take_roster`] keeps the Spaces this Device
//! already has. File tools do not read [`isolation`] yet: it is the Runner hook
//! the boundary is enforced through.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::app::App;
use crate::config::now_secs;
use crate::model::{Approval, Bot, Sandbox, Space};

/// What [`isolation`] reports for a bot. Open is a solo bot, or a Space member
/// whose Space has no sandbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Isolation {
    Open,
    /// The Space's file boundary on its Runner.
    Boundary {
        space_id: String,
        root: PathBuf,
    },
}

/// Whether storing `approval` requires a sandbox. `never` does not: a solo bot
/// passes `sandbox == None`, and so does a Space bot whose sandbox is off.
pub fn approval_requires_sandbox(approval: Approval, sandbox: Option<&Sandbox>) -> bool {
    let _ = (approval, sandbox);
    false
}

/// The Runner hook for a bot's file boundary. Open when the bot is solo or its
/// Space has no sandbox. The default root is `<lorca_home>/spaces/<id>`.
pub fn isolation(lorca_home: &Path, spaces: &[Space], bot: &Bot) -> Isolation {
    let Some(space_id) = bot.space_id.as_deref() else {
        return Isolation::Open;
    };
    let Some(space) = spaces.iter().find(|space| space.id == space_id) else {
        return Isolation::Open;
    };
    let Some(sandbox) = &space.sandbox else {
        return Isolation::Open;
    };
    let root = match sandbox.root.as_deref() {
        Some(root) => expand_path(root),
        None => lorca_home.join("spaces").join(&space.id),
    };
    Isolation::Boundary {
        space_id: space.id.clone(),
        root,
    }
}

/// [`isolation`] for the Spaces this Device holds.
pub fn isolation_for(app: &App, bot: &Bot) -> Isolation {
    let state = app.state.lock().unwrap();
    isolation(&app.config.home, &state.spaces, bot)
}

/// A bot may join a Space only on that Space's Runner. Approval is checked
/// against the Space's sandbox, which `never` does not require.
pub fn assignment_ok(spaces: &[Space], bot: &Bot) -> Result<(), String> {
    let sandbox = match bot.space_id.as_deref() {
        None => None,
        Some(space_id) => {
            let space = spaces
                .iter()
                .find(|space| space.id == space_id)
                .ok_or_else(|| format!("Unknown space {space_id}"))?;
            if space.runner_id != bot.runner_id {
                return Err("A bot joins a Space on that Space's Runner.".into());
            }
            space.sandbox.as_ref()
        }
    };
    if approval_requires_sandbox(bot.approval_mode(), sandbox) {
        return Err("This approval needs a sandbox.".into());
    }
    Ok(())
}

/// Applies an incoming roster's Spaces.
///
/// `incoming == None` means the sender left the field out. This Device keeps its
/// Spaces and puts `space_id` and `approval` back on bots the sender dropped.
/// `Some` is the sender's whole list, including an empty one, and a `space_id`
/// that names nothing in it is cleared. Returns whether the roster should be
/// uploaded again so a sender that omitted Spaces does not drop them.
pub fn take_roster(
    spaces: &mut Vec<Space>,
    current_bots: &[Bot],
    incoming: Option<Vec<Space>>,
    incoming_bots: &mut [Bot],
) -> bool {
    let Some(incoming) = incoming else {
        let mut republish = !spaces.is_empty();
        for bot in incoming_bots.iter_mut() {
            let Some(old) = current_bots.iter().find(|old| old.id == bot.id) else {
                continue;
            };
            if bot.space_id.is_none() && old.space_id.is_some() {
                bot.space_id = old.space_id.clone();
                republish = true;
            }
            if bot.approval.is_none() && old.approval.is_some() {
                bot.approval = old.approval;
                republish = true;
            }
        }
        return republish;
    };
    *spaces = incoming;
    let mut republish = false;
    for bot in incoming_bots.iter_mut() {
        if bot
            .space_id
            .as_deref()
            .is_some_and(|id| !spaces.iter().any(|space| space.id == id))
        {
            bot.space_id = None;
            republish = true;
        }
    }
    republish
}

pub fn dispatch(app: &Arc<App>, method: &str, params: &Value) -> Result<Value, String> {
    match method {
        "spaces.list" => Ok(json!({ "spaces": app.state.lock().unwrap().spaces.clone() })),
        "spaces.create" => {
            let space = create(app, params)?;
            Ok(json!({ "space": space }))
        }
        "spaces.update" => {
            let space = update(app, params)?;
            Ok(json!({ "space": space }))
        }
        "spaces.delete" => {
            delete(app, &string(params, "id")?)?;
            Ok(Value::Null)
        }
        _ => Err(format!("Unknown method {method}")),
    }
}

fn create(app: &App, params: &Value) -> Result<Space, String> {
    let runner_id = string(params, "runner_id")?;
    require_runner(app, &runner_id)?;
    let sandbox = match params.get("sandbox") {
        None | Some(Value::Null) => None,
        Some(value) => parse_sandbox(value)?,
    };
    let space = Space {
        id: match params.get("id") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(id)) => clean_id(id)?,
            Some(_) => return Err("id is a space id".into()),
        },
        name: space_name(&string(params, "name")?)?,
        runner_id,
        sandbox,
        created_at: 0.0,
    };
    insert(app, space)
}

fn update(app: &App, params: &Value) -> Result<Space, String> {
    let id = string(params, "id")?;
    if let Some(runner_id) = params.get("runner_id").and_then(|value| value.as_str()) {
        let current = app
            .state
            .lock()
            .unwrap()
            .spaces
            .iter()
            .find(|space| space.id == id)
            .cloned()
            .ok_or_else(|| format!("Unknown space {id}"))?;
        if runner_id != current.runner_id {
            return Err("A Space stays on the Runner it was created for.".into());
        }
    }
    let name = match params.get("name").and_then(|value| value.as_str()) {
        Some(name) => Some(space_name(name)?),
        None => None,
    };
    let sandbox = match params.get("sandbox") {
        None => None,
        Some(value) => Some(parse_sandbox(value)?),
    };
    let space = {
        let mut state = app.state.lock().unwrap();
        let space = state
            .spaces
            .iter_mut()
            .find(|space| space.id == id)
            .ok_or_else(|| format!("Unknown space {id}"))?;
        if let Some(name) = name {
            space.name = name;
        }
        if let Some(sandbox) = sandbox {
            space.sandbox = sandbox;
        }
        space.clone()
    };
    app.roster_changed(true);
    Ok(space)
}

fn delete(app: &App, id: &str) -> Result<(), String> {
    {
        let mut state = app.state.lock().unwrap();
        if !state.spaces.iter().any(|space| space.id == id) {
            return Err(format!("Unknown space {id}"));
        }
        state.spaces.retain(|space| space.id != id);
        for bot in &mut state.bots {
            if bot.space_id.as_deref() == Some(id) {
                bot.space_id = None;
            }
        }
    }
    app.roster_changed(true);
    Ok(())
}

fn insert(app: &App, mut space: Space) -> Result<Space, String> {
    if space.created_at == 0.0 {
        space.created_at = now_secs();
    }
    let mut state = app.state.lock().unwrap();
    if state.spaces.len() >= 100 {
        return Err("An account holds at most 100 Spaces.".into());
    }
    if space.id.is_empty() {
        space.id = format!("space-{}", &uuid::Uuid::new_v4().to_string()[..8]);
    }
    if state.spaces.iter().any(|existing| existing.id == space.id) {
        return Err("Space id already exists".into());
    }
    state.spaces.push(space.clone());
    drop(state);
    app.roster_changed(true);
    Ok(space)
}

fn require_runner(app: &App, runner_id: &str) -> Result<(), String> {
    let device = app
        .device(runner_id)
        .ok_or_else(|| format!("Unknown Runner {runner_id}"))?;
    if !device.is_runner() {
        return Err(format!(
            "{} runs {} and cannot hold a Space",
            device.name, device.os
        ));
    }
    Ok(())
}

fn string(params: &Value, key: &str) -> Result<String, String> {
    params[key]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("missing {key}"))
}

fn space_name(name: &str) -> Result<String, String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || name.chars().count() > 60 {
        return Err("A Space name is one line of at most 60 characters.".into());
    }
    Ok(name)
}

fn clean_id(id: &str) -> Result<String, String> {
    let id = id.trim();
    let plain = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    if !plain {
        return Err("A Space id is at most 64 letters, digits, or . _ -".into());
    }
    Ok(id.to_string())
}

fn parse_sandbox(value: &Value) -> Result<Option<Sandbox>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Object(map) => {
            let root = match map.get("root") {
                None | Some(Value::Null) => None,
                Some(Value::String(root)) => clean_root(root)?,
                Some(_) => return Err("sandbox.root is a path".into()),
            };
            Ok(Some(Sandbox { root }))
        }
        _ => Err("sandbox is an object or null".into()),
    }
}

fn clean_root(root: &str) -> Result<Option<String>, String> {
    let root = root.trim();
    if root.is_empty() {
        return Ok(None);
    }
    if root.contains('\0') || root.len() > 1024 {
        return Err("A sandbox root is a path under 1024 characters.".into());
    }
    let windows = root.len() >= 3
        && root.as_bytes()[1] == b':'
        && root.as_bytes()[0].is_ascii_alphabetic()
        && matches!(root.as_bytes()[2], b'\\' | b'/');
    if !(root.starts_with('/') || root.starts_with('~') || windows) {
        return Err("A sandbox root is an absolute path.".into());
    }
    Ok(Some(root.to_string()))
}

fn expand_path(path: &str) -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    match path.strip_prefix('~') {
        Some(rest) => home.join(rest.trim_start_matches(['/', '\\'])),
        None => PathBuf::from(path),
    }
}

/// `space_id` and `approval` from a `bots.create` or `bots.update` params object.
/// `None` leaves the field alone. `Some(None)` clears it.
pub fn profile_patch(
    params: &Value,
) -> Result<(Option<Option<String>>, Option<Option<Approval>>), String> {
    let space_id = match params.get("space_id") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(id)) if id.trim().is_empty() => Some(None),
        Some(Value::String(id)) => Some(Some(id.trim().to_string())),
        Some(_) => return Err("space_id is a space id or null".into()),
    };
    let approval = match params.get("approval") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(value) => Some(Some(Approval::parse(value)?)),
    };
    Ok((space_id, approval))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Device, RosterBlob};
    use serde_json::json;
    use std::sync::Arc;

    struct Scratch(Arc<App>, std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn scratch() -> (Scratch, String) {
        let home = std::env::temp_dir().join(format!("lorca-spaces-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config {
            home: home.clone(),
            port: 0,
        })
        .unwrap();
        crate::identity::create(&app, Some("Space Runner".into())).unwrap();
        let runner = app.this_device_id().unwrap();
        (Scratch(app, home), runner)
    }

    fn bot(id: &str, runner: &str) -> Bot {
        app_bot_placeholder(id, runner)
    }

    fn app_bot_placeholder(id: &str, runner: &str) -> Bot {
        Bot {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            symbol_name: "sparkles".into(),
            accent: "indigo".into(),
            avatar: None,
            runner_id: runner.into(),
            provider: "deepseek".into(),
            model: None,
            thinking: None,
            legacy_instructions: String::new(),
            workdir: None,
            permissions: None,
            space_id: None,
            approval: None,
            created_at: 1.0,
        }
    }

    #[test]
    fn never_needs_no_sandbox_for_a_solo_bot_or_a_space_bot() {
        assert!(!approval_requires_sandbox(Approval::Never, None));
        assert!(!approval_requires_sandbox(
            Approval::Never,
            Some(&Sandbox { root: None })
        ));
        assert!(!approval_requires_sandbox(Approval::Ask, None));
        let home = Path::new("/tmp/lorca");
        let space = Space {
            id: "space-lab".into(),
            name: "Lab".into(),
            runner_id: "runner".into(),
            sandbox: None,
            created_at: 1.0,
        };
        let mut member = bot("bot", "runner");
        member.space_id = Some(space.id.clone());
        member.approval = Some(Approval::Never);
        assert_eq!(isolation(home, &[space.clone()], &member), Isolation::Open);
        assert!(assignment_ok(&[space.clone()], &member).is_ok());
        let mut solo = bot("solo", "runner");
        solo.approval = Some(Approval::Never);
        assert_eq!(isolation(home, &[space], &solo), Isolation::Open);
        assert!(assignment_ok(&[], &solo).is_ok());
    }

    #[test]
    fn isolation_reports_the_sandbox_root_and_open_without_one() {
        let home = Path::new("/tmp/lorca");
        let mut space = Space {
            id: "space-lab".into(),
            name: "Lab".into(),
            runner_id: "runner".into(),
            sandbox: Some(Sandbox { root: None }),
            created_at: 1.0,
        };
        let mut member = bot("bot", "runner");
        member.space_id = Some(space.id.clone());
        assert_eq!(
            isolation(home, &[space.clone()], &member),
            Isolation::Boundary {
                space_id: space.id.clone(),
                root: home.join("spaces").join(&space.id)
            }
        );
        space.sandbox = Some(Sandbox {
            root: Some("/var/lab".into()),
        });
        assert_eq!(
            isolation(home, &[space.clone()], &member),
            Isolation::Boundary {
                space_id: space.id.clone(),
                root: PathBuf::from("/var/lab")
            }
        );
        space.sandbox = None;
        assert_eq!(isolation(home, &[space], &member), Isolation::Open);
    }

    #[test]
    fn an_upstream_roster_keeps_spaces_and_a_dots_roster_can_clear_membership() {
        let space = Space {
            id: "space-lab".into(),
            name: "Lab".into(),
            runner_id: "runner".into(),
            sandbox: None,
            created_at: 1.0,
        };
        let mut current = bot("bot", "runner");
        current.space_id = Some(space.id.clone());
        current.approval = Some(Approval::Never);
        let mut spaces = vec![space.clone()];
        let mut echoed = bot("bot", "runner");
        assert!(take_roster(
            &mut spaces,
            &[current.clone()],
            None,
            std::slice::from_mut(&mut echoed)
        ));
        assert_eq!(spaces, vec![space.clone()]);
        assert_eq!(echoed.space_id.as_deref(), Some(space.id.as_str()));
        assert_eq!(echoed.approval, Some(Approval::Never));

        let mut cleared = bot("bot", "runner");
        assert!(!take_roster(
            &mut spaces,
            &[current],
            Some(vec![space.clone()]),
            std::slice::from_mut(&mut cleared)
        ));
        assert!(cleared.space_id.is_none());
        assert_eq!(spaces, vec![space]);

        let mut dangling = bot("bot", "runner");
        dangling.space_id = Some("space-gone".into());
        let mut dropped = Vec::new();
        assert!(take_roster(
            &mut dropped,
            &[],
            Some(vec![]),
            std::slice::from_mut(&mut dangling)
        ));
        assert!(dangling.space_id.is_none());

        let omitted = RosterBlob::default();
        assert!(serde_json::to_value(&omitted)
            .unwrap()
            .get("spaces")
            .is_none());
        let sent = RosterBlob {
            spaces: Some(vec![]),
            ..RosterBlob::default()
        };
        assert_eq!(serde_json::to_value(&sent).unwrap()["spaces"], json!([]));
    }

    #[tokio::test]
    async fn spaces_round_trip_and_never_without_a_sandbox() {
        let (scratch, runner) = scratch();
        let app = &scratch.0;
        let phone = Device {
            id: "phone".into(),
            name: "Phone".into(),
            os: "ios".into(),
            ..Device::default()
        };
        app.state.lock().unwrap().devices.push(phone);
        assert!(crate::api::dispatch(
            app,
            "spaces.create",
            json!({"name": "Lab", "runner_id": "phone"})
        )
        .await
        .unwrap_err()
        .contains("cannot hold a Space"));

        let created = crate::api::dispatch(
            app,
            "spaces.create",
            json!({"id": "space-lab", "name": "Lab", "runner_id": runner}),
        )
        .await
        .unwrap();
        assert_eq!(created["space"]["id"], "space-lab");
        assert!(created["space"].get("sandbox").is_none());
        let chef = app.state.lock().unwrap().bots[0].id.clone();
        assert!(crate::api::dispatch(
            app,
            "bots.update",
            json!({"id": chef, "space_id": "missing"})
        )
        .await
        .is_err());
        assert!(
            app.bot(&chef).unwrap().space_id.is_none(),
            "a refused join leaves the bot solo"
        );

        let other = Device {
            id: "other".into(),
            name: "Other".into(),
            os: "linux".into(),
            ..Device::default()
        };
        app.state.lock().unwrap().devices.push(other);
        crate::api::dispatch(
            app,
            "spaces.create",
            json!({"id": "space-other", "name": "Other", "runner_id": "other"}),
        )
        .await
        .unwrap();
        assert!(crate::api::dispatch(
            app,
            "bots.update",
            json!({"id": chef, "space_id": "space-other"})
        )
        .await
        .unwrap_err()
        .contains("that Space's Runner"));
        assert!(app.bot(&chef).unwrap().space_id.is_none());

        crate::api::dispatch(app, "bots.update", json!({"id": chef, "approval": "never"}))
            .await
            .unwrap();
        assert_eq!(app.bot(&chef).unwrap().approval, Some(Approval::Never));
        assert!(app.bot(&chef).unwrap().space_id.is_none());

        crate::api::dispatch(
            app,
            "bots.update",
            json!({"id": chef, "space_id": "space-lab", "approval": "never"}),
        )
        .await
        .unwrap();
        let joined = app.bot(&chef).unwrap();
        assert_eq!(joined.space_id.as_deref(), Some("space-lab"));
        assert_eq!(isolation_for(app, &joined), Isolation::Open);

        crate::api::dispatch(
            app,
            "spaces.update",
            json!({"id": "space-lab", "sandbox": {}}),
        )
        .await
        .unwrap();
        match isolation_for(app, &app.bot(&chef).unwrap()) {
            Isolation::Boundary { space_id, root } => {
                assert_eq!(space_id, "space-lab");
                assert_eq!(root, scratch.1.join("spaces").join("space-lab"));
            }
            Isolation::Open => panic!("a sandbox reports a boundary"),
        }
        crate::api::dispatch(
            app,
            "spaces.update",
            json!({"id": "space-lab", "sandbox": {"root": "/var/lab"}}),
        )
        .await
        .unwrap();
        assert!(
            matches!(isolation_for(app, &app.bot(&chef).unwrap()), Isolation::Boundary { root, .. } if root == PathBuf::from("/var/lab"))
        );
        assert!(crate::api::dispatch(
            app,
            "spaces.update",
            json!({"id": "space-lab", "sandbox": {"root": "relative"}})
        )
        .await
        .is_err());
        assert!(crate::api::dispatch(
            app,
            "spaces.update",
            json!({"id": "space-lab", "runner_id": "other"})
        )
        .await
        .is_err());
        crate::api::dispatch(
            app,
            "spaces.update",
            json!({"id": "space-lab", "sandbox": null}),
        )
        .await
        .unwrap();
        assert_eq!(app.bot(&chef).unwrap().approval, Some(Approval::Never));
        assert_eq!(
            isolation_for(app, &app.bot(&chef).unwrap()),
            Isolation::Open
        );

        let listed = crate::api::dispatch(app, "spaces.list", json!({}))
            .await
            .unwrap();
        assert_eq!(listed["spaces"].as_array().unwrap().len(), 2);
        let uploaded: RosterBlob =
            crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &roster_ciphertext(app))
                .unwrap();
        assert!(uploaded.spaces.as_ref().is_some_and(|spaces| spaces
            .iter()
            .any(|space| space.id == "space-lab" && space.sandbox.is_none())));
        assert!(uploaded.bots.iter().any(|bot| bot.id == chef
            && bot.space_id.as_deref() == Some("space-lab")
            && bot.approval == Some(Approval::Never)));

        let reloaded = App::load(crate::config::Config {
            home: scratch.1.clone(),
            port: 0,
        })
        .unwrap();
        assert_eq!(reloaded.state.lock().unwrap().spaces.len(), 2);
        assert_eq!(
            reloaded.bot(&chef).unwrap().space_id.as_deref(),
            Some("space-lab")
        );

        crate::api::dispatch(app, "spaces.delete", json!({"id": "space-lab"}))
            .await
            .unwrap();
        assert!(app.bot(&chef).unwrap().space_id.is_none());
        assert_eq!(app.bot(&chef).unwrap().approval, Some(Approval::Never));
        assert!(app
            .state
            .lock()
            .unwrap()
            .spaces
            .iter()
            .all(|space| space.id != "space-lab"));
    }

    fn roster_ciphertext(app: &App) -> Vec<u8> {
        app.store
            .outbox()
            .unwrap()
            .into_iter()
            .rev()
            .find(|blob| blob.kind == "roster")
            .unwrap()
            .ciphertext
    }
}
