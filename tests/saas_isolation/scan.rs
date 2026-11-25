//! After-the-run audits: what every user's files hold, and which credential
//! carried which user's content to the backend.

use std::path::Path;

use super::mock_llm::Recorded;
use super::world::{owners_in, USERS};

/// Every file under `dir`, recursively.
/// Directories or entries that cannot be read are recorded in `errors`: an
/// unreadable tree is an unaudited tree, not a clean one.
fn files(dir: &Path, out: &mut Vec<std::path::PathBuf>, errors: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            errors.push(format!("could not list {}: {e}", dir.display()));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                errors.push(format!("could not read an entry of {}: {e}", dir.display()));
                continue;
            }
        };
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => files(&path, out, errors),
            Ok(t) if t.is_file() => out.push(path),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => errors.push(format!("could not stat {}: {e}", path.display())),
        }
    }
}

/// Largest file the audit reads whole; a bigger one is itself a violation
/// rather than a silently unaudited file.
const MAX_AUDITED_BYTES: u64 = 512 * 1024 * 1024;

/// The user whose `users/<id>/` tree holds `path`, if any.
fn owner_of(root: &Path, path: &Path) -> Option<usize> {
    let rel = path.strip_prefix(root.join("users")).ok()?;
    let first = rel.components().next()?.as_os_str().to_str()?;
    USERS.iter().position(|u| *u == first)
}

/// Files that hold a canary they must not: another user's under a user's
/// tree, any user's outside every user's tree (the operator plane, shared
/// state, the child's `$HOME`). Returns the violations and, per user, how
/// many of their own files carry their canaries (proof the scan saw data).
pub fn scan_files(root: &Path, home: &Path, shared_storage: &Path) -> (Vec<String>, [usize; 3]) {
    let mut violations = Vec::new();
    let mut own = [0usize; 3];
    for base in [root, home] {
        let mut all = Vec::new();
        files(base, &mut all, &mut violations);
        for path in all {
            if base == home && path.starts_with(root) {
                continue;
            }
            // The configured shared SQLite database contains every tenant's
            // bytes by design. Tenant ownership is expressed by storage scope,
            // so raw database scans cannot classify those bytes. The suite
            // checks each tenant's persisted sentinel through the scoped RPC
            // surface and checks backend requests against their credentials.
            if is_shared_storage_artifact(&path, shared_storage) {
                continue;
            }
            // The child's own log sits beside the root; it is the operator's.
            if path.extension().is_some_and(|e| e == "log") && !path.starts_with(root) {
                continue;
            }
            let meta = match std::fs::metadata(&path) {
                Ok(meta) => meta,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    violations.push(format!("could not stat {}: {e}", path.display()));
                    continue;
                }
            };
            if meta.len() > MAX_AUDITED_BYTES {
                violations.push(format!(
                    "{} is {} bytes, too large to audit",
                    path.display(),
                    meta.len()
                ));
                continue;
            }
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    violations.push(format!("could not read {}: {e}", path.display()));
                    continue;
                }
            };
            let text = String::from_utf8_lossy(&bytes);
            // The process keyring (the dev file backend in this build) is one
            // store for the whole process: each profile's credential sits in
            // it under `<profile id>:…`. An entry is fine only under its own
            // owner's namespace.
            if path.file_name().is_some_and(|n| n == "dev-keychain.json") {
                violations.extend(keychain_violations(&path, &text));
                continue;
            }
            let found = owners_in(&text);
            if found.is_empty() {
                continue;
            }
            let owner = if base == root {
                owner_of(root, &path)
            } else {
                None
            };
            for user in found {
                if Some(user) == owner {
                    own[user] += 1;
                } else {
                    violations.push(format!(
                        "{} holds {}'s canary (owner: {})",
                        path.display(),
                        USERS[user],
                        owner.map(|o| USERS[o]).unwrap_or("nobody"),
                    ));
                }
            }
        }
    }
    (violations, own)
}

fn is_shared_storage_artifact(path: &Path, storage: &Path) -> bool {
    path == storage
        || ["-wal", "-shm"]
            .into_iter()
            .any(|suffix| path == Path::new(&format!("{}{suffix}", storage.display())))
}

/// Keyring entries holding a user's canary outside that user's namespace.
fn keychain_violations(path: &Path, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let map: std::collections::HashMap<String, String> = match serde_json::from_str(text) {
        Ok(map) => map,
        Err(e) => {
            out.push(format!("{}: malformed keychain data: {e}", path.display()));
            return out;
        }
    };
    for (key, value) in &map {
        let namespace = key.split(':').next().unwrap_or_default();
        for user in owners_in(value) {
            if namespace != USERS[user] {
                out.push(format!(
                    "{}: entry `{key}` holds {}'s secret",
                    path.display(),
                    USERS[user]
                ));
            }
        }
    }
    out
}

/// Requests whose content and credential disagree: any request carrying a
/// user's canary must carry that user's credential and no one else's
/// content, and every inference request must carry exactly one user's
/// credential. Returns the violations and inference requests per user.
pub fn audit_backend(requests: &[Recorded]) -> (Vec<String>, [usize; 3]) {
    let mut violations = Vec::new();
    let mut per_user = [0usize; 3];
    for r in requests {
        let by = owners_in(&r.auth);
        let content = owners_in(&r.body);
        if by.len() > 1 {
            violations.push(format!("{} carried several credentials: {:?}", r.path, by));
            continue;
        }
        let by = by.first().copied();
        if r.is_inference() {
            match by {
                Some(user) => per_user[user] += 1,
                None => violations.push(format!(
                    "inference request without a user credential (content of {:?}, auth `{}`)",
                    content.iter().map(|&u| USERS[u]).collect::<Vec<_>>(),
                    redact(&r.auth),
                )),
            }
        }
        for user in content {
            if Some(user) != by {
                violations.push(format!(
                    "{} carried {}'s content under {}'s credential",
                    r.path,
                    USERS[user],
                    by.map(|b| USERS[b]).unwrap_or("no user"),
                ));
            }
        }
    }
    (violations, per_user)
}

fn redact(auth: &str) -> String {
    auth.chars().take(12).collect::<String>() + "…"
}

#[cfg(test)]
#[path = "scan_tests.rs"]
mod tests;
