//! Git-backed persistence for memory diff snapshots, checkpoints, and read
//! markers.
//!
//! The ledger is a libgit2 repository at `<workspace>/memory_diff/repo`.
//! It is a *derived* view of the chunk store — `mem_tree_chunks` remains the
//! authoritative source of memory. Each snapshot materialises a source's
//! current items as blobs under `<source_id>/` and records them as a commit;
//! the rest of the tree (other sources) is carried forward from the parent so
//! HEAD always reflects the whole world. This maps the diff domain onto git's
//! native primitives:
//!
//! - **Snapshot**  → commit (`Snapshot.id` is the commit SHA)
//! - **Checkpoint**→ annotated tag `ckpt_<uuid>` at HEAD
//! - **Read marker**→ ref `refs/openhuman/read/<source_id>` → commit SHA
//! - **Diff**       → `git diff <from-tree>..<to-tree>` scoped to the source path
//!
//! Item identity is the file name: each item is one flat blob whose name is the
//! item id encoded into a git-safe path component (`encode_item_id`). A content
//! change keeps the same name → `Modified`; renaming the item id is
//! `Removed` + `Added`, matching the previous id-keyed semantics.
//!
//! Snapshot metadata that has no natural git home (source kind/label, trigger,
//! item count, millisecond timestamp) rides in the commit message as trailers.
//! All mutations serialise through a process-global [`WRITE_LOCK`] because the
//! repository's parent/HEAD bookkeeping is not safe to interleave.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use git2::{Delta, DiffOptions, Object, ObjectType, Oid, Repository, Signature, Time};

use super::types::{ChangeKind, Checkpoint, DiffSummary, ItemChange, Snapshot, SnapshotTrigger};

/// Serialises all writes (commits, tags, ref updates) to the ledger. libgit2's
/// HEAD/parent resolution is read-modify-write, so concurrent commits could
/// otherwise fork history or lose a snapshot.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

const BLOB_MODE: i32 = 0o100644;
const TREE_MODE: i32 = 0o040000;
const SIG_NAME: &str = "OpenHuman Memory";
const SIG_EMAIL: &str = "memory-diff@openhuman.local";
const READ_MARKER_PREFIX: &str = "refs/openhuman/read/";
const CHECKPOINT_PREFIX: &str = "ckpt_";

/// Upper bound on a single modified-item unified diff embedded in `text_diff`.
const MAX_TEXT_DIFF_CHARS: usize = 2000;

// ── Repository handle ──────────────────────────────────────────────────

/// A handle to the diff ledger. Cheap to open; callers construct one per
/// blocking task (mirroring the previous `with_connection` pattern).
pub struct Ledger {
    repo: Repository,
}

/// Metadata describing the snapshot a commit represents. Persisted as commit
/// trailers and reconstructed by [`Ledger::snapshot_from_commit`].
pub struct SnapshotMeta {
    pub source_id: String,
    pub source_kind: String,
    pub label: String,
    pub trigger: SnapshotTrigger,
}

impl Ledger {
    /// Open the ledger, initialising the repository on first use.
    pub fn open(workspace_dir: &Path) -> Result<Self> {
        let repo_path = workspace_dir.join("memory_diff").join("repo");
        std::fs::create_dir_all(&repo_path)
            .with_context(|| format!("create memory_diff repo dir: {}", repo_path.display()))?;

        let repo = match Repository::open(&repo_path) {
            Ok(repo) => repo,
            Err(_) => {
                tracing::debug!(
                    path = %repo_path.display(),
                    "[memory_diff::git] initialising diff ledger repository"
                );
                Repository::init(&repo_path)
                    .with_context(|| format!("init memory_diff repo: {}", repo_path.display()))?
            }
        };
        Ok(Self { repo })
    }

    // ── Snapshots (commits) ────────────────────────────────────────────

    /// Commit a snapshot for one source: replace the source's subtree with the
    /// given items (each `(item_id, content)`), carrying every other source
    /// forward from the parent. Returns the resulting [`Snapshot`].
    pub fn commit_snapshot(
        &self,
        meta: &SnapshotMeta,
        items: &[(String, String)],
        taken_at_ms: i64,
    ) -> Result<Snapshot> {
        let _guard = WRITE_LOCK.lock().expect("memory_diff write lock poisoned");

        // Build the source subtree from scratch: one blob per item.
        let source_tree_oid = {
            let mut tb = self.repo.treebuilder(None)?;
            for (item_id, content) in items {
                let blob = self.repo.blob(content.as_bytes())?;
                tb.insert(encode_item_id(item_id), blob, BLOB_MODE)?;
            }
            tb.write()?
        };

        // Start the root tree from the parent commit (carry other sources),
        // then graft in the new source subtree (or drop it if empty).
        let parent_commit = match self.repo.head() {
            Ok(head) => Some(head.peel_to_commit()?),
            Err(_) => None, // unborn HEAD on a fresh repo
        };
        let parent_root = match &parent_commit {
            Some(c) => Some(c.tree()?),
            None => None,
        };
        let root_oid = {
            let mut tb = self.repo.treebuilder(parent_root.as_ref())?;
            if items.is_empty() {
                if tb.get(meta.source_id.as_str())?.is_some() {
                    tb.remove(meta.source_id.as_str())?;
                }
            } else {
                tb.insert(meta.source_id.as_str(), source_tree_oid, TREE_MODE)?;
            }
            tb.write()?
        };
        let tree = self.repo.find_tree(root_oid)?;

        let message = build_commit_message(meta, items.len() as u32, taken_at_ms);
        let sig = signature(taken_at_ms)?;
        let parents: Vec<&git2::Commit> = parent_commit.iter().collect();
        let commit_oid = self
            .repo
            .commit(Some("HEAD"), &sig, &sig, &message, &tree, &parents)
            .context("write snapshot commit")?;

        tracing::debug!(
            commit = %commit_oid,
            source_id = %meta.source_id,
            items = items.len(),
            "[memory_diff::git] snapshot committed"
        );

        Ok(Snapshot {
            id: commit_oid.to_string(),
            source_id: meta.source_id.clone(),
            source_kind: meta.source_kind.clone(),
            label: meta.label.clone(),
            trigger: meta.trigger.clone(),
            item_count: items.len() as u32,
            taken_at_ms,
        })
    }

    /// List snapshots newest-first, optionally filtered to one source.
    ///
    /// Walks the commit history from HEAD; each commit is one source's
    /// snapshot, identified by its `Source-Id` trailer.
    pub fn list_snapshots(&self, source_id: Option<&str>, limit: u32) -> Result<Vec<Snapshot>> {
        let mut walk = match self.repo.revwalk() {
            Ok(w) => w,
            Err(_) => return Ok(Vec::new()),
        };
        if walk.push_head().is_err() {
            // Unborn HEAD → no snapshots yet.
            return Ok(Vec::new());
        }
        walk.set_sorting(git2::Sort::TIME)?;

        let mut out = Vec::new();
        for oid in walk {
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;
            let snap = self.snapshot_from_commit(&commit);
            if let Some(filter) = source_id {
                if snap.source_id != filter {
                    continue;
                }
            }
            out.push(snap);
            if out.len() as u32 >= limit {
                break;
            }
        }
        Ok(out)
    }

    /// Fetch a single snapshot by commit SHA, if it exists.
    pub fn get_snapshot(&self, snapshot_id: &str) -> Result<Option<Snapshot>> {
        let Ok(oid) = Oid::from_str(snapshot_id) else {
            return Ok(None);
        };
        match self.repo.find_commit(oid) {
            Ok(commit) => Ok(Some(self.snapshot_from_commit(&commit))),
            Err(_) => Ok(None),
        }
    }

    /// The `count` most recent snapshots for a source, newest-first.
    pub fn latest_snapshots_for_source(
        &self,
        source_id: &str,
        count: u32,
    ) -> Result<Vec<Snapshot>> {
        self.list_snapshots(Some(source_id), count)
    }

    /// Number of distinct sources that have at least one snapshot.
    pub fn snapshot_count_for_source(&self, source_id: &str) -> Result<usize> {
        Ok(self.list_snapshots(Some(source_id), u32::MAX)?.len())
    }

    // ── Diff (tree-to-tree) ─────────────────────────────────────────────

    /// Compute item-level changes for `source_id` between two snapshots.
    ///
    /// `from` is `None` for a first-ever diff (everything added). Both commits
    /// must belong to `source_id`; cross-source mixing is rejected by the
    /// caller before reaching here.
    pub fn compute_changes(
        &self,
        from: Option<&str>,
        to: &str,
        source_id: &str,
        to_item_count: u32,
        include_text_diff: bool,
    ) -> Result<(Vec<ItemChange>, DiffSummary)> {
        let to_oid = Oid::from_str(to).with_context(|| format!("bad to snapshot id: {to}"))?;
        let to_tree = self.repo.find_commit(to_oid)?.tree()?;

        let from_tree = match from {
            Some(f) => {
                let oid = Oid::from_str(f).with_context(|| format!("bad from snapshot id: {f}"))?;
                Some(self.repo.find_commit(oid)?.tree()?)
            }
            None => None,
        };

        let path_prefix = format!("{source_id}/");
        let mut opts = DiffOptions::new();
        opts.pathspec(source_id);
        opts.context_lines(3);
        let diff =
            self.repo
                .diff_tree_to_tree(from_tree.as_ref(), Some(&to_tree), Some(&mut opts))?;

        let mut changes = Vec::new();
        let mut summary = DiffSummary::default();

        for (idx, delta) in diff.deltas().enumerate() {
            // Resolve the item path; guard against pathspec prefix overreach
            // (e.g. "src_a" must not match "src_abc/...").
            let path = delta
                .new_file()
                .path()
                .or_else(|| delta.old_file().path())
                .and_then(|p| p.to_str())
                .unwrap_or("");
            let Some(encoded) = path.strip_prefix(&path_prefix) else {
                continue;
            };
            let item_id = decode_item_id(encoded);

            let new_oid = delta.new_file().id();
            let old_oid = delta.old_file().id();

            let (kind, title) = match delta.status() {
                Delta::Added | Delta::Copied | Delta::Untracked => {
                    summary.added += 1;
                    (ChangeKind::Added, self.title_for(&item_id, new_oid))
                }
                Delta::Deleted => {
                    summary.removed += 1;
                    (ChangeKind::Removed, self.title_for(&item_id, old_oid))
                }
                Delta::Modified | Delta::Renamed | Delta::Typechange => {
                    summary.modified += 1;
                    (ChangeKind::Modified, self.title_for(&item_id, new_oid))
                }
                // Unmodified / ignored / conflicted: nothing to report.
                _ => continue,
            };

            let text_diff = if include_text_diff && kind == ChangeKind::Modified {
                patch_text(&diff, idx)
            } else {
                None
            };

            changes.push(ItemChange {
                item_id,
                title,
                kind,
                old_content_hash: oid_hash(old_oid),
                new_content_hash: oid_hash(new_oid),
                text_diff,
            });
        }

        // git only reports changed entries; unchanged = everything in `to`
        // that wasn't added or modified.
        summary.unchanged = to_item_count
            .saturating_sub(summary.added)
            .saturating_sub(summary.modified);

        Ok((changes, summary))
    }

    // ── Read markers (refs) ─────────────────────────────────────────────

    /// The commit SHA a source's read marker points at, if set.
    pub fn get_read_marker(&self, source_id: &str) -> Result<Option<String>> {
        let name = read_marker_ref(source_id);
        match self.repo.find_reference(&name) {
            Ok(r) => Ok(r.target().map(|o| o.to_string())),
            Err(_) => Ok(None),
        }
    }

    /// Set (or advance) a source's read marker to a commit SHA.
    pub fn set_read_marker(&self, source_id: &str, snapshot_id: &str) -> Result<()> {
        let _guard = WRITE_LOCK.lock().expect("memory_diff write lock poisoned");
        let oid = Oid::from_str(snapshot_id)
            .with_context(|| format!("bad read-marker snapshot id: {snapshot_id}"))?;
        let name = read_marker_ref(source_id);
        self.repo
            .reference(&name, oid, true, "advance memory_diff read marker")
            .with_context(|| format!("set read marker ref: {name}"))?;
        Ok(())
    }

    // ── Checkpoints (tags) ──────────────────────────────────────────────

    /// Create an annotated tag at HEAD recording a checkpoint. The label and
    /// per-source head snapshot ids ride in the tag message as JSON.
    pub fn create_checkpoint(
        &self,
        id: &str,
        label: &str,
        snapshot_ids: &[String],
        created_at_ms: i64,
    ) -> Result<()> {
        let _guard = WRITE_LOCK.lock().expect("memory_diff write lock poisoned");
        let head = self
            .repo
            .head()
            .context("checkpoint requires at least one snapshot")?
            .peel_to_commit()?;
        let target: Object = head.into_object();
        let sig = signature(created_at_ms)?;
        let message = checkpoint_message(label, snapshot_ids, created_at_ms);
        self.repo
            .tag(id, &target, &sig, &message, true)
            .with_context(|| format!("create checkpoint tag: {id}"))?;
        Ok(())
    }

    /// Load a checkpoint by tag name.
    pub fn get_checkpoint(&self, checkpoint_id: &str) -> Result<Option<Checkpoint>> {
        let refname = format!("refs/tags/{checkpoint_id}");
        let Ok(reference) = self.repo.find_reference(&refname) else {
            return Ok(None);
        };
        let obj = reference.peel(ObjectType::Tag).ok();
        let Some(tag) = obj.and_then(|o| o.into_tag().ok()) else {
            return Ok(None);
        };
        Ok(Some(checkpoint_from_message(
            checkpoint_id,
            tag.message().ok().flatten().unwrap_or(""),
        )))
    }

    /// List checkpoints newest-first, up to `limit`.
    pub fn list_checkpoints(&self, limit: u32) -> Result<Vec<Checkpoint>> {
        let pattern = format!("{CHECKPOINT_PREFIX}*");
        let names = self.repo.tag_names(Some(&pattern))?;
        let mut out = Vec::new();
        // StringArray::iter() yields Result<Option<&str>, _>; drop errors/non-utf8.
        for name in names.iter().flatten().flatten() {
            if let Some(ckpt) = self.get_checkpoint(name)? {
                out.push(ckpt);
            }
        }
        out.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
        out.truncate(limit as usize);
        Ok(out)
    }

    /// Delete checkpoint tags created before `older_than_ms`. Snapshot commits
    /// are retained — git history is the ledger — so this only prunes named
    /// baselines. Returns the number of tags deleted.
    pub fn cleanup_checkpoints(&self, older_than_ms: i64) -> Result<u64> {
        let _guard = WRITE_LOCK.lock().expect("memory_diff write lock poisoned");
        let pattern = format!("{CHECKPOINT_PREFIX}*");
        let names = self.repo.tag_names(Some(&pattern))?;
        let mut deleted = 0u64;
        for name in names.iter().flatten().flatten() {
            if let Some(ckpt) = self.get_checkpoint(name)? {
                if ckpt.created_at_ms < older_than_ms {
                    self.repo.tag_delete(name)?;
                    deleted += 1;
                }
            }
        }
        Ok(deleted)
    }

    // ── Helpers ─────────────────────────────────────────────────────────

    /// Reconstruct a [`Snapshot`] from a commit's trailers, falling back to
    /// the commit time when a millisecond trailer is absent.
    fn snapshot_from_commit(&self, commit: &git2::Commit) -> Snapshot {
        let trailers = parse_trailers(commit.message().unwrap_or(""));
        let taken_at_ms = trailers
            .get("taken-at-ms")
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or_else(|| commit.time().seconds() * 1000);
        Snapshot {
            id: commit.id().to_string(),
            source_id: trailers.get("source-id").cloned().unwrap_or_default(),
            source_kind: trailers.get("source-kind").cloned().unwrap_or_default(),
            label: trailers.get("source-label").cloned().unwrap_or_default(),
            trigger: match trailers.get("trigger").map(String::as_str) {
                Some("manual") => SnapshotTrigger::Manual,
                _ => SnapshotTrigger::Auto,
            },
            item_count: trailers
                .get("item-count")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0),
            taken_at_ms,
        }
    }

    /// Derive a display title from a blob's content. Returns the item id when
    /// the blob is missing or yields no usable line.
    fn title_for(&self, item_id: &str, oid: Oid) -> String {
        if oid.is_zero() {
            return item_id.to_string();
        }
        match self.repo.find_blob(oid) {
            Ok(blob) => {
                let content = String::from_utf8_lossy(blob.content());
                derive_title(item_id, &content)
            }
            Err(_) => item_id.to_string(),
        }
    }
}

// ── Free helpers ───────────────────────────────────────────────────────

fn signature(at_ms: i64) -> Result<Signature<'static>> {
    let time = Time::new(at_ms / 1000, 0);
    Signature::new(SIG_NAME, SIG_EMAIL, &time).context("build git signature")
}

fn read_marker_ref(source_id: &str) -> String {
    format!("{READ_MARKER_PREFIX}{}", encode_item_id(source_id))
}

fn build_commit_message(meta: &SnapshotMeta, item_count: u32, taken_at_ms: i64) -> String {
    format!(
        "snapshot: {source} ({count} item(s))\n\n\
         Source-Id: {source}\n\
         Source-Kind: {kind}\n\
         Source-Label: {label}\n\
         Trigger: {trigger}\n\
         Item-Count: {count}\n\
         Taken-At-Ms: {taken}\n",
        source = meta.source_id,
        kind = meta.source_kind,
        label = sanitize_trailer(&meta.label),
        trigger = meta.trigger.as_str(),
        count = item_count,
        taken = taken_at_ms,
    )
}

/// Trailer values are single-line; collapse newlines so a multi-line label
/// can't corrupt the trailer block.
fn sanitize_trailer(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

/// Parse `Key: value` trailer lines from a commit message into a lowercase-keyed map.
fn parse_trailers(message: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for line in message.lines() {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            if !key.is_empty() && !key.contains(' ') {
                map.insert(key, v.trim().to_string());
            }
        }
    }
    map
}

fn checkpoint_message(label: &str, snapshot_ids: &[String], created_at_ms: i64) -> String {
    let payload = serde_json::json!({
        "label": label,
        "snapshot_ids": snapshot_ids,
        "created_at_ms": created_at_ms,
    });
    payload.to_string()
}

fn checkpoint_from_message(id: &str, message: &str) -> Checkpoint {
    let value: serde_json::Value = serde_json::from_str(message.trim()).unwrap_or_default();
    Checkpoint {
        id: id.to_string(),
        label: value
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        created_at_ms: value
            .get("created_at_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        snapshot_ids: value
            .get("snapshot_ids")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// A git blob oid as a content hash, or `None` for the zero oid (absent side).
fn oid_hash(oid: Oid) -> Option<String> {
    if oid.is_zero() {
        None
    } else {
        Some(oid.to_string())
    }
}

/// Render a single delta's unified patch, truncated to [`MAX_TEXT_DIFF_CHARS`].
fn patch_text(diff: &git2::Diff, delta_idx: usize) -> Option<String> {
    let mut patch = git2::Patch::from_diff(diff, delta_idx).ok().flatten()?;
    let buf = patch.to_buf().ok()?;
    let text = buf.as_str().ok()?;
    if text.trim().is_empty() {
        None
    } else {
        Some(truncate(text, MAX_TEXT_DIFF_CHARS))
    }
}

/// Encode an item id into a single git-safe path component. Bytes outside
/// `[A-Za-z0-9._-]` become `%XX`; an `i_` prefix keeps the result clear of the
/// reserved names `.`/`..`/empty. Reversible via [`decode_item_id`].
fn encode_item_id(item_id: &str) -> String {
    let mut out = String::with_capacity(item_id.len() + 2);
    out.push_str("i_");
    for &b in item_id.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

/// Inverse of [`encode_item_id`].
fn decode_item_id(encoded: &str) -> String {
    let body = encoded.strip_prefix("i_").unwrap_or(encoded);
    let bytes = body.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        s.to_string()
    } else {
        let mut end = max_chars;
        while !s.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        format!("{}…(truncated)", &s[..end])
    }
}

/// Derive a human-readable title from item content: the first non-empty line
/// (Markdown heading markers stripped), bounded. Falls back to the item id.
fn derive_title(item_id: &str, content: &str) -> String {
    let first_line = content
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.trim_start_matches('#').trim());
    match first_line {
        Some(l) if !l.is_empty() => truncate(l, 120),
        _ => item_id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_ledger() -> (Ledger, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open(dir.path()).unwrap();
        (ledger, dir)
    }

    fn meta(source_id: &str) -> SnapshotMeta {
        SnapshotMeta {
            source_id: source_id.to_string(),
            source_kind: "folder".to_string(),
            label: "Docs".to_string(),
            trigger: SnapshotTrigger::Auto,
        }
    }

    fn items(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn encode_decode_round_trips() {
        for id in [
            "readme.md",
            "path/to/file.md",
            "user@example.com:msg_xxx",
            "weird name (1)!",
            "..",
            ".",
        ] {
            let enc = encode_item_id(id);
            assert!(!enc.contains('/'), "no slash in {enc}");
            assert!(enc != "." && enc != ".." && !enc.is_empty());
            assert_eq!(decode_item_id(&enc), id, "round trip for {id}");
        }
    }

    #[test]
    fn commit_and_list_snapshots() {
        let (ledger, _dir) = temp_ledger();
        assert!(ledger.list_snapshots(None, 10).unwrap().is_empty());

        let snap = ledger
            .commit_snapshot(&meta("src_a"), &items(&[("a", "alpha")]), 1000)
            .unwrap();
        assert_eq!(snap.source_id, "src_a");
        assert_eq!(snap.item_count, 1);
        assert_eq!(snap.taken_at_ms, 1000);

        let listed = ledger.list_snapshots(Some("src_a"), 10).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, snap.id);

        let fetched = ledger.get_snapshot(&snap.id).unwrap().unwrap();
        assert_eq!(fetched.source_id, "src_a");
        assert_eq!(fetched.label, "Docs");
        assert_eq!(fetched.item_count, 1);
    }

    #[test]
    fn snapshots_carry_other_sources_forward() {
        let (ledger, _dir) = temp_ledger();
        ledger
            .commit_snapshot(&meta("src_a"), &items(&[("a", "alpha")]), 1000)
            .unwrap();
        let b = ledger
            .commit_snapshot(&meta("src_b"), &items(&[("b", "beta")]), 2000)
            .unwrap();

        // src_a remains listable after a src_b commit (carried forward in tree).
        assert_eq!(ledger.list_snapshots(Some("src_a"), 10).unwrap().len(), 1);
        assert_eq!(ledger.list_snapshots(Some("src_b"), 10).unwrap().len(), 1);
        assert_eq!(ledger.list_snapshots(None, 10).unwrap().len(), 2);
        assert_eq!(b.source_id, "src_b");
    }

    #[test]
    fn compute_changes_added_modified_removed_unchanged() {
        let (ledger, _dir) = temp_ledger();
        let from = ledger
            .commit_snapshot(
                &meta("src_a"),
                &items(&[("a", "alpha"), ("b", "beta"), ("c", "gamma")]),
                1000,
            )
            .unwrap();
        let to = ledger
            .commit_snapshot(
                &meta("src_a"),
                &items(&[("a", "alpha"), ("b", "beta v2"), ("d", "delta")]),
                2000,
            )
            .unwrap();

        let (changes, summary) = ledger
            .compute_changes(Some(&from.id), &to.id, "src_a", 3, false)
            .unwrap();
        assert_eq!(summary.added, 1, "d added");
        assert_eq!(summary.modified, 1, "b modified");
        assert_eq!(summary.removed, 1, "c removed");
        assert_eq!(summary.unchanged, 1, "a unchanged");

        let kind_of = |id: &str| {
            changes
                .iter()
                .find(|c| c.item_id == id)
                .map(|c| c.kind.clone())
        };
        assert_eq!(kind_of("d"), Some(ChangeKind::Added));
        assert_eq!(kind_of("b"), Some(ChangeKind::Modified));
        assert_eq!(kind_of("c"), Some(ChangeKind::Removed));
        assert_eq!(kind_of("a"), None);
    }

    #[test]
    fn compute_changes_from_none_marks_all_added() {
        let (ledger, _dir) = temp_ledger();
        let to = ledger
            .commit_snapshot(&meta("src_a"), &items(&[("a", "x")]), 1000)
            .unwrap();
        let (changes, summary) = ledger
            .compute_changes(None, &to.id, "src_a", 1, false)
            .unwrap();
        assert_eq!(summary.added, 1);
        assert_eq!(changes.len(), 1);
    }

    #[test]
    fn compute_changes_text_diff_only_when_requested() {
        let (ledger, _dir) = temp_ledger();
        let from = ledger
            .commit_snapshot(
                &meta("src_a"),
                &items(&[("a", "line one\nline two\n")]),
                1000,
            )
            .unwrap();
        let to = ledger
            .commit_snapshot(
                &meta("src_a"),
                &items(&[("a", "line one\nline TWO changed\n")]),
                2000,
            )
            .unwrap();

        let (without, _) = ledger
            .compute_changes(Some(&from.id), &to.id, "src_a", 1, false)
            .unwrap();
        assert!(without[0].text_diff.is_none());

        let (with, _) = ledger
            .compute_changes(Some(&from.id), &to.id, "src_a", 1, true)
            .unwrap();
        let td = with[0].text_diff.as_ref().expect("text diff present");
        assert!(td.contains("line TWO changed"), "got: {td}");
    }

    #[test]
    fn pathspec_does_not_leak_across_prefixed_sources() {
        let (ledger, _dir) = temp_ledger();
        ledger
            .commit_snapshot(&meta("src_a"), &items(&[("a", "x")]), 1000)
            .unwrap();
        // src_abc shares the "src_a" prefix; its items must not appear in
        // src_a's diff.
        let abc = ledger
            .commit_snapshot(&meta("src_abc"), &items(&[("z", "zeta")]), 2000)
            .unwrap();
        let a2 = ledger
            .commit_snapshot(&meta("src_a"), &items(&[("a", "x"), ("b", "y")]), 3000)
            .unwrap();

        let (changes, summary) = ledger
            .compute_changes(Some(&abc.id), &a2.id, "src_a", 2, false)
            .unwrap();
        assert_eq!(summary.added, 1, "only b is new in src_a");
        assert!(changes.iter().all(|c| c.item_id != "z"));
    }

    #[test]
    fn read_marker_set_and_get() {
        let (ledger, _dir) = temp_ledger();
        assert_eq!(ledger.get_read_marker("src_a").unwrap(), None);
        let snap = ledger
            .commit_snapshot(&meta("src_a"), &items(&[("a", "x")]), 1000)
            .unwrap();
        ledger.set_read_marker("src_a", &snap.id).unwrap();
        assert_eq!(
            ledger.get_read_marker("src_a").unwrap().as_deref(),
            Some(snap.id.as_str())
        );
        assert_eq!(ledger.get_read_marker("src_b").unwrap(), None);
    }

    #[test]
    fn checkpoint_round_trip() {
        let (ledger, _dir) = temp_ledger();
        let a = ledger
            .commit_snapshot(&meta("src_a"), &items(&[("a", "x")]), 1000)
            .unwrap();
        let b = ledger
            .commit_snapshot(&meta("src_b"), &items(&[("b", "y")]), 1000)
            .unwrap();
        ledger
            .create_checkpoint("ckpt_1", "baseline", &[a.id.clone(), b.id.clone()], 1500)
            .unwrap();

        let loaded = ledger.get_checkpoint("ckpt_1").unwrap().unwrap();
        assert_eq!(loaded.label, "baseline");
        assert_eq!(loaded.created_at_ms, 1500);
        assert_eq!(loaded.snapshot_ids.len(), 2);

        let all = ledger.list_checkpoints(10).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "ckpt_1");
    }

    #[test]
    fn cleanup_checkpoints_removes_old_tags() {
        let (ledger, _dir) = temp_ledger();
        ledger
            .commit_snapshot(&meta("src_a"), &items(&[("a", "x")]), 1000)
            .unwrap();
        ledger
            .create_checkpoint("ckpt_old", "old", &[], 100)
            .unwrap();
        ledger
            .create_checkpoint("ckpt_new", "new", &[], 5000)
            .unwrap();

        let deleted = ledger.cleanup_checkpoints(1000).unwrap();
        assert_eq!(deleted, 1);
        let remaining = ledger.list_checkpoints(10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "ckpt_new");
    }
}
