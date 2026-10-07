//! A stopped profile is an account-key encrypted archive, kept only on its Runner.
//! Chromium uses a private working directory while the session is open.

use std::path::{Path, PathBuf};

const MAX_PROFILE_BYTES: u64 = 512 * 1024 * 1024;

pub fn directory(home: &Path, id: &str) -> PathBuf {
    home.join(id)
}

fn archive_path(dir: &Path) -> PathBuf {
    dir.with_extension("enc")
}

pub fn restore(dir: &Path, dek: &[u8; 32], id: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    crate::config::set_private(dir).map_err(|e| e.to_string())?;
    // A interrupted process can leave a working profile. Do not overwrite it
    // with an older checkpoint.
    if dir.read_dir().map_err(|e| e.to_string())?.next().is_some() {
        return Ok(());
    }
    let path = archive_path(dir);
    if !path.is_file() {
        return Ok(());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let plaintext = crate::crypto::decrypt(dek, &format!("browser-profile/{id}"), &bytes)
        .map_err(|e| e.to_string())?;
    let mut archive = tar::Archive::new(&plaintext[..]);
    for entry in archive.entries().map_err(|e| e.to_string())? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        if !(entry.header().entry_type().is_dir() || entry.header().entry_type().is_file()) {
            return Err("Browser archive contains a link or special file.".into());
        }
        if !entry.unpack_in(dir).map_err(|e| e.to_string())? {
            return Err("Browser archive path escapes its profile.".into());
        }
    }
    Ok(())
}

pub fn seal(dir: &Path, dek: &[u8; 32], id: &str) -> Result<(), String> {
    if !dir.is_dir() {
        return Ok(());
    }
    let mut archive = tar::Builder::new(Vec::new());
    let mut total = 0;
    append(&mut archive, dir, dir, &mut total).map_err(|e| e.to_string())?;
    let bytes = archive.into_inner().map_err(|e| e.to_string())?;
    let encrypted = crate::crypto::encrypt(dek, &format!("browser-profile/{id}"), &bytes)
        .map_err(|e| e.to_string())?;
    let path = archive_path(dir);
    let pending = path.with_extension("enc.pending");
    crate::config::write_private(&pending, &encrypted).map_err(|e| e.to_string())?;
    std::fs::rename(pending, path).map_err(|e| e.to_string())?;
    std::fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
    Ok(())
}

fn append(
    archive: &mut tar::Builder<Vec<u8>>,
    root: &Path,
    dir: &Path,
    total: &mut u64,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        // Chromium's Singleton locks are links; caches regenerate and need not
        // retain copies of visited pages alongside the encrypted account data.
        let name = entry.file_name();
        if metadata.file_type().is_symlink()
            || matches!(
                name.to_str(),
                Some("Cache" | "Code Cache" | "GPUCache" | "Crashpad")
            )
        {
            continue;
        }
        if metadata.is_dir() {
            archive.append_dir(path.strip_prefix(root).unwrap(), &path)?;
            append(archive, root, &path, total)?;
        } else if metadata.is_file() {
            *total += metadata.len();
            if *total > MAX_PROFILE_BYTES {
                return Err(std::io::Error::other("Browser profile exceeds the 512 MiB checkpoint limit; the private working copy is retained."));
            }
            archive.append_path_with_name(&path, path.strip_prefix(root).unwrap())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopped_profile_is_encrypted_and_restores_sign_in_state() {
        let dir = std::env::temp_dir().join(format!("lorca-profile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("Default")).unwrap();
        std::fs::write(dir.join("Default/Cookies"), b"private sign-in cookie").unwrap();
        let key = crate::keys::random_32();
        seal(&dir, &key, "test").unwrap();
        assert!(!dir.exists());
        let bytes = std::fs::read(archive_path(&dir)).unwrap();
        assert!(!bytes.windows(7).any(|b| b == b"sign-in"));
        assert!(restore(&dir, &key, "another-session").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
        restore(&dir, &key, "test").unwrap();
        assert_eq!(
            std::fs::read(dir.join("Default/Cookies")).unwrap(),
            b"private sign-in cookie"
        );
        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_file(archive_path(&dir)).unwrap();
    }
}
