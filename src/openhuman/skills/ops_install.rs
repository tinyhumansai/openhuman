//! URL-based skill installation: fetch, validate, and write SKILL.md from a remote URL.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::ops_discover::{discover_skills_inner, is_workspace_trusted};
use super::ops_parse::parse_skill_md_str;
use super::ops_types::{SkillFrontmatter, SkillScope, MAX_NAME_LEN, SKILL_MD};

/// Default wall-clock budget for the SKILL.md fetch.
pub const DEFAULT_INSTALL_TIMEOUT_SECS: u64 = 60;
/// Hard ceiling callers can request via `timeout_secs`.
pub const MAX_INSTALL_TIMEOUT_SECS: u64 = 600;
/// Upper bound on raw URL length accepted by [`validate_install_url`].
pub const MAX_INSTALL_URL_LEN: usize = 2048;
/// Upper bound on the fetched SKILL.md body. Single-file skills rarely exceed
/// a few KB; the 1 MiB cap here is a defensive limit against a hostile or
/// misconfigured host streaming an unbounded response into memory.
pub const MAX_SKILL_MD_BYTES: usize = 1024 * 1024;

/// Input for [`install_skill_from_url`]. Mirrors the `skills.install_from_url`
/// JSON-RPC payload.
#[derive(Debug, Clone, Deserialize)]
pub struct InstallSkillFromUrlParams {
    /// Remote SKILL.md URL. Must be `https://`, resolve to a non-private host
    /// (see [`validate_install_url`]), and point at a `.md` file after
    /// github.com `/blob/` normalization.
    pub url: String,
    /// Optional wall-clock budget override, in seconds. Defaults to
    /// [`DEFAULT_INSTALL_TIMEOUT_SECS`] and is capped at
    /// [`MAX_INSTALL_TIMEOUT_SECS`].
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

/// Outcome of a successful install. `new_skills` is the set of skill slugs
/// that appeared in the catalog since the start of the call (post-discovery
/// minus pre-discovery).
#[derive(Debug, Clone, Serialize)]
pub struct InstallSkillFromUrlOutcome {
    /// The URL the caller submitted, trimmed.
    pub url: String,
    /// Human-readable install log — typically `Fetched N bytes from <url>\n
    /// Installed to <path>`. Repurposed from the old npx stdout field so the
    /// UI success panel keeps the same `<details>` layout.
    pub stdout: String,
    /// Non-fatal warnings surfaced during parse (e.g. deprecated top-level
    /// `version`/`author`/`tags`). Empty on the happy path. Repurposed from
    /// the old npx stderr field.
    pub stderr: String,
    /// Slugs that appeared in the workspace skill catalog as a result of the
    /// install. Usually one, empty only when the SKILL.md could not be
    /// enumerated by discovery (rare — indicates workspace trust mismatch).
    pub new_skills: Vec<String>,
}

/// Install a skill by fetching its `SKILL.md` directly over HTTPS and writing
/// it to `<workspace>/.openhuman/skills/<slug>/SKILL.md`.
///
/// Design rationale: openhuman's skill discovery scans
/// `<workspace>/.openhuman/skills/` (plus `~/.openhuman/skills/` and legacy
/// paths), **not** the per-agent subdirectories that the vercel-labs `skills`
/// CLI writes to (`./claude-code/skills/`, `./cursor/skills/`, …). The CLI's
/// agent ecosystem is incompatible with openhuman's skill layout, so we fetch
/// the SKILL.md file directly and install it into a layout discovery sees.
///
/// Validation applied before any network I/O:
/// * URL length, scheme (`https` only), and host safety via
///   [`validate_install_url`] — rejects loopback, private, link-local,
///   multicast, shared-address ranges, `localhost`, and `.local` / `.localhost`
///   mDNS-style hostnames.
/// * `github.com/<o>/<r>/blob/<b>/<p>` is rewritten to the raw
///   `raw.githubusercontent.com/<o>/<r>/<b>/<p>` equivalent so humans can
///   paste the URL they see in the browser.
/// * The path must end in `.md` (case-insensitive). Repo/tree URLs and
///   tarballs are rejected with `unsupported url form:`.
/// * `timeout_secs` is clamped to [`MAX_INSTALL_TIMEOUT_SECS`].
///
/// Runtime:
/// * Body size is capped by [`MAX_SKILL_MD_BYTES`] (1 MiB). The advertised
///   `Content-Length` is checked up front; the buffered body length is
///   checked again after the download as defense against a lying header.
/// * Frontmatter is validated — `name` and `description` are required per
///   the agentskills.io spec.
/// * The slug is derived from `metadata.id` when present, otherwise the
///   sanitized `name` field. Collision with an existing directory is fatal
///   (no silent overwrite).
/// * Write is atomic: `SKILL.md.tmp` in the target dir, then `rename` on
///   success.
///
/// On success the full post-install skills catalog is re-discovered and the
/// outcome includes the list of skill slugs that appeared since the start of
/// the call.
pub async fn install_skill_from_url(
    workspace_dir: &Path,
    params: InstallSkillFromUrlParams,
) -> Result<InstallSkillFromUrlOutcome, String> {
    let raw_url = params.url.trim().to_string();
    validate_install_url(&raw_url)?;

    let timeout_secs = params
        .timeout_secs
        .unwrap_or(DEFAULT_INSTALL_TIMEOUT_SECS)
        .clamp(1, MAX_INSTALL_TIMEOUT_SECS);

    let fetch_url = normalize_install_url(&raw_url)?;

    // Second-layer SSRF guard: a public-looking hostname can still resolve
    // to a loopback / private / link-local address (DNS-to-private-IP). We
    // resolve the host up-front and reject if any returned IP is private.
    // Known caveat: this does not fully prevent DNS rebinding — reqwest's
    // resolver may see different answers than ours. Closing that gap requires
    // pinning a `SocketAddr` and passing it to reqwest via a custom resolver,
    // tracked separately.
    validate_resolved_host(&fetch_url).await?;

    tracing::debug!(
        raw_url = %raw_url,
        fetch_url = %fetch_url,
        workspace = %workspace_dir.display(),
        timeout_secs = timeout_secs,
        "[skills] install_skill_from_url: entry"
    );

    let home = dirs::home_dir();
    let trusted_before = is_workspace_trusted(workspace_dir);
    let before: std::collections::HashSet<String> =
        discover_skills_inner(home.as_deref(), Some(workspace_dir), trusted_before)
            .into_iter()
            .map(|s| s.name)
            .collect();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("fetch failed: build http client: {e}"))?;

    tracing::info!(
        fetch_url = %fetch_url,
        "[skills] install_skill_from_url: fetching SKILL.md"
    );

    let response = match client.get(&fetch_url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            if e.is_timeout() {
                return Err(format!("fetch timed out after {timeout_secs}s"));
            }
            return Err(format!("fetch failed: {e}"));
        }
    };

    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "fetch failed: {fetch_url} returned status {}",
            status.as_u16()
        ));
    }

    if let Some(len) = response.content_length() {
        if len > MAX_SKILL_MD_BYTES as u64 {
            return Err(format!(
                "fetch too large: {} bytes exceeds {MAX_SKILL_MD_BYTES} limit",
                len
            ));
        }
    }

    let bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            if e.is_timeout() {
                return Err(format!("fetch timed out after {timeout_secs}s"));
            }
            return Err(format!("fetch failed: reading body: {e}"));
        }
    };

    if bytes.len() > MAX_SKILL_MD_BYTES {
        return Err(format!(
            "fetch too large: {} bytes exceeds {MAX_SKILL_MD_BYTES} limit",
            bytes.len()
        ));
    }

    let content = String::from_utf8(bytes.to_vec())
        .map_err(|e| format!("invalid SKILL.md: body is not valid utf-8: {e}"))?;

    let (frontmatter, _body, parse_warnings) = parse_skill_md_str(&content).ok_or_else(|| {
        "invalid SKILL.md: frontmatter block opened with `---` but never terminated".to_string()
    })?;

    if frontmatter.name.trim().is_empty() {
        return Err("invalid SKILL.md: missing required field 'name'".to_string());
    }
    if frontmatter.description.trim().is_empty() {
        return Err("invalid SKILL.md: missing required field 'description'".to_string());
    }

    let slug = derive_install_slug(&frontmatter)?;

    // Install to user scope (`~/.openhuman/skills/<slug>`), which `discover_skills`
    // scans unconditionally. Project scope (`<ws>/.openhuman/skills/`) is gated on
    // a `<ws>/.openhuman/trust` marker and would render the install invisible to the
    // skills list until the user opts the workspace into trust.
    let skills_root = home
        .as_deref()
        .ok_or_else(|| "write failed: unable to resolve home directory".to_string())?
        .join(".openhuman")
        .join("skills");
    let target_dir = skills_root.join(&slug);
    if target_dir.exists() {
        return Err(format!(
            "skill already installed as {slug:?} at {}",
            target_dir.display()
        ));
    }

    std::fs::create_dir_all(&target_dir).map_err(|e| {
        format!(
            "write failed: create directory {}: {e}",
            target_dir.display()
        )
    })?;

    let target_file = target_dir.join(SKILL_MD);
    let temp_file = target_dir.join("SKILL.md.tmp");

    // Roll the partial install back if either filesystem op fails so the
    // next retry isn't blocked by a leftover empty directory. Cleanup is
    // best-effort — if it fails, we surface the original write error.
    let write_result: Result<(), String> = std::fs::write(&temp_file, &content)
        .map_err(|e| format!("write failed: {}: {e}", temp_file.display()))
        .and_then(|_| {
            std::fs::rename(&temp_file, &target_file)
                .map_err(|e| format!("write failed: rename {}: {e}", target_file.display()))
        });

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&temp_file);
        if let Err(rm_err) = std::fs::remove_dir(&target_dir) {
            tracing::warn!(
                target_dir = %target_dir.display(),
                error = %rm_err,
                "[skills] install_skill_from_url: rollback remove_dir failed (non-fatal)"
            );
        } else {
            tracing::warn!(
                target_dir = %target_dir.display(),
                "[skills] install_skill_from_url: rolled back partial install after write failure"
            );
        }
        return Err(e);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o644);
        if let Err(e) = std::fs::set_permissions(&target_file, perms) {
            tracing::warn!(
                target = %target_file.display(),
                error = %e,
                "[skills] install_skill_from_url: chmod 0644 failed (non-fatal)"
            );
        }
    }

    let trusted_after = is_workspace_trusted(workspace_dir);
    let after = discover_skills_inner(home.as_deref(), Some(workspace_dir), trusted_after);
    let new_skills: Vec<String> = after
        .into_iter()
        .map(|s| s.name)
        .filter(|name| !before.contains(name))
        .collect();

    tracing::info!(
        raw_url = %raw_url,
        fetch_url = %fetch_url,
        slug = %slug,
        bytes = content.len(),
        new_count = new_skills.len(),
        "[skills] install_skill_from_url: completed"
    );

    let stdout = format!(
        "Fetched {} bytes from {fetch_url}\nInstalled to {}",
        content.len(),
        target_file.display()
    );
    let stderr = parse_warnings.join("\n");

    Ok(InstallSkillFromUrlOutcome {
        url: raw_url,
        stdout,
        stderr,
        new_skills,
    })
}

/// Input for [`uninstall_skill`]. Mirrors the `skills.uninstall` JSON-RPC payload.
#[derive(Debug, Clone, Deserialize)]
pub struct UninstallSkillParams {
    /// On-disk slug of the installed skill — the directory name under
    /// `~/.openhuman/skills/<slug>/`. Retained as `name` for wire-format
    /// back-compat with pre-existing clients; semantics are slug-only.
    pub name: String,
}

/// Outcome of a successful uninstall.
#[derive(Debug, Clone, Serialize)]
pub struct UninstallSkillOutcome {
    /// The normalised slug that was removed.
    pub name: String,
    /// Absolute on-disk path that was deleted (post-canonicalisation).
    pub removed_path: String,
    /// Scope the uninstall applied to. Always `User` today.
    pub scope: SkillScope,
}

/// Remove an installed user-scope SKILL.md skill from `~/.openhuman/skills/`.
///
/// Only user-scope uninstalls are supported. Resolution is defensive:
/// canonicalises paths, refuses symlinks, requires SKILL.md to be present.
///
/// `home_dir_override` is for tests; production callers pass `None`.
pub fn uninstall_skill(
    params: UninstallSkillParams,
    home_dir_override: Option<&Path>,
) -> Result<UninstallSkillOutcome, String> {
    let trimmed = params.name.trim().to_string();
    if trimmed.is_empty() {
        return Err("skill name is required".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        log::warn!(
            "[skills] uninstall_skill: rejected name with path separators name={:?}",
            trimmed
        );
        return Err(format!(
            "skill name '{trimmed}' must not contain path separators"
        ));
    }
    if trimmed.len() > MAX_NAME_LEN {
        return Err(format!(
            "skill name is {} chars (max {MAX_NAME_LEN})",
            trimmed.len()
        ));
    }

    let home = match home_dir_override
        .map(|p| p.to_path_buf())
        .or_else(dirs::home_dir)
    {
        Some(h) => h,
        None => return Err("could not resolve user home directory".to_string()),
    };

    let skills_root = home.join(".openhuman").join("skills");
    if !skills_root.exists() {
        return Err(format!(
            "no user skills directory at {}",
            skills_root.display()
        ));
    }

    let root_meta = std::fs::symlink_metadata(&skills_root)
        .map_err(|e| format!("stat {} failed: {e}", skills_root.display()))?;
    if root_meta.file_type().is_symlink() {
        log::warn!(
            "[skills] uninstall_skill: refused symlinked skills root path={}",
            skills_root.display()
        );
        return Err(format!(
            "skills root {} is a symlink — refusing to resolve",
            skills_root.display()
        ));
    }

    let canonical_root = std::fs::canonicalize(&skills_root)
        .map_err(|e| format!("canonicalize {} failed: {e}", skills_root.display()))?;

    let candidate = skills_root.join(&trimmed);
    match std::fs::symlink_metadata(&candidate) {
        Ok(m) if m.file_type().is_symlink() => {
            log::warn!(
                "[skills] uninstall_skill: refused symlinked alias name={trimmed} path={}",
                candidate.display()
            );
            return Err(format!(
                "skill '{trimmed}' is a symlinked alias — refusing to resolve"
            ));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!("skill '{trimmed}' is not installed"));
        }
        Err(e) => {
            return Err(format!("stat {} failed: {e}", candidate.display()));
        }
    }

    let canonical_candidate = std::fs::canonicalize(&candidate).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("skill '{trimmed}' is not installed")
        } else {
            format!("canonicalize {} failed: {e}", candidate.display())
        }
    })?;

    if !canonical_candidate.starts_with(&canonical_root) {
        log::warn!(
            "[skills] uninstall_skill: path escape rejected candidate={} root={}",
            canonical_candidate.display(),
            canonical_root.display()
        );
        return Err(format!(
            "refused to remove {} — path escapes skills root",
            canonical_candidate.display()
        ));
    }

    let meta = std::fs::symlink_metadata(&canonical_candidate)
        .map_err(|e| format!("stat {} failed: {e}", canonical_candidate.display()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!(
            "{} is not a directory — refusing to remove",
            canonical_candidate.display()
        ));
    }

    let skill_md = canonical_candidate.join(SKILL_MD);
    if !skill_md.exists() {
        return Err(format!(
            "{} does not look like a SKILL.md skill (missing {SKILL_MD})",
            canonical_candidate.display()
        ));
    }

    log::info!(
        "[skills] uninstall_skill: removing name={trimmed} path={}",
        canonical_candidate.display()
    );
    std::fs::remove_dir_all(&canonical_candidate)
        .map_err(|e| format!("remove {} failed: {e}", canonical_candidate.display()))?;

    Ok(UninstallSkillOutcome {
        name: trimmed,
        removed_path: canonical_candidate.display().to_string(),
        scope: SkillScope::User,
    })
}

/// Rewrite `github.com/<o>/<r>/blob/<branch>/<path>` into its raw counterpart
/// so a URL copied from a browser's GitHub page resolves to the file body
/// instead of the HTML wrapper. Any other host is returned unchanged.
///
/// Also enforces that the final path ends in `.md` (case-insensitive). Tree,
/// commit, and whole-repo URLs are rejected here — they require a
/// fundamentally different install path (recursive fetch / tarball) that is
/// out of scope for single-file SKILL.md installs.
pub(crate) fn normalize_install_url(raw: &str) -> Result<String, String> {
    let parsed =
        url::Url::parse(raw).map_err(|e| format!("unsupported url form: parse {raw:?}: {e}"))?;
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();

    let normalized = if host == "github.com" {
        let segments: Vec<&str> = parsed
            .path_segments()
            .map(|it| it.collect())
            .unwrap_or_default();
        if segments.len() >= 5 && segments[2] == "blob" {
            let owner = segments[0];
            let repo = segments[1];
            let branch = segments[3];
            let rest = segments[4..].join("/");
            format!("https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{rest}")
        } else if segments.len() >= 3 && (segments[2] == "tree" || segments[2] == "raw") {
            return Err(format!(
                "unsupported url form: only direct SKILL.md links are supported, got {raw:?} (tree/dir URLs are not yet supported)"
            ));
        } else if segments.len() <= 2 {
            return Err(format!(
                "unsupported url form: only direct SKILL.md links are supported, got {raw:?} (whole-repo URLs are not yet supported)"
            ));
        } else {
            raw.to_string()
        }
    } else {
        raw.to_string()
    };

    let check = url::Url::parse(&normalized)
        .map_err(|e| format!("unsupported url form: parse normalized {normalized:?}: {e}"))?;
    let path_lower = check.path().to_ascii_lowercase();
    if !path_lower.ends_with(".md") {
        return Err(format!(
            "unsupported url form: path must end in .md, got {normalized:?}"
        ));
    }

    Ok(normalized)
}

/// Derive the install directory slug from the SKILL.md frontmatter.
///
/// Prefers `metadata.id` (the spec-aligned identifier) when present. Falls
/// back to a sanitized form of `name`:
///   * lowercase ASCII
///   * non-alphanumeric runs collapsed to a single `-`
///   * leading/trailing `-` trimmed
///
/// Rejects the empty string and paths that would escape the skills root
/// (`..`, `/`, `\`). Max length is [`MAX_NAME_LEN`].
pub(crate) fn derive_install_slug(fm: &SkillFrontmatter) -> Result<String, String> {
    let candidate = fm
        .metadata
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| fm.name.clone());

    let mut out = String::with_capacity(candidate.len());
    let mut last_dash = false;
    for ch in candidate.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }

    if out.is_empty() {
        return Err(
            "invalid SKILL.md: cannot derive slug from empty name/id — set a value in frontmatter"
                .to_string(),
        );
    }
    if out.len() > MAX_NAME_LEN {
        return Err(format!(
            "invalid SKILL.md: derived slug {out:?} exceeds {MAX_NAME_LEN} chars"
        ));
    }
    if out.contains("..") || out.contains('/') || out.contains('\\') {
        return Err(format!(
            "invalid SKILL.md: derived slug {out:?} contains forbidden path components"
        ));
    }

    Ok(out)
}

/// Validate a remote skill install URL. Returns `Ok(())` when the URL is
/// well-formed, uses `https`, and points at a public host.
///
/// Rejects:
/// * empty string or > [`MAX_INSTALL_URL_LEN`] bytes
/// * non-`https` schemes (including `http`, `ftp`, `file`, `git+ssh`)
/// * missing or empty host
/// * `localhost`, `*.localhost`, `*.local`
/// * IPv4 literals in loopback (127.0.0.0/8), private (10/8, 172.16/12,
///   192.168/16), link-local (169.254/16), shared-address (100.64/10),
///   multicast, broadcast, or unspecified (0.0.0.0) ranges
/// * IPv6 literals in loopback (::1), unspecified (::), unique-local
///   (fc00::/7), link-local (fe80::/10), or multicast (ff00::/8)
pub fn validate_install_url(raw: &str) -> Result<(), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("url must not be empty".to_string());
    }
    if trimmed.len() > MAX_INSTALL_URL_LEN {
        return Err(format!(
            "url exceeds max {MAX_INSTALL_URL_LEN} chars (got {})",
            trimmed.len()
        ));
    }
    let parsed = url::Url::parse(trimmed).map_err(|e| format!("invalid url {trimmed:?}: {e}"))?;
    if parsed.scheme() != "https" {
        return Err(format!(
            "url scheme {:?} not allowed; https only",
            parsed.scheme()
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| format!("url {trimmed:?} has no host"))?;
    if host.is_empty() {
        return Err(format!("url {trimmed:?} has empty host"));
    }
    if is_blocked_install_host(host) {
        return Err(format!(
            "host {host:?} not allowed (loopback/private/link-local/multicast)"
        ));
    }
    Ok(())
}

/// Resolve the host in the given URL and reject if any returned IP falls in
/// loopback / private / link-local / multicast / unspecified ranges.
///
/// Covers the DNS-to-private-IP SSRF vector: a public-looking hostname can
/// still resolve to 127.0.0.1 / 169.254.x / fc00::/7 etc., which
/// [`validate_install_url`] alone cannot detect because it only inspects
/// literal IP hosts.
///
/// Caveat: does **not** close the DNS-rebinding gap. `reqwest` performs its
/// own DNS lookup on the GET below, and a rebinding server can answer the
/// check with a public IP and answer reqwest with a private one. Full
/// mitigation requires resolving to a `SocketAddr` here and passing it to
/// reqwest via a custom resolver that only honours the pinned address.
pub async fn validate_resolved_host(raw_url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(raw_url)
        .map_err(|e| format!("invalid url {raw_url:?} during DNS guard: {e}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| format!("url {raw_url:?} has no host (DNS guard)"))?;
    // `tokio::net::lookup_host` wants "host:port". Default https → 443.
    let port = parsed.port_or_known_default().unwrap_or(443);
    // IPv6 literal hosts come back bracketed from `url::Url`; `lookup_host`
    // needs the bracketed form for IPv6 to parse correctly.
    let lookup_target = if parsed
        .host()
        .map(|h| matches!(h, url::Host::Ipv6(_)))
        .unwrap_or(false)
    {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };

    tracing::debug!(
        host = %host,
        port = port,
        "[skills] validate_resolved_host: resolving"
    );

    let mut addrs = tokio::net::lookup_host(&lookup_target)
        .await
        .map_err(|e| format!("dns lookup failed for {host:?}: {e}"))?
        .peekable();
    if addrs.peek().is_none() {
        return Err(format!("host {host:?} resolved to no IP addresses"));
    }
    for addr in addrs {
        let ip = addr.ip();
        match ip {
            std::net::IpAddr::V4(v4) => {
                if is_private_v4(&v4) {
                    tracing::warn!(
                        host = %host,
                        resolved = %v4,
                        "[skills] validate_resolved_host: rejected private IPv4"
                    );
                    return Err(format!(
                        "host {host:?} resolved to non-public IPv4 {v4} (loopback/private/link-local)"
                    ));
                }
            }
            std::net::IpAddr::V6(v6) => {
                if is_private_v6(&v6) {
                    tracing::warn!(
                        host = %host,
                        resolved = %v6,
                        "[skills] validate_resolved_host: rejected private IPv6"
                    );
                    return Err(format!(
                        "host {host:?} resolved to non-public IPv6 {v6} (loopback/ula/link-local)"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn is_blocked_install_host(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    // url::Url::host_str returns IPv6 literals wrapped in brackets (e.g. "[::1]").
    // Strip them before attempting Ipv6Addr parse.
    let stripped = lower
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(&lower);
    if stripped == "localhost" || stripped.ends_with(".localhost") || stripped.ends_with(".local") {
        return true;
    }
    if let Ok(v4) = stripped.parse::<Ipv4Addr>() {
        return is_private_v4(&v4);
    }
    if let Ok(v6) = stripped.parse::<Ipv6Addr>() {
        return is_private_v6(&v6);
    }
    false
}

fn is_private_v4(ip: &Ipv4Addr) -> bool {
    if ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_multicast()
    {
        return true;
    }
    let [a, b, _, _] = ip.octets();
    // 100.64.0.0/10 shared address (CGN)
    if a == 100 && (64..=127).contains(&b) {
        return true;
    }
    // 0.0.0.0/8
    if a == 0 {
        return true;
    }
    false
}

fn is_private_v6(ip: &Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    let first = ip.segments()[0];
    // fc00::/7 unique-local
    if (first & 0xfe00) == 0xfc00 {
        return true;
    }
    // fe80::/10 link-local
    if (first & 0xffc0) == 0xfe80 {
        return true;
    }
    false
}
