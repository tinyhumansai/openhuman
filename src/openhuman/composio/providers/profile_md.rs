//! `PROFILE.md` markdown bridge — mirrors the per-toolkit identity
//! fragments we already persist into the `user_profile` facet table
//! into a managed block inside `{workspace_dir}/PROFILE.md` so the
//! agent prompt loader (`agent/prompts/mod.rs::UserFilesSection`)
//! picks them up on the next turn.
//!
//! The block lives between the markers
//!
//! ```md
//! <!-- openhuman:connected-accounts:start -->
//! ...
//! <!-- openhuman:connected-accounts:end -->
//! ```
//!
//! Anything outside the markers is left untouched, so a profile authored
//! by the LinkedIn onboarding pipeline or hand-edited by the user is
//! preserved across reconnects.
//!
//! All operations are best-effort and log on failure rather than
//! propagating, matching the existing PII-discipline pattern in
//! `on_connection_created`.

use super::ProviderUserProfile;
use std::fs;
use std::io;
use std::path::Path;

const BLOCK_START: &str = "<!-- openhuman:connected-accounts:start -->";
const BLOCK_END: &str = "<!-- openhuman:connected-accounts:end -->";
const SECTION_HEADING: &str = "## Connected Accounts";
const FILE_HEADER: &str = "# User Profile\n";

/// Upsert the per-toolkit bullet for `profile` inside the managed
/// Connected Accounts block of `{workspace_dir}/PROFILE.md`.
///
/// Creates the file with a `# User Profile` header if it does not
/// exist. Idempotent — re-connecting the same toolkit replaces the
/// existing bullet rather than duplicating it.
pub fn merge_provider_into_profile_md(
    workspace_dir: &Path,
    profile: &ProviderUserProfile,
) -> io::Result<()> {
    let toolkit = normalize_token(&profile.toolkit);
    if toolkit.is_empty() {
        return Ok(());
    }
    let identifier = profile
        .connection_id
        .as_deref()
        .map(normalize_token)
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "default".to_string());

    let bullet = match render_bullet(&toolkit, &identifier, profile) {
        Some(b) => b,
        // No non-empty fields — nothing worth writing.
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

    let updated = upsert_bullet(&existing, &toolkit, &identifier, &bullet);
    fs::write(&path, updated)?;
    tracing::debug!(
        path = %path.display(),
        toolkit = %toolkit,
        identifier = %identifier,
        "[composio:profile_md] merged provider profile into PROFILE.md"
    );
    Ok(())
}

/// Remove the per-toolkit bullet for `(source, identifier)` from the
/// managed Connected Accounts block. If the block becomes empty as a
/// result, the whole block is dropped. Missing file or missing block
/// are no-ops.
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
    let updated = remove_bullet(&existing, &toolkit, &identifier);
    if updated != existing {
        fs::write(&path, updated)?;
        tracing::debug!(
            path = %path.display(),
            toolkit = %toolkit,
            identifier = %identifier,
            "[composio:profile_md] removed provider bullet from PROFILE.md"
        );
    }
    Ok(())
}

// ── Internals ────────────────────────────────────────────────────────

/// Build the markdown bullet for one provider connection. Returns
/// `None` if the profile carries no usable fields.
fn render_bullet(toolkit: &str, identifier: &str, profile: &ProviderUserProfile) -> Option<String> {
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
    // Stable per-(toolkit,identifier) marker so we can locate this
    // bullet on later upserts even if the rendered text changes.
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

/// Insert or replace `bullet` inside the managed block.
fn upsert_bullet(existing: &str, toolkit: &str, identifier: &str, bullet: &str) -> String {
    let marker = bullet_marker(toolkit, identifier);
    let (prefix, block_body, suffix) = split_block(existing);

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
        "{BLOCK_START}\n{SECTION_HEADING}\n\n{body}\n{BLOCK_END}",
        body = bullets.join("\n")
    );

    assemble(&prefix, &block, &suffix)
}

/// Remove the bullet matching `(toolkit, identifier)` from the managed
/// block. Drops the block entirely if no bullets remain.
fn remove_bullet(existing: &str, toolkit: &str, identifier: &str) -> String {
    let marker = bullet_marker(toolkit, identifier);
    let (prefix, block_body, suffix) = split_block(existing);
    if block_body.is_empty() && prefix == existing {
        // No managed block present.
        return existing.to_string();
    }
    let bullets: Vec<String> = block_body
        .lines()
        .filter(|l| l.trim_start().starts_with("- <!-- acct:") && !l.contains(&marker))
        .map(|l| l.to_string())
        .collect();
    if bullets.is_empty() {
        // Drop the entire block.
        return assemble(&prefix, "", &suffix);
    }
    let block = format!(
        "{BLOCK_START}\n{SECTION_HEADING}\n\n{body}\n{BLOCK_END}",
        body = bullets.join("\n")
    );
    assemble(&prefix, &block, &suffix)
}

/// Split the file into `(prefix, block_body, suffix)` around the
/// managed block. If no block is present, `prefix` is the full file
/// (with a trailing newline ensured for clean assembly) and
/// `block_body` is empty.
fn split_block(existing: &str) -> (String, String, String) {
    if let (Some(start), Some(end)) = (existing.find(BLOCK_START), existing.find(BLOCK_END)) {
        if end > start {
            let prefix = existing[..start].trim_end().to_string();
            let body = existing[start + BLOCK_START.len()..end].to_string();
            let suffix_start = end + BLOCK_END.len();
            let suffix = existing[suffix_start..]
                .trim_start_matches('\n')
                .to_string();
            return (prefix, body, suffix);
        }
    }
    let prefix = if existing.is_empty() {
        FILE_HEADER.to_string()
    } else if existing.ends_with('\n') {
        existing.to_string()
    } else {
        format!("{existing}\n")
    };
    (prefix, String::new(), String::new())
}

fn assemble(prefix: &str, block: &str, suffix: &str) -> String {
    let prefix_trim = prefix.trim_end();
    let suffix_trim = suffix.trim();
    if block.is_empty() {
        let mut out = String::new();
        if !prefix_trim.is_empty() {
            out.push_str(prefix_trim);
            out.push('\n');
        }
        if !suffix_trim.is_empty() {
            out.push('\n');
            out.push_str(suffix_trim);
            out.push('\n');
        }
        return out;
    }
    let mut out = String::new();
    if prefix_trim.is_empty() {
        out.push_str(FILE_HEADER);
    } else {
        out.push_str(prefix_trim);
        out.push('\n');
    }
    out.push('\n');
    out.push_str(block);
    out.push('\n');
    if !suffix_trim.is_empty() {
        out.push('\n');
        out.push_str(suffix_trim);
        out.push('\n');
    }
    out
}

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
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample(toolkit: &str, conn: &str) -> ProviderUserProfile {
        ProviderUserProfile {
            toolkit: toolkit.into(),
            connection_id: Some(conn.into()),
            display_name: Some("Jane Doe".into()),
            email: Some("jane@example.com".into()),
            username: Some("janedoe".into()),
            avatar_url: None,
            profile_url: Some("https://example.com/jane".into()),
            extras: serde_json::Value::Null,
        }
    }

    #[test]
    fn creates_file_when_missing() {
        let tmp = TempDir::new().unwrap();
        merge_provider_into_profile_md(tmp.path(), &sample("gmail", "c-1")).unwrap();
        let body = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
        assert!(body.starts_with("# User Profile"), "body was:\n{body}");
        assert!(body.contains(BLOCK_START));
        assert!(body.contains(SECTION_HEADING));
        assert!(body.contains("**Gmail** (c-1):"));
        assert!(body.contains("jane@example.com"));
        assert!(body.contains("@janedoe"));
        assert!(body.contains(BLOCK_END));
    }

    #[test]
    fn upsert_is_idempotent_for_same_toolkit_connection() {
        let tmp = TempDir::new().unwrap();
        let mut p = sample("gmail", "c-1");
        merge_provider_into_profile_md(tmp.path(), &p).unwrap();
        p.display_name = Some("Jane D.".into());
        merge_provider_into_profile_md(tmp.path(), &p).unwrap();
        let body = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
        let occurrences = body.matches("acct:gmail:c-1").count();
        assert_eq!(occurrences, 1, "duplicate bullet:\n{body}");
        assert!(body.contains("Jane D."));
        assert!(!body.contains("Jane Doe"));
    }

    #[test]
    fn multiple_toolkits_render_separate_bullets() {
        let tmp = TempDir::new().unwrap();
        merge_provider_into_profile_md(tmp.path(), &sample("gmail", "c-1")).unwrap();
        merge_provider_into_profile_md(tmp.path(), &sample("twitter", "c-2")).unwrap();
        let body = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
        assert!(body.contains("acct:gmail:c-1"));
        assert!(body.contains("acct:twitter:c-2"));
        assert_eq!(body.matches(BLOCK_START).count(), 1);
        assert_eq!(body.matches(BLOCK_END).count(), 1);
    }

    #[test]
    fn preserves_user_authored_content_outside_block() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("PROFILE.md");
        fs::write(
            &path,
            "# User Profile\n\nSome bio paragraph from LinkedIn.\n\n## Key facts\n- a\n- b\n",
        )
        .unwrap();
        merge_provider_into_profile_md(tmp.path(), &sample("gmail", "c-1")).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("Some bio paragraph from LinkedIn."));
        assert!(body.contains("## Key facts"));
        assert!(body.contains("- a"));
        assert!(body.contains("acct:gmail:c-1"));
    }

    #[test]
    fn skips_when_no_useful_fields() {
        let tmp = TempDir::new().unwrap();
        let p = ProviderUserProfile {
            toolkit: "gmail".into(),
            connection_id: Some("c-1".into()),
            display_name: Some("   ".into()),
            email: None,
            username: Some("".into()),
            avatar_url: None,
            profile_url: None,
            extras: serde_json::Value::Null,
        };
        merge_provider_into_profile_md(tmp.path(), &p).unwrap();
        assert!(!tmp.path().join("PROFILE.md").exists());
    }

    #[test]
    fn remove_drops_specific_bullet() {
        let tmp = TempDir::new().unwrap();
        merge_provider_into_profile_md(tmp.path(), &sample("gmail", "c-1")).unwrap();
        merge_provider_into_profile_md(tmp.path(), &sample("twitter", "c-2")).unwrap();
        remove_provider_from_profile_md(tmp.path(), "gmail", "c-1").unwrap();
        let body = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
        assert!(!body.contains("acct:gmail:c-1"));
        assert!(body.contains("acct:twitter:c-2"));
    }

    #[test]
    fn remove_drops_block_when_empty() {
        let tmp = TempDir::new().unwrap();
        merge_provider_into_profile_md(tmp.path(), &sample("gmail", "c-1")).unwrap();
        remove_provider_from_profile_md(tmp.path(), "gmail", "c-1").unwrap();
        let body = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
        assert!(!body.contains(BLOCK_START), "block remained:\n{body}");
        assert!(!body.contains(BLOCK_END));
        assert!(body.starts_with("# User Profile"));
    }

    #[test]
    fn remove_is_noop_when_file_missing() {
        let tmp = TempDir::new().unwrap();
        remove_provider_from_profile_md(tmp.path(), "gmail", "c-1").unwrap();
        assert!(!tmp.path().join("PROFILE.md").exists());
    }

    #[test]
    fn sanitize_strips_pipes_and_newlines() {
        assert_eq!(sanitize("foo\nbar"), "foo bar");
        assert_eq!(sanitize("a | b"), "a / b");
        assert_eq!(sanitize("  multi   space  "), "multi space");
    }
}
