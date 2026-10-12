//! Files a relay on a LAN can hand out at `/static`, with no token. Ciphertext never
//! lives in this directory. A request whose real path leaves the directory is not a file
//! here.

use std::path::{Component, Path, PathBuf};

use axum::body::Body;
use axum::extract::{Path as UrlPath, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::AppState;

const LIST_LIMIT: usize = 200;

/// `GET /static`: the files under the directory, relative and sorted. 404 when unset.
pub async fn index(State(state): State<AppState>) -> Response {
    let Some(root) = state.static_dir.as_deref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match list(root) {
        Ok(files) => Json(json!({ "files": files })).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `GET /static/{path}`: one file's bytes. 404 when the directory is unset, the name
/// leaves it, or the path is not a file.
pub async fn file(State(state): State<AppState>, UrlPath(path): UrlPath<String>) -> Response {
    let Some(root) = state.static_dir.as_deref() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(file) = resolve(root, &path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let bytes = match tokio::fs::read(&file).await {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let mime = mime_of(&file);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .body(Body::from(bytes))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// The file `request` names inside `root`, after `..` and symlinks resolve. `None` when
/// it is not a file inside `root`.
pub fn resolve(root: &Path, request: &str) -> Option<PathBuf> {
    let request = request.trim_matches('/');
    if request.is_empty() || request.contains('\0') {
        return None;
    }
    let relative = Path::new(request);
    if relative.components().any(|component| matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
        return None;
    }
    let root = std::fs::canonicalize(root).ok()?;
    let file = std::fs::canonicalize(root.join(relative)).ok()?;
    (file.starts_with(&root) && file.is_file()).then_some(file)
}

fn list(root: &Path) -> std::io::Result<Vec<String>> {
    let root = std::fs::canonicalize(root)?;
    let mut files = Vec::new();
    walk(&root, &root, &mut files);
    files.sort();
    Ok(files)
}

fn walk(root: &Path, dir: &Path, files: &mut Vec<String>) {
    if files.len() >= LIST_LIMIT {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if files.len() >= LIST_LIMIT {
            return;
        }
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if meta.file_type().is_symlink() {
            let Ok(real) = std::fs::canonicalize(&path) else { continue };
            if real.starts_with(root) && real.is_file() {
                push_relative(root, &path, files);
            }
            continue;
        }
        if meta.is_dir() {
            walk(root, &path, files);
        } else if meta.is_file() {
            push_relative(root, &path, files);
        }
    }
}

fn push_relative(root: &Path, path: &Path, files: &mut Vec<String>) {
    if let Ok(relative) = path.strip_prefix(root) {
        files.push(relative.to_string_lossy().replace('\\', "/"));
    }
}

fn mime_of(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or("").to_ascii_lowercase().as_str() {
        "json" => "application/json",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_inside_is_served_and_a_path_that_leaves_is_not() {
        let root = std::env::temp_dir().join(format!("lorca-static-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("linux")).unwrap();
        std::fs::write(root.join("linux/lorca-relay"), b"binary").unwrap();
        let outside = std::env::temp_dir().join(format!("lorca-static-out-{}", uuid::Uuid::new_v4()));
        std::fs::write(&outside, b"nope").unwrap();
        assert!(resolve(&root, "linux/lorca-relay").is_some());
        assert!(resolve(&root, "../").is_none());
        assert!(resolve(&root, &format!("../{}", outside.file_name().unwrap().to_string_lossy())).is_none());
        assert!(resolve(&root, "/etc/hostname").is_none());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
            assert!(resolve(&root, "escape").is_none());
        }
        let listed = list(&root).unwrap();
        assert!(listed.iter().any(|name| name == "linux/lorca-relay"), "{listed:?}");
        assert!(listed.iter().all(|name| !name.contains("..")));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&outside);
    }
}
