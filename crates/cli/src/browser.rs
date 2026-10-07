//! Owned browser sessions. The assigned Runner holds the profile and input gate;
//! paired Devices send the same encrypted requests used for other Runner controls.

use crate::app::App;
#[cfg(feature = "runner")]
use serde_json::json;
use serde_json::Value;
use std::sync::Arc;

#[cfg(feature = "runner")]
mod runner;
#[cfg(feature = "runner")]
pub use runner::{publish_image, review_call, SessionTool, Sessions};

pub const PLUGIN_ID: &str = "playwright";

pub async fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    let bot_id = params["bot_id"].as_str().ok_or("missing bot_id")?;
    let bot = app.bot(bot_id).ok_or("Unknown bot")?;
    if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
        // Opening the Runner's screen from elsewhere is not remote viewing or input.
        if method == "browser.open" {
            return Err("Open this session on its assigned Runner. Remote live viewing and input are unavailable.".into());
        }
        return crate::requests::ask_within(
            app,
            &bot.runner_id,
            method,
            params.clone(),
            std::time::Duration::from_secs(150),
        )
        .await;
    }
    #[cfg(feature = "runner")]
    return serve(app, method, &params, false).await;
    #[cfg(not(feature = "runner"))]
    Err("Browser sessions require an owned Runner.".into())
}

/// A sealed request is already addressed to this Runner; it never forwards or
/// opens a window on a different Device.
#[cfg(feature = "runner")]
pub async fn serve(
    app: &Arc<App>,
    method: &str,
    params: &Value,
    remote: bool,
) -> Result<Value, String> {
    let bot_id = params["bot_id"].as_str().ok_or("missing bot_id")?;
    app.browser_sessions.local_bot(app, bot_id)?;
    let capabilities = json!({
        "visible_open": !remote,
        "local_input": !remote,
        "pause": true, "resume": true, "stop": true, "screenshot": true,
        "remote_live_view": false, "remote_input": false, "native_input": false
    });
    if method == "browser.sessions" {
        return Ok(
            json!({ "sessions": app.browser_sessions.list(app, bot_id)?, "capabilities": capabilities }),
        );
    }
    let session = if method == "browser.create" {
        app.browser_sessions.create(
            app,
            bot_id,
            params["account"].as_str().unwrap_or("Default"),
            params["profile"].as_str().unwrap_or("Browser"),
        )?
    } else {
        let id = params["session_id"].as_str().ok_or("missing session_id")?;
        match method {
            "browser.open" if remote => {
                return Err("Visible open requires the local Runner.".into())
            }
            "browser.open" => app.browser_sessions.open(app, bot_id, id, true).await?,
            "browser.takeover" => app.browser_sessions.takeover(app, bot_id, id).await?,
            "browser.resume" => {
                let revision = params["revision"].as_u64().ok_or("missing revision")?;
                app.browser_sessions
                    .resume(app, bot_id, id, revision)
                    .await?
            }
            "browser.stop" => app.browser_sessions.stop(app, bot_id, id).await?,
            "browser.screenshot" => {
                let chat_id = params["chat_id"].as_str().ok_or("missing chat_id")?;
                let summary = params["summary"]
                    .as_str()
                    .unwrap_or("Browser verification screenshot");
                let message = app
                    .browser_sessions
                    .screenshot(app, bot_id, id, chat_id, summary)
                    .await?;
                return Ok(
                    json!({ "message_id": message.id, "session_id": id, "capabilities": capabilities }),
                );
            }
            _ => return Err(format!("Unknown browser method {method}")),
        }
    };
    Ok(json!({ "session": session, "capabilities": capabilities }))
}
