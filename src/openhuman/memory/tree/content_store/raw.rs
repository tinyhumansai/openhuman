//! On-disk archive of raw provider items (one .md per source item).
//!
//! Lives alongside the chunked content store but writes a *separate*
//! tree at `<content_root>/raw/<source_slug>/<kind>/<created_at_ms>_<uid>.md`,
//! where `<kind>` is one of `emails`, `chats`, `documents`, `contacts`,
//! `posts` (see [`RawKind`]). The kind subdir keeps a single source's
//! items split by category so Obsidian `.base` files at
//! `<content_root>/raw/<source_slug>/<kind>.base` can render
//! per-category views. Contacts and documents are scoped to one source.
//!
//! This is the verbatim payload captured at sync time — no chunking, no
//! summarisation. Useful for:
//!
//!   - feeding Obsidian a per-message file the user can read directly,
//!   - reproducing the original ingest input when debugging chunker
//!     output,
//!   - diffing future re-syncs without round-tripping through the
//!     chunker.
//!
//! Each file is written atomically (tempfile + rename) so a partial
//! write can never leak into the directory listing. Re-writing the
//! same `(source, uid, ts)` triple is idempotent — same path, same
//! bytes when the upstream item is unchanged.
//!
//! Naming: `<created_at_ms>_<uid>.md` puts the on-disk listing in
//! chronological order while keeping a stable identity suffix so
//! re-syncing the same message overwrites the same file.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::paths::slugify_source_id;

/// Category of a raw item. Used to split a single source's items into
/// per-kind subdirectories under `raw/<source_slug>/<kind>/`.
///
/// Each connector picks a kind per item — a single connector can write
/// into multiple kinds (e.g. Gmail → [`Self::Email`] for messages,
/// [`Self::Contact`] for senders, [`Self::Document`] for attachments).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum RawKind {
    /// Email messages (Gmail, Outlook, …).
    Email,
    /// Chat / DM messages (Slack, Telegram, WhatsApp, Discord, …).
    Chat,
    /// Standalone documents — Notion pages, Drive files, attachments.
    Document,
    /// One file per person reachable via this source.
    Contact,
    /// Long-form posts — LinkedIn posts, tweets, blog entries.
    Post,
}

impl RawKind {
    /// Directory name used on disk for this kind. Plural to match the
    /// canonical layout (`emails/`, `chats/`, `documents/`, …).
    pub const fn as_dir(&self) -> &'static str {
        match self {
            Self::Email => "emails",
            Self::Chat => "chats",
            Self::Document => "documents",
            Self::Contact => "contacts",
            Self::Post => "posts",
        }
    }
}

/// One raw item ready to land on disk.
pub struct RawItem<'a> {
    /// Stable upstream identifier (e.g. Gmail message id). Used for the
    /// filename suffix; sanitised before being placed in a path.
    pub uid: &'a str,
    /// Authoritative timestamp from the upstream item (ms since epoch).
    /// Drives the filename prefix so files sort chronologically in any
    /// file browser.
    pub created_at_ms: i64,
    /// Markdown body to write. Should be self-contained (front-matter
    /// optional but encouraged).
    pub markdown: &'a str,
    /// Category subdir under the source (`emails/`, `chats/`, …).
    pub kind: RawKind,
}

/// Write a batch of raw items under `raw/<source_slug>/<kind>/`.
///
/// `content_root` is the same root that backs `chunk_rel_path` /
/// `summary_rel_path` — i.e. `<workspace>/memory_tree/content/`.
/// `source_id` is the chunk-store source id (e.g.
/// `"gmail:stevent95-at-gmail-dot-com"`); we slugify it the same way
/// the chunk path does so the raw and chunk trees line up under
/// matching directory names. Each item carries its own [`RawKind`],
/// which selects the per-kind subdir.
///
/// Returns the number of files written.
pub fn write_raw_items(
    content_root: &Path,
    source_id: &str,
    items: &[RawItem<'_>],
) -> Result<usize> {
    if items.is_empty() {
        return Ok(0);
    }
    let mut written = 0usize;
    for item in items {
        let dir = raw_kind_dir(content_root, source_id, item.kind);
        fs::create_dir_all(&dir).with_context(|| format!("create raw dir {}", dir.display()))?;
        let filename = build_filename(item.created_at_ms, item.uid);
        let path = dir.join(&filename);
        write_atomic(&path, item.markdown.as_bytes())
            .with_context(|| format!("write raw file {}", path.display()))?;
        written += 1;
    }
    Ok(written)
}

/// Resolve the on-disk directory for a source's raw archive (the
/// per-source folder that holds every kind subdir plus `_source.md`
/// and `<kind>.base` views).
pub fn raw_source_dir(content_root: &Path, source_id: &str) -> PathBuf {
    let slug = slugify_source_id(source_id);
    content_root.join("raw").join(slug)
}

/// Resolve the on-disk directory for a single kind under a source —
/// e.g. `<root>/raw/<source_slug>/emails/`.
pub fn raw_kind_dir(content_root: &Path, source_id: &str, kind: RawKind) -> PathBuf {
    raw_source_dir(content_root, source_id).join(kind.as_dir())
}

/// Forward-slash relative path of a raw file under `<content_root>/`,
/// e.g. `"raw/gmail-acct/emails/1700000000000_msg-1.md"`. Used by
/// callers that record a [`crate::openhuman::memory::tree::store::RawRef`]
/// so reads can resolve the file later without re-deriving the layout.
pub fn raw_rel_path(source_id: &str, kind: RawKind, created_at_ms: i64, uid: &str) -> String {
    let slug = slugify_source_id(source_id);
    let filename = build_filename(created_at_ms, uid);
    format!("raw/{}/{}/{}", slug, kind.as_dir(), filename)
}

fn build_filename(created_at_ms: i64, uid: &str) -> String {
    let ts = created_at_ms.max(0);
    let uid = sanitize_uid(uid);
    format!("{ts}_{uid}.md")
}

/// Replace path-illegal characters in the upstream uid before splicing
/// it into a filename. Mirrors `paths::sanitize_filename` but is local
/// so a future change to either side stays decoupled.
fn sanitize_uid(uid: &str) -> String {
    let cleaned: String = uid
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | ' ' => '-',
            other => other,
        })
        .collect();
    if cleaned.is_empty() {
        "unknown".into()
    } else {
        cleaned
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("path has no parent: {}", path.display()))?;
    // Per-writer unique tempfile so two concurrent ingest workers
    // staging into the same source folder can't trample each other's
    // staging path. PID + nanos is collision-free for any realistic
    // local concurrency level; the tempfile lands in `parent` so the
    // subsequent `rename` is still atomic-on-same-filesystem.
    let tmp = parent.join(format!(
        ".tmp_raw_{}_{}.md",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut f = fs::File::create(&tmp).with_context(|| format!("create tmp {}", tmp.display()))?;
    f.write_all(bytes)
        .with_context(|| format!("write tmp {}", tmp.display()))?;
    f.sync_all()
        .with_context(|| format!("fsync tmp {}", tmp.display()))?;
    drop(f);
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    // Best-effort fsync of the directory so the rename is durable on
    // crash. We don't surface as an error (the rename has already
    // committed; missing dirent fsync is a durability degradation,
    // not a failure), but operators want visibility when it happens.
    if let Ok(dir_handle) = fs::File::open(parent) {
        if let Err(e) = dir_handle.sync_all() {
            // Avoid logging the absolute path (embeds workspace /
            // home directory). The basename is enough signal for
            // operators to correlate with the source slug.
            let dir_hint = parent
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("<unknown>");
            log::debug!("[content_store::raw] parent dir fsync failed dir={dir_hint} err={e}");
        }
    }
    Ok(())
}

/// Slug an account email like `stevent95@gmail.com` to
/// `stevent95-at-gmail-dot-com`. Used to build per-account source ids
/// from the Composio connection's account email so every memory
/// source is uniquely identified by its connection identity.
///
/// Rules:
/// - lowercase
/// - `@` → `-at-`
/// - `.` → `-dot-`
/// - any other non-`[a-z0-9]` run collapses to a single `-`
/// - trim leading/trailing `-`
pub fn slug_account_email(email: &str) -> String {
    let lower = email.trim().to_lowercase();
    let mut out = String::with_capacity(lower.len() + 8);
    let mut last_dash = true;
    let mut chars = lower.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '@' => {
                if !last_dash {
                    out.push('-');
                }
                out.push_str("at-");
                last_dash = true;
            }
            '.' => {
                if !last_dash {
                    out.push('-');
                }
                out.push_str("dot-");
                last_dash = true;
            }
            c if c.is_ascii_alphanumeric() => {
                out.push(c);
                last_dash = false;
            }
            _ => {
                if !last_dash {
                    out.push('-');
                    last_dash = true;
                }
            }
        }
    }
    let trimmed = out.trim_end_matches('-').trim_start_matches('-');
    if trimmed.is_empty() {
        "unknown".into()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn slug_account_email_basic() {
        assert_eq!(
            slug_account_email("stevent95@gmail.com"),
            "stevent95-at-gmail-dot-com"
        );
    }

    #[test]
    fn slug_account_email_lowercases_and_trims() {
        assert_eq!(
            slug_account_email("  Alice.Smith@Example.CO.UK "),
            "alice-dot-smith-at-example-dot-co-dot-uk"
        );
    }

    #[test]
    fn slug_account_email_handles_plus_aliases() {
        assert_eq!(
            slug_account_email("alice+work@example.com"),
            "alice-work-at-example-dot-com"
        );
    }

    #[test]
    fn slug_account_email_falls_back_to_unknown() {
        assert_eq!(slug_account_email(""), "unknown");
        assert_eq!(slug_account_email("@@@"), "at-at-at");
        assert_eq!(slug_account_email("///"), "unknown");
    }

    #[test]
    fn write_raw_items_creates_named_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let items = [
            RawItem {
                uid: "msg-1",
                created_at_ms: 1_700_000_000_000,
                markdown: "# hello",
                kind: RawKind::Email,
            },
            RawItem {
                uid: "msg-2",
                created_at_ms: 1_700_000_010_000,
                markdown: "# world",
                kind: RawKind::Email,
            },
        ];
        let n = write_raw_items(root, "gmail:stevent95-at-gmail-dot-com", &items).unwrap();
        assert_eq!(n, 2);
        let dir = raw_kind_dir(root, "gmail:stevent95-at-gmail-dot-com", RawKind::Email);
        assert!(
            dir.exists(),
            "raw dir should be created at {}",
            dir.display()
        );
        // Source-level dir is the parent of the kind dir.
        assert_eq!(
            dir.parent().unwrap(),
            raw_source_dir(root, "gmail:stevent95-at-gmail-dot-com")
        );
        // Files must sort chronologically (created_at_ms prefix).
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "1700000000000_msg-1.md".to_string(),
                "1700000010000_msg-2.md".to_string()
            ]
        );
    }

    #[test]
    fn write_raw_items_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let item = RawItem {
            uid: "msg-1",
            created_at_ms: 1_700_000_000_000,
            markdown: "v1",
            kind: RawKind::Email,
        };
        write_raw_items(root, "gmail:acct", &[item]).unwrap();
        let item2 = RawItem {
            uid: "msg-1",
            created_at_ms: 1_700_000_000_000,
            markdown: "v2",
            kind: RawKind::Email,
        };
        write_raw_items(root, "gmail:acct", &[item2]).unwrap();
        let dir = raw_kind_dir(root, "gmail:acct", RawKind::Email);
        let path = dir.join("1700000000000_msg-1.md");
        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(body, "v2");
    }

    #[test]
    fn write_raw_items_sanitises_uid_path_chars() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let item = RawItem {
            uid: "msg/with:dangerous*chars",
            created_at_ms: 0,
            markdown: "x",
            kind: RawKind::Email,
        };
        write_raw_items(root, "gmail:acct", &[item]).unwrap();
        let dir = raw_kind_dir(root, "gmail:acct", RawKind::Email);
        let entries: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].starts_with("0_msg-with-dangerous-chars"));
    }

    #[test]
    fn write_raw_items_empty_is_noop() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let n = write_raw_items(root, "gmail:acct", &[]).unwrap();
        assert_eq!(n, 0);
        // Neither source nor any kind dir should exist for an empty batch.
        assert!(!raw_source_dir(root, "gmail:acct").exists());
        assert!(!raw_kind_dir(root, "gmail:acct", RawKind::Email).exists());
    }

    #[test]
    fn write_raw_items_splits_kinds_into_subdirs() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let items = [
            RawItem {
                uid: "msg-1",
                created_at_ms: 1_700_000_000_000,
                markdown: "email",
                kind: RawKind::Email,
            },
            RawItem {
                uid: "person-1",
                created_at_ms: 0,
                markdown: "contact",
                kind: RawKind::Contact,
            },
        ];
        let n = write_raw_items(root, "gmail:acct", &items).unwrap();
        assert_eq!(n, 2);
        assert!(raw_kind_dir(root, "gmail:acct", RawKind::Email)
            .join("1700000000000_msg-1.md")
            .exists());
        assert!(raw_kind_dir(root, "gmail:acct", RawKind::Contact)
            .join("0_person-1.md")
            .exists());
    }

    #[test]
    fn raw_rel_path_uses_kind_subdir() {
        assert_eq!(
            raw_rel_path("gmail:acct", RawKind::Email, 1_700_000_000_000, "msg-1"),
            "raw/gmail-acct/emails/1700000000000_msg-1.md"
        );
        assert_eq!(
            raw_rel_path("slack:team", RawKind::Chat, 42, "msg/with:bad"),
            "raw/slack-team/chats/42_msg-with-bad.md"
        );
    }
}
