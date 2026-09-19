//! HTTPS fetch of a remote `SKILL.md` and installation into the user skills
//! root.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::super::ops_discover::{discover_workflows_inner, is_workspace_trusted};
use super::super::ops_parse::parse_workflow_md_str;
use super::super::ops_types::SKILL_MD;
use super::url_validation::{is_loopback_http_url, read_allow_local_http_env};
use super::url_validation::{
    normalize_install_url, validate_install_url_with_config, validate_resolved_host,
};

/// Strip userinfo, query, and fragment from a URL for safe inclusion in
/// observability tags. Returns `<scheme>://<host>[:<port>]<path>` on success,
/// or `"<unparseable>"` on parse failure. Never returns the raw URL — even
/// validated install URLs may carry signed query params or embedded creds we
/// don't want flowing to Sentry.
fn redact_url(raw: &str) -> String {
    match url::Url::parse(raw) {
        Ok(u) => {
            let scheme = u.scheme();
            let host = u.host_str().unwrap_or("");
            let port = u.port().map(|p| format!(":{p}")).unwrap_or_default();
            let path = u.path();
            format!("{scheme}://{host}{port}{path}")
        }
        Err(_) => "<unparseable>".to_string(),
    }
}

/// Default wall-clock budget for the SKILL.md fetch.
pub const DEFAULT_INSTALL_TIMEOUT_SECS: u64 = 60;
/// Hard ceiling callers can request via `timeout_secs`.
pub const MAX_INSTALL_TIMEOUT_SECS: u64 = 600;
/// Upper bound on the fetched SKILL.md body. Single-file skills rarely exceed
/// a few KB; the 1 MiB cap here is a defensive limit against a hostile or
/// misconfigured host streaming an unbounded response into memory.
pub const MAX_WORKFLOW_MD_BYTES: usize = 1024 * 1024;

/// Input for [`install_workflow_from_url`]. Mirrors the `skills.install_from_url`
/// JSON-RPC payload.
#[derive(Debug, Clone, Deserialize)]
pub struct InstallWorkflowFromUrlParams {
    /// Remote SKILL.md URL. Must be `https://`, resolve to a non-private host
    /// (see [`super::url_validation::validate_install_url`]), and point at a `.md`
    /// file after github.com `/blob/` normalization.
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
pub struct InstallWorkflowFromUrlOutcome {
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
///   [`super::url_validation::validate_install_url`] — rejects loopback, private,
///   link-local, multicast, shared-address ranges, `localhost`, and `.local` /
///   `.localhost` mDNS-style hostnames.
/// * `github.com/<o>/<r>/blob/<b>/<p>` is rewritten to the raw
///   `raw.githubusercontent.com/<o>/<r>/<b>/<p>` equivalent so humans can
///   paste the URL they see in the browser.
/// * The path must end in `.md` (case-insensitive). Repo/tree URLs and
///   tarballs are rejected with `unsupported url form:`.
/// * `timeout_secs` is clamped to [`MAX_INSTALL_TIMEOUT_SECS`].
///
/// Runtime:
/// * Body size is capped by [`MAX_WORKFLOW_MD_BYTES`] (1 MiB). The advertised
///   `Content-Length` is checked up front; the buffered body length is
///   checked again after the download as defense against a lying header.
/// * Frontmatter is validated — `name` and `description` are required per
///   the agentskills.io spec.
/// * The slug is derived from `metadata.id` when present, otherwise the
///   sanitized `name` field. If the target directory already contains a
///   `SKILL.md`, the install is treated as an idempotent success and reports
///   that the skill is already installed. Other directory collisions remain
///   fatal, and existing files are never silently overwritten.
/// * Write is atomic: `SKILL.md.tmp` in the target dir, then `rename` on
///   success.
///
/// On success the full post-install skills catalog is re-discovered and the
/// outcome includes the list of skill slugs that appeared since the start of
/// the call.
pub async fn install_workflow_from_url(
    workspace_dir: &Path,
    params: InstallWorkflowFromUrlParams,
) -> Result<InstallWorkflowFromUrlOutcome, String> {
    let home = dirs::home_dir();
    let allow_local_http = read_allow_local_http_env();
    install_workflow_from_url_with_home(workspace_dir, params, home.as_deref(), allow_local_http)
        .await
}

pub(crate) fn should_report_install_fetch_status(status: reqwest::StatusCode) -> bool {
    !status.is_success() && !status.is_client_error()
}

pub(crate) async fn install_workflow_from_url_with_home(
    workspace_dir: &Path,
    params: InstallWorkflowFromUrlParams,
    home: Option<&Path>,
    allow_local_http: bool,
) -> Result<InstallWorkflowFromUrlOutcome, String> {
    let raw_url = params.url.trim().to_string();
    validate_install_url_with_config(&raw_url, allow_local_http)?;

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
    if !(allow_local_http && is_loopback_http_url(&fetch_url)) {
        validate_resolved_host(&fetch_url).await?;
    }

    let redacted_raw_url = redact_url(&raw_url);
    let redacted_fetch_url = redact_url(&fetch_url);

    tracing::debug!(
        raw_url = %redacted_raw_url,
        fetch_url = %redacted_fetch_url,
        workspace = %workspace_dir.display(),
        timeout_secs = timeout_secs,
        "[skills] install_workflow_from_url: entry"
    );

    let trusted_before = is_workspace_trusted(workspace_dir);
    let before: std::collections::HashSet<String> =
        discover_workflows_inner(home, Some(workspace_dir), trusted_before)
            .into_iter()
            .map(|s| s.name)
            .collect();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("fetch failed: build http client: {e}"))?;

    tracing::info!(
        fetch_url = %redacted_fetch_url,
        "[skills] install_workflow_from_url: fetching SKILL.md"
    );

    let response = match client.get(&fetch_url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            let (failure, msg) = if e.is_timeout() {
                ("timeout", format!("fetch timed out after {timeout_secs}s"))
            } else {
                ("transport", format!("fetch failed: {e}"))
            };
            crate::core::observability::report_error(
                msg.as_str(),
                "skills",
                "install_fetch",
                &[("url", redacted_fetch_url.as_str()), ("failure", failure)],
            );
            return Err(msg);
        }
    };

    let status = response.status();
    if !status.is_success() {
        // A 4xx (esp. 404/410) means the requested SKILL.md is gone or the URL
        // is wrong — expected user/catalog input state, surfaced to the UI as
        // "skill not found". Don't page Sentry for it (TAURI-RUST-CGE: ~1,446
        // events / 72 users on `openhuman@0.57.53`, almost all 404). Keep
        // reporting 5xx — a genuine remote failure is still Sentry-actionable.
        // The `Err(msg)` return is unchanged in both cases so the UI always
        // surfaces the failure.
        let status_str = status.as_u16().to_string();
        let msg = format!(
            "fetch failed: {fetch_url} returned status {}",
            status.as_u16()
        );
        let report_msg = format!(
            "fetch failed: {redacted_fetch_url} returned status {}",
            status.as_u16()
        );
        if should_report_install_fetch_status(status) {
            crate::core::observability::report_error(
                report_msg.as_str(),
                "skills",
                "install_fetch",
                &[
                    ("url", redacted_fetch_url.as_str()),
                    ("status", status_str.as_str()),
                    ("failure", "non_2xx"),
                ],
            );
        } else {
            tracing::debug!(
                fetch_url = %redacted_fetch_url,
                status = status.as_u16(),
                "[skills] install_workflow_from_url: skipped Sentry report for user/catalog fetch status"
            );
        }
        return Err(msg);
    }

    if let Some(len) = response.content_length() {
        if len > MAX_WORKFLOW_MD_BYTES as u64 {
            return Err(format!(
                "fetch too large: {} bytes exceeds {MAX_WORKFLOW_MD_BYTES} limit",
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

    if bytes.len() > MAX_WORKFLOW_MD_BYTES {
        return Err(format!(
            "fetch too large: {} bytes exceeds {MAX_WORKFLOW_MD_BYTES} limit",
            bytes.len()
        ));
    }

    let content = String::from_utf8(bytes.to_vec())
        .map_err(|e| format!("invalid SKILL.md: body is not valid utf-8: {e}"))?;

    let (frontmatter, _body, parse_warnings) =
        parse_workflow_md_str(&content).ok_or_else(|| {
            "invalid SKILL.md: frontmatter block opened with `---` but never terminated".to_string()
        })?;

    if frontmatter.name.trim().is_empty() {
        return Err("invalid SKILL.md: missing required field 'name'".to_string());
    }
    if frontmatter.description.trim().is_empty() {
        return Err("invalid SKILL.md: missing required field 'description'".to_string());
    }

    let slug = super::url_validation::derive_install_slug(&frontmatter)?;

    // Install to user scope (`~/.openhuman/skills/<slug>`), which `discover_workflows`
    // scans unconditionally. Project scope (`<ws>/.openhuman/skills/`) is gated on
    // a `<ws>/.openhuman/trust` marker and would render the install invisible to the
    // skills list until the user opts the workspace into trust.
    let skills_root = home
        .ok_or_else(|| "write failed: unable to resolve home directory".to_string())?
        .join(".openhuman")
        .join("skills");
    let target_dir = skills_root.join(&slug);
    if target_dir.exists() {
        let target_file = target_dir.join(SKILL_MD);
        if !target_file.is_file() {
            return Err(format!(
                "skill install target already exists but has no {SKILL_MD}: {}",
                target_dir.display()
            ));
        }

        tracing::info!(
            raw_url = %redacted_raw_url,
            fetch_url = %redacted_fetch_url,
            slug = %slug,
            target = %target_file.display(),
            "[skills] install_workflow_from_url: already installed"
        );

        return Ok(InstallWorkflowFromUrlOutcome {
            url: raw_url,
            stdout: format!(
                "Skill {slug:?} is already installed at {}",
                target_file.display()
            ),
            stderr: parse_warnings.join("\n"),
            new_skills: Vec::new(),
        });
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
                "[skills] install_workflow_from_url: rollback remove_dir failed (non-fatal)"
            );
        } else {
            tracing::warn!(
                target_dir = %target_dir.display(),
                "[skills] install_workflow_from_url: rolled back partial install after write failure"
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
                "[skills] install_workflow_from_url: chmod 0644 failed (non-fatal)"
            );
        }
    }

    let trusted_after = is_workspace_trusted(workspace_dir);
    let after = discover_workflows_inner(home, Some(workspace_dir), trusted_after);
    let new_skills: Vec<String> = after
        .into_iter()
        .map(|s| s.name)
        .filter(|name| !before.contains(name))
        .collect();

    tracing::info!(
        raw_url = %redacted_raw_url,
        fetch_url = %redacted_fetch_url,
        slug = %slug,
        bytes = content.len(),
        new_count = new_skills.len(),
        "[skills] install_workflow_from_url: completed"
    );

    let stdout = format!(
        "Fetched {} bytes from {fetch_url}\nInstalled to {}",
        content.len(),
        target_file.display()
    );
    let stderr = parse_warnings.join("\n");

    // Notify live agent sessions so they refresh their `## Installed Skills`
    // catalogue mid-conversation (see `OpenHumanSessionHost::refresh_workflows`).
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::WorkflowsChanged {
        reason: "install".to_string(),
    });

    Ok(InstallWorkflowFromUrlOutcome {
        url: raw_url,
        stdout,
        stderr,
        new_skills,
    })
}
