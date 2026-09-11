//! `PROFILE.md` markdown bridge — mirrors managed facet blocks into
//! `{workspace_dir}/PROFILE.md` so the agent prompt loader
//! (`agent/prompts/mod.rs::UserFilesSection`) picks them up on the next
//! turn.
//!
//! Ported verbatim from the deleted
//! `tinymemory_core::sync::composio::providers::profile_md` (tinymemory
//! v1.13.4 removed it along with the rest of the in-process Composio
//! pipeline). Nothing here ever depended on that pipeline — it is pure text
//! manipulation over a host-owned file and [`ProviderUserProfile`], which is
//! contract vocabulary (`tinymemory_api::composio`) rather than engine state
//! — so it moves to host code unchanged rather than needing a redesign.
//!
//! ## Block convention
//!
//! Each managed section lives between a pair of HTML comment markers:
//!
//! ```md
//! <!-- openhuman:<block_name>:start -->
//! ## <Section Heading>
//!
//! <body_markdown>
//!
//! <!-- openhuman:<block_name>:end -->
//! ```
//!
//! Anything outside the markers is left untouched, so user-authored prose
//! or hand-edited bullets are preserved across provider reconnects or
//! cache rebuilds.
//!
//! All operations are best-effort — errors are logged rather than
//! propagated, matching the PII-discipline pattern used elsewhere in this
//! domain.

use std::fs;
use std::io;
use std::path::Path;

use super::providers::ProviderUserProfile;

// ── Legacy connected-accounts constants (kept for internal helpers) ───────────

const CA_BLOCK: &str = "connected-accounts";
const CA_HEADING: &str = "## Connected Accounts";
const FILE_HEADER: &str = "# User Profile\n";

/// All managed block names, in the order they are appended when a new
/// `PROFILE.md` is created.
pub const BLOCKS: &[&str] = &[
    "connected-accounts", // written by provider path (merge_provider_into_profile_md)
    "style",
    "identity",
    "tooling",
    "vetoes",
    "goals",
];

// ── Public API ────────────────────────────────────────────────────────────────

/// Upsert the per-toolkit bullet for `profile` inside the managed
/// `connected-accounts` block of `{workspace_dir}/PROFILE.md`.
///
/// Creates the file with a `# User Profile` header if it does not exist.
/// Idempotent — re-connecting the same toolkit replaces the existing
/// bullet rather than duplicating it.
pub fn merge_provider_into_profile_md(
    workspace_dir: &Path,
    profile: &ProviderUserProfile,
) -> io::Result<()> {
    let toolkit = normalize_token(&profile.toolkit);
    if toolkit.is_empty() {
        return Ok(());
    }
    // Require a real connection_id so the bullet keys match what the
    // disconnect path (`composio_delete_connection`) will look up.
    let identifier = profile
        .connection_id
        .as_deref()
        .map(normalize_token)
        .filter(|v| !v.is_empty());
    let identifier = match identifier {
        Some(id) => id,
        None => {
            tracing::debug!(
                toolkit = %toolkit,
                "[composio:profile_md] skipping merge — connection_id missing or empty"
            );
            return Ok(());
        }
    };

    let bullet = match render_provider_bullet(&toolkit, &identifier, profile) {
        Some(b) => b,
        None => return Ok(()),
    };

    let path = workspace_dir.join("PROFILE.md");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let existing = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };

    let updated = upsert_provider_bullet(&existing, &toolkit, &identifier, &bullet);
    fs::write(&path, updated)?;
    tracing::debug!(
        target_file = "PROFILE.md",
        toolkit = %toolkit,
        identifier = %identifier,
        "[composio:profile_md] merged provider profile into PROFILE.md"
    );
    Ok(())
}

/// Remove the per-toolkit bullet for `(source, identifier)` from the
/// managed Connected Accounts block. If the block becomes empty the whole
/// block is dropped. Missing file or missing block are no-ops.
pub fn remove_provider_from_profile_md(
    workspace_dir: &Path,
    source: &str,
    identifier: &str,
) -> io::Result<()> {
    let path = workspace_dir.join("PROFILE.md");
    let existing = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let toolkit = normalize_token(source);
    let identifier = normalize_token(identifier);
    if toolkit.is_empty() || identifier.is_empty() {
        return Ok(());
    }
    let updated = remove_provider_bullet(&existing, &toolkit, &identifier);
    if updated != existing {
        fs::write(&path, updated)?;
        tracing::debug!(
            target_file = "PROFILE.md",
            toolkit = %toolkit,
            identifier = %identifier,
            "[composio:profile_md] removed provider bullet from PROFILE.md"
        );
    }
    Ok(())
}

/// Upsert a generic managed block.
///
/// * `block_name` — one of [`BLOCKS`] (e.g. `"style"`, `"identity"`).
/// * `section_heading` — heading rendered inside the block (e.g. `"## Style"`).
/// * `body_markdown` — pre-rendered content (bullets, prose). Must not
///   contain the block markers themselves.
///
/// Creates `PROFILE.md` if it does not exist. If the block is absent it is
/// appended at the end of the file. If the block exists its body is replaced
/// in-place — content outside the markers is left byte-for-byte untouched.
///
/// An empty `body_markdown` renders a `*(no entries yet)*` placeholder
/// instead of deleting the block; this preserves the block's position for the
/// next write.
///
/// Idempotent: calling with the same inputs twice produces the same file.
pub fn replace_managed_block(
    workspace_dir: &Path,
    block_name: &str,
    section_heading: &str,
    body_markdown: String,
) -> io::Result<()> {
    let path = workspace_dir.join("PROFILE.md");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let existing = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };

    let updated = upsert_block(&existing, block_name, section_heading, &body_markdown);
    fs::write(&path, updated)?;
    tracing::debug!(
        block_name = %block_name,
        "[composio:profile_md] replaced managed block '{}' in PROFILE.md",
        block_name
    );
    Ok(())
}

// ── Connected-accounts internals ──────────────────────────────────────────────

fn render_provider_bullet(
    toolkit: &str,
    identifier: &str,
    profile: &ProviderUserProfile,
) -> Option<String> {
    let mut fields: Vec<String> = Vec::new();
    if let Some(v) = profile.display_name.as_deref().map(sanitize) {
        if !v.is_empty() {
            fields.push(v);
        }
    }
    if let Some(v) = profile.email.as_deref().map(sanitize) {
        if !v.is_empty() {
            fields.push(v);
        }
    }
    if let Some(v) = profile.username.as_deref().map(sanitize) {
        if !v.is_empty() {
            fields.push(format!("@{v}"));
        }
    }
    if let Some(v) = profile.profile_url.as_deref().map(sanitize) {
        if !v.is_empty() {
            fields.push(v);
        }
    }
    if fields.is_empty() {
        return None;
    }
    let marker = bullet_marker(toolkit, identifier);
    Some(format!(
        "- {marker} **{title}** ({identifier}): {fields}",
        title = title_case(toolkit),
        identifier = identifier,
        fields = fields.join(" | ")
    ))
}

fn bullet_marker(toolkit: &str, identifier: &str) -> String {
    format!("<!-- acct:{toolkit}:{identifier} -->")
}

/// Insert or replace `bullet` inside the connected-accounts managed block.
fn upsert_provider_bullet(existing: &str, toolkit: &str, identifier: &str, bullet: &str) -> String {
    let marker = bullet_marker(toolkit, identifier);
    let start_tag = block_start(CA_BLOCK);
    let end_tag = block_end(CA_BLOCK);
    let (prefix, block_body, suffix) = split_any_block(existing, &start_tag, &end_tag);

    let mut lines: Vec<String> = block_body
        .lines()
        .filter(|l| !l.contains(&marker))
        .map(|l| l.to_string())
        .collect();
    lines.push(bullet.to_string());

    let mut bullets = lines
        .into_iter()
        .filter(|l| l.trim_start().starts_with("- <!-- acct:"))
        .collect::<Vec<_>>();
    bullets.sort();

    let block = format!(
        "{start_tag}\n{CA_HEADING}\n\n{body}\n{end_tag}",
        body = bullets.join("\n")
    );

    assemble(&prefix, &block, &suffix)
}

/// Remove the bullet matching `(toolkit, identifier)` from the connected-
/// accounts managed block. Drops the block entirely if no bullets remain.
fn remove_provider_bullet(existing: &str, toolkit: &str, identifier: &str) -> String {
    let marker = bullet_marker(toolkit, identifier);
    let start_tag = block_start(CA_BLOCK);
    let end_tag = block_end(CA_BLOCK);
    let (prefix, block_body, suffix) = split_any_block(existing, &start_tag, &end_tag);
    if block_body.is_empty() && prefix == existing {
        return existing.to_string();
    }
    let bullets: Vec<String> = block_body
        .lines()
        .filter(|l| l.trim_start().starts_with("- <!-- acct:") && !l.contains(&marker))
        .map(|l| l.to_string())
        .collect();
    if bullets.is_empty() {
        return assemble(&prefix, "", &suffix);
    }
    let block = format!(
        "{start_tag}\n{CA_HEADING}\n\n{body}\n{end_tag}",
        body = bullets.join("\n")
    );
    assemble(&prefix, &block, &suffix)
}

// ── Generic managed-block helpers ─────────────────────────────────────────────

/// Build the start marker for `block_name`.
pub fn block_start(block_name: &str) -> String {
    format!("<!-- openhuman:{block_name}:start -->")
}

/// Build the end marker for `block_name`.
pub fn block_end(block_name: &str) -> String {
    format!("<!-- openhuman:{block_name}:end -->")
}

/// Insert or replace a generic managed block in `existing`.
///
/// If the block is absent it is appended. If it exists its body (between the
/// markers) is replaced. Content outside the markers is returned unchanged.
fn upsert_block(
    existing: &str,
    block_name: &str,
    section_heading: &str,
    body_markdown: &str,
) -> String {
    let start_tag = block_start(block_name);
    let end_tag = block_end(block_name);

    let body = if body_markdown.trim().is_empty() {
        "*(no entries yet)*".to_string()
    } else {
        body_markdown.to_string()
    };

    let block = format!("{start_tag}\n{section_heading}\n\n{body}\n\n{end_tag}");

    let (prefix, _old_body, suffix) = split_any_block(existing, &start_tag, &end_tag);

    if prefix == existing {
        // Block was absent — append.
        assemble(existing, &block, "")
    } else {
        assemble(&prefix, &block, &suffix)
    }
}

/// Split `existing` around the markers `[start_tag, end_tag]`.
///
/// Returns `(prefix, block_body, suffix)`.  If no block is present,
/// `prefix` is the full string and `block_body` / `suffix` are empty.
/// `block_body` is the content *between* the markers (excluding the
/// markers themselves).
fn split_any_block(existing: &str, start_tag: &str, end_tag: &str) -> (String, String, String) {
    if let (Some(start), Some(end)) = (existing.find(start_tag), existing.find(end_tag)) {
        if end > start {
            let prefix = existing[..start].to_string();
            let body = existing[start + start_tag.len()..end].to_string();
            let suffix_start = end + end_tag.len();
            let suffix = existing[suffix_start..].to_string();
            return (prefix, body, suffix);
        }
    }
    (existing.to_string(), String::new(), String::new())
}

/// Assemble `prefix + block + suffix`, normalising the newlines immediately
/// adjacent to the managed block while leaving the user's bytes elsewhere
/// untouched.
fn assemble(prefix: &str, block: &str, suffix: &str) -> String {
    if block.is_empty() {
        // Removing the block entirely.
        let p = prefix.trim_end_matches('\n');
        let s = suffix.trim_start_matches('\n');
        let mut out = String::with_capacity(p.len() + s.len() + 2);
        out.push_str(p);
        if !p.is_empty() {
            out.push('\n');
            if !s.is_empty() {
                out.push('\n');
            }
        }
        out.push_str(s);
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        return out;
    }

    let mut out = String::new();
    if prefix.trim().is_empty() {
        // Seed with a header on first creation.
        out.push_str(FILE_HEADER);
        out.push('\n');
    } else {
        let p = prefix.trim_end_matches('\n');
        out.push_str(p);
        out.push_str("\n\n");
    }
    out.push_str(block);
    if suffix.is_empty() {
        out.push('\n');
    } else {
        let s = suffix.trim_start_matches('\n');
        if s.is_empty() {
            // Suffix was only whitespace — end with single newline, no blank line.
            out.push('\n');
        } else {
            out.push_str("\n\n");
            out.push_str(s);
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out
}

// ── Token / string helpers ────────────────────────────────────────────────────

fn normalize_token(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() || lower == '-' || lower == '_' {
            out.push(lower);
        } else {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

fn title_case(raw: &str) -> String {
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn sanitize(raw: &str) -> String {
    let replaced = raw.replace(['\n', '\r', '\t'], " ").replace('|', "/");
    replaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "profile_md_tests.rs"]
mod tests;
