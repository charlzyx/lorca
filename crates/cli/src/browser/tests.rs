use super::*;

struct Scratch(Arc<App>, std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        self.0.browser_sessions.reset();
        let _ = std::fs::remove_dir_all(&self.1);
    }
}

fn setup() -> Scratch {
    let home = std::env::temp_dir().join(format!("lorca-browser-{}", uuid::Uuid::new_v4()));
    let app = App::load(crate::config::Config {
        home: home.clone(),
        port: 0,
    })
    .unwrap();
    crate::identity::create(&app, Some("Browser test Runner".into())).unwrap();
    let manifest = crate::plugins::Manifest::parse(&json!({ "id": super::super::PLUGIN_ID, "name": "Browser", "servers": { "browser": { "type": "stdio", "command": "false", "args": ["--headless"] } } })).unwrap();
    crate::plugins::install(&app, manifest, "inline").unwrap();
    Scratch(app, home)
}

fn bot(app: &App) -> Bot {
    app.state.lock().unwrap().bots[0].clone()
}

async fn opened(app: &Arc<App>, owner: &Bot) -> (Session, Arc<Mutex<Vec<String>>>) {
    let session = app
        .browser_sessions
        .create(app, &owner.id, "Work account", "Separate profile")
        .unwrap();
    let runtime = app
        .browser_sessions
        .owned(app, &owner.id, &session.id)
        .unwrap();
    let (server, calls) = crate::plugins::mcp::tests::fake_browser(app).await;
    *runtime.server.lock().unwrap() = Some(server);
    runtime.meta.lock().unwrap().state = Control::Bot;
    app.browser_sessions.save(app).unwrap();
    (session, calls)
}

#[tokio::test]
async fn ownership_and_encrypted_persistence_survive_restart_without_auto_open() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, _) = opened(app, &owner).await;
    let bytes = std::fs::read(scratch.1.join("browser/sessions.enc")).unwrap();
    assert!(!bytes
        .windows(owner.id.len())
        .any(|part| part == owner.id.as_bytes()));
    assert!(!bytes.windows(12).any(|part| part == b"Work account"));
    let mut other = owner.clone();
    other.id = "another-bot".into();
    app.state.lock().unwrap().bots.push(other.clone());
    assert!(app
        .browser_sessions
        .owned(app, &other.id, &session.id)
        .err()
        .unwrap()
        .contains("another bot"));
    app.browser_sessions.reset();
    let restored = app.browser_sessions.list(app, &owner.id).unwrap();
    assert_eq!(restored[0].id, session.id);
    assert_eq!(restored[0].account, "Work account");
    assert_eq!(restored[0].state, Control::Stopped);
    assert!(app.browser_sessions.bot_ready(app, &owner.id).is_err());
    app.state.lock().unwrap().bots[0].runner_id = "another-runner".into();
    assert!(app
        .browser_sessions
        .list(app, &owner.id)
        .unwrap_err()
        .contains("assigned"));
}

#[tokio::test]
async fn takeover_drains_active_input_and_parks_next_call_until_explicit_resume() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, calls) = opened(app, &owner).await;
    let cancel = CancellationToken::new();
    let active = app
        .browser_sessions
        .input(app, &owner.id, &cancel)
        .await
        .unwrap();
    let takeover = {
        let (app, bot_id, id) = (app.clone(), owner.id.clone(), session.id.clone());
        tokio::spawn(async move { app.browser_sessions.takeover(&app, &bot_id, &id).await })
    };
    // Observe the revocation before releasing the active call's guard.
    for _ in 0..100 {
        if app.browser_sessions.list(app, &owner.id).unwrap()[0].state == Control::TakingOver {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        app.browser_sessions.list(app, &owner.id).unwrap()[0].state,
        Control::TakingOver
    );
    assert!(!takeover.is_finished());
    let next = {
        let (app, bot_id) = (app.clone(), owner.id.clone());
        tokio::spawn(async move {
            let input = app
                .browser_sessions
                .input(&app, &bot_id, &CancellationToken::new())
                .await?;
            input
                .server
                .browser_call("browser_snapshot", json!({}))
                .await
                .map(|_| ())
        })
    };
    active
        .server
        .browser_call("browser_snapshot", json!({}))
        .await
        .unwrap();
    drop(active);
    let human = takeover.await.unwrap().unwrap();
    assert_eq!(human.state, Control::Human);
    assert_eq!(
        calls.lock().unwrap().len(),
        1,
        "only the active call executed"
    );
    assert!(
        !next.is_finished(),
        "the next call keeps its task state parked"
    );
    assert!(app
        .browser_sessions
        .resume(app, &owner.id, &session.id, human.revision - 1)
        .await
        .unwrap_err()
        .contains("changed"));
    app.browser_sessions
        .resume(app, &owner.id, &session.id, human.revision)
        .await
        .unwrap();
    next.await.unwrap().unwrap();
    assert_eq!(calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn stop_during_takeover_wakes_waiters_and_closes_only_that_session() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, _) = opened(app, &owner).await;
    let mut other = owner.clone();
    other.id = "other-browser-bot".into();
    app.state.lock().unwrap().bots.push(other.clone());
    let (other_session, _) = opened(app, &other).await;
    let other_runtime = app
        .browser_sessions
        .owned(app, &other.id, &other_session.id)
        .unwrap();
    app.browser_sessions
        .takeover(app, &owner.id, &session.id)
        .await
        .unwrap();
    let pending = {
        let (app, bot_id) = (app.clone(), owner.id.clone());
        tokio::spawn(async move {
            app.browser_sessions
                .input(&app, &bot_id, &CancellationToken::new())
                .await
                .map(|_| ())
        })
    };
    tokio::task::yield_now().await;
    assert!(!pending.is_finished());
    let stopped = app
        .browser_sessions
        .stop(app, &owner.id, &session.id)
        .await
        .unwrap();
    assert_eq!(stopped.state, Control::Stopped);
    assert!(pending.await.unwrap().unwrap_err().contains("stopped"));
    assert!(app
        .browser_sessions
        .resume(app, &owner.id, &session.id, stopped.revision)
        .await
        .is_err());
    assert!(!other_runtime
        .server
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .is_closed());
    assert!(app
        .browser_sessions
        .input(app, &other.id, &CancellationToken::new())
        .await
        .is_ok());
}

#[tokio::test]
async fn screenshot_evidence_uses_encrypted_file_and_chat_blobs() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, _) = opened(app, &owner).await;
    let chat_id = app.state.lock().unwrap().chats[0].meta.id.clone();
    let message = app
        .browser_sessions
        .screenshot(app, &owner.id, &session.id, &chat_id, "After sign-in")
        .await
        .unwrap();
    let Body::Text {
        attachments, text, ..
    } = &message.body
    else {
        panic!("evidence is visible on existing clients")
    };
    assert!(text.contains("outcome unverified"));
    assert_eq!(attachments.len(), 1);
    let pending = app.store.outbox().unwrap();
    let file = pending
        .iter()
        .find(|item| item.id == attachments[0].id)
        .unwrap();
    assert_eq!(file.kind, "file");
    assert_eq!(file.group.as_deref(), Some(chat_id.as_str()));
    let decrypted = crate::crypto::decrypt(&app.dek().unwrap(), "file", &file.ciphertext).unwrap();
    assert_eq!(&decrypted[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(app.message(&chat_id, &message.id).unwrap().id, message.id);
    assert!(app
        .browser_sessions
        .screenshot(app, &owner.id, &session.id, "wrong-chat", "Before")
        .await
        .is_err());
    let remote = crate::browser::serve(
        app,
        "browser.sessions",
        &json!({ "bot_id": owner.id }),
        true,
    )
    .await
    .unwrap();
    assert_eq!(remote["capabilities"]["remote_input"], false);
    assert_eq!(remote["capabilities"]["visible_open"], false);
    assert!(crate::browser::serve(
        app,
        "browser.open",
        &json!({ "bot_id": owner.id, "session_id": session.id }),
        true
    )
    .await
    .is_err());
}

#[tokio::test]
async fn cancellation_releases_a_parked_turn_and_bot_cannot_override_human_control() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, _) = opened(app, &owner).await;
    app.browser_sessions
        .takeover(app, &owner.id, &session.id)
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert_eq!(
        app.browser_sessions
            .wait_if_taken_over(app, &owner.id, &cancel)
            .await
            .unwrap_err(),
        "Stopped"
    );
    assert!(app
        .browser_sessions
        .open(app, &owner.id, &session.id, false)
        .await
        .unwrap_err()
        .contains("Only the user"));
    assert_eq!(
        app.browser_sessions.list(app, &owner.id).unwrap()[0].state,
        Control::Human
    );
}

#[tokio::test]
async fn forgetting_identity_revokes_processes_and_removes_browser_state() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, _) = opened(app, &owner).await;
    let runtime = app
        .browser_sessions
        .owned(app, &owner.id, &session.id)
        .unwrap();
    let server = runtime.server.lock().unwrap().clone().unwrap();
    app.forget_identity().unwrap();
    assert!(server.is_closed());
    assert!(!scratch.1.join("browser").exists());
    assert!(app.browser_sessions.list(app, &owner.id).is_err());
}

#[tokio::test]
async fn stop_invalidates_an_open_that_queued_behind_active_input() {
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let (session, _) = opened(app, &owner).await;
    let active = app
        .browser_sessions
        .input(app, &owner.id, &CancellationToken::new())
        .await
        .unwrap();
    let pending_open = {
        let (app, bot_id, id) = (app.clone(), owner.id.clone(), session.id.clone());
        tokio::spawn(async move { app.browser_sessions.open(&app, &bot_id, &id, true).await })
    };
    tokio::task::yield_now().await;
    let stop = {
        let (app, bot_id, id) = (app.clone(), owner.id.clone(), session.id.clone());
        tokio::spawn(async move { app.browser_sessions.stop(&app, &bot_id, &id).await })
    };
    tokio::task::yield_now().await;
    drop(active);
    assert!(pending_open
        .await
        .unwrap()
        .unwrap_err()
        .contains("control changed"));
    stop.await.unwrap().unwrap();
    let runtime = app
        .browser_sessions
        .owned(app, &owner.id, &session.id)
        .unwrap();
    assert!(runtime.server.lock().unwrap().is_none());
    assert_eq!(runtime.meta.lock().unwrap().state, Control::Stopped);
}

/// Manual integration verification: opens a real headed browser on the host
/// using a fresh profile and a loopback-only demo sign-in page.
#[tokio::test]
#[ignore = "requires Node/npx, Chrome, and an interactive desktop; opens a fresh visible browser"]
async fn live_visible_browser_retains_sign_in_and_captures_evidence() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let scratch = setup();
    let app = &scratch.0;
    let owner = bot(app);
    let manifest = crate::plugins::Manifest::parse(&json!({ "id": super::super::PLUGIN_ID, "name": "Browser", "servers": { "browser": { "type": "stdio", "command": "npx", "args": ["-y", "@playwright/mcp@latest", "--headless"] } } })).unwrap();
    crate::plugins::install(app, manifest, "inline").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let fixture = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await;
            let page = r#"<!doctype html><html><head><title>Lorca browser session verification</title><style>body{font:18px system-ui;background:#f6f8fa;color:#1f2937;padding:72px}main{background:white;border:1px solid #dde3e9;border-radius:18px;padding:40px;max-width:620px}h1{margin-top:0}input,button{font:inherit;padding:10px;margin:10px 0;border:1px solid #ccd2db;border-radius:8px}button{background:#3665e8;color:white}small{color:#64748b}#result{color:#17834e;font-weight:600}</style></head><body><main><small>ISSUE #85 · OWNED RUNNER · ISOLATED DEMO PROFILE</small><h1>Visible browser session</h1><p>A user signs in while the bot waits for explicit return of control.</p><form><label>Demo account<br><input name=email value=demo@example.test></label><br><button type=submit>Sign in to demo account</button></form><p id=result></p><small>This page runs on localhost and uses no real credentials.</small></main><script>const update=()=>{const signed=document.cookie.includes('lorca-demo=signed-in');document.querySelector('#result').textContent=signed?'Signed in · profile state preserved':'Awaiting human sign-in';document.querySelector('form').style.display=signed?'none':'block'};document.querySelector('form').onsubmit=e=>{e.preventDefault();document.cookie='lorca-demo=signed-in; Max-Age=86400; Path=/';update()};update();</script></body></html>"#;
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", page.len(), page);
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    let session = app
        .browser_sessions
        .create(app, &owner.id, "Demo account", "Issue 85 verification")
        .unwrap();
    let human = app
        .browser_sessions
        .open(app, &owner.id, &session.id, true)
        .await
        .unwrap();
    let runtime = app
        .browser_sessions
        .owned(app, &owner.id, &session.id)
        .unwrap();
    let server = runtime.server.lock().unwrap().clone().unwrap();
    server
        .browser_call("browser_navigate", json!({ "url": url }))
        .await
        .unwrap();
    let chat_id = app.state.lock().unwrap().chats[0].meta.id.clone();
    let evidence_dir =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/issue85-evidence");
    std::fs::create_dir_all(&evidence_dir).unwrap();
    let before = app
        .browser_sessions
        .screenshot(app, &owner.id, &session.id, &chat_id, "Before demo sign-in")
        .await
        .unwrap();
    copy_evidence(app, &before, &evidence_dir.join("browser-before.png"));
    // This call represents the local human's fixture interaction, rather
    // than a bot input call, which remains parked under human control.
    server.browser_call("browser_click", json!({ "element": "Demo sign-in button", "target": "button[type=submit]" })).await.unwrap();
    app.browser_sessions
        .resume(app, &owner.id, &session.id, human.revision)
        .await
        .unwrap();
    let input = app
        .browser_sessions
        .input(app, &owner.id, &CancellationToken::new())
        .await
        .unwrap();
    let snapshot = input
        .server
        .browser_call("browser_snapshot", json!({}))
        .await
        .unwrap();
    assert!(serde_json::to_string(&snapshot)
        .unwrap()
        .contains("profile state preserved"));
    drop(input);
    app.browser_sessions
        .takeover(app, &owner.id, &session.id)
        .await
        .unwrap();
    app.browser_sessions
        .stop(app, &owner.id, &session.id)
        .await
        .unwrap();
    assert!(!scratch
        .1
        .join("browser/profiles")
        .join(&session.id)
        .exists());
    assert!(scratch
        .1
        .join("browser/profiles")
        .join(&session.id)
        .with_extension("enc")
        .is_file());
    app.browser_sessions
        .open(app, &owner.id, &session.id, true)
        .await
        .unwrap();
    let server = runtime.server.lock().unwrap().clone().unwrap();
    server
        .browser_call("browser_navigate", json!({ "url": url }))
        .await
        .unwrap();
    let snapshot = server
        .browser_call("browser_snapshot", json!({}))
        .await
        .unwrap();
    assert!(
        serde_json::to_string(&snapshot)
            .unwrap()
            .contains("profile state preserved"),
        "sign-in survives encrypted Stop and reopen"
    );
    let after = app
        .browser_sessions
        .screenshot(
            app,
            &owner.id,
            &session.id,
            &chat_id,
            "After encrypted profile reopen",
        )
        .await
        .unwrap();
    copy_evidence(app, &after, &evidence_dir.join("browser-after.png"));
    app.browser_sessions
        .stop(app, &owner.id, &session.id)
        .await
        .unwrap();
    fixture.abort();
    println!("Visible browser evidence: {}", evidence_dir.display());
}

fn copy_evidence(app: &App, message: &Message, target: &std::path::Path) {
    let Body::Text { attachments, .. } = &message.body else {
        panic!("screenshot attachment")
    };
    std::fs::copy(crate::files::local_path(app, &attachments[0].id), target).unwrap();
}
