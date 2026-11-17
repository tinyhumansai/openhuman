//! HTTPS fetch of a remote `SKILL.md` and installation into the user skills
//! root.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use tinyskills::{
    fetch_skill_document, redact_url, write_installed_document, DocumentWrite, FetchPolicy,
    FetchedDocument, RegistryError, RegistryLimits, SystemResolver, MAX_INSTALL_DOCUMENT_BYTES,
};

use super::super::ops_discover::{discover_workflows_inner, is_workspace_trusted};
use super::scan_gate::{fetch_scanned, gate_install, ScanAcknowledgement, SkillInstallOutcome};
use super::url_validation::read_allow_local_http_env;
use super::url_validation::{normalize_install_url, validate_install_url_with_config};
use crate::skills::catalog::ReqwestTransport;

/// Default wall-clock budget for the SKILL.md fetch.
pub const DEFAULT_INSTALL_TIMEOUT_SECS: u64 = 60;
/// Prefix on the error a throttled host produces, so callers can tell
/// "come back shortly" apart from "this host is unreachable" without parsing
/// a status code back out of prose. A `Retry-After` delay is appended when the
/// host sent a parseable one.
pub const RATE_LIMITED_ERROR_PREFIX: &str = "rate limited";
/// Hard ceiling callers can request via `timeout_secs`.
pub const MAX_INSTALL_TIMEOUT_SECS: u64 = 600;
/// Upper bound on the fetched SKILL.md body. Single-file skills rarely exceed
/// a few KB; the 1 MiB cap here is a defensive limit against a hostile or
/// misconfigured host streaming an unbounded response into memory.
pub const MAX_WORKFLOW_MD_BYTES: usize = MAX_INSTALL_DOCUMENT_BYTES;

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
/// Runtime (the tinyskills fetch guard over [`ReqwestTransport`]):
/// * The host is resolved once and the connection pinned to the checked
///   public addresses; every redirect hop is re-validated.
/// * Body size is capped by [`MAX_WORKFLOW_MD_BYTES`] (1 MiB), from the
///   advertised `Content-Length` and again while the body streams.
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
///
/// The fetched document goes through the supply-chain scan gate: a blocking
/// scan or a failed fetch is retried once, and a document that still blocks
/// is returned as [`SkillInstallOutcome::ScanBlocked`] unless `acknowledgement`
/// names that document's digest.
pub async fn install_workflow_from_url(
    workspace_dir: &Path,
    params: InstallWorkflowFromUrlParams,
    acknowledgement: ScanAcknowledgement,
) -> Result<SkillInstallOutcome, String> {
    let home = dirs::home_dir();
    let allow_local_http = read_allow_local_http_env();
    install_workflow_from_url_with_home(
        workspace_dir,
        params,
        home.as_deref(),
        allow_local_http,
        acknowledgement,
    )
    .await
}

/// Whether a non-`2xx` status is worth reporting: a `4xx` means the URL is
/// wrong or the skill is gone, which is user or catalog input, not a defect.
pub(crate) fn should_report_install_fetch_status(status: u16) -> bool {
    !(200..300).contains(&status) && !(400..500).contains(&status)
}

/// Report a failed `SKILL.md` fetch when it is not user input: timeouts,
/// transport failures, a broken transport contract and `5xx` statuses. `url`
/// is the redacted URL when the caller knows it.
pub(crate) fn report_install_fetch_failure(error: &RegistryError, url: Option<&str>) {
    let (failure, status) = match error {
        _ if error.is_timeout() => ("timeout", None),
        RegistryError::Transport(_) => ("transport", None),
        RegistryError::TransportContract { .. } => ("transport_contract", None),
        RegistryError::Unavailable { status } if should_report_install_fetch_status(*status) => {
            ("non_2xx", Some(status.to_string()))
        }
        _ => {
            tracing::debug!(
                kind = error.kind().as_str(),
                "[skills] install fetch: not reported (user or catalog input)"
            );
            return;
        }
    };
    let message = match (url, status.as_deref()) {
        (Some(url), Some(status)) => format!("fetch failed: {url} returned status {status}"),
        _ => format!("fetch failed: {error}"),
    };
    let mut tags = vec![("failure", failure)];
    if let Some(url) = url {
        tags.push(("url", url));
    }
    if let Some(status) = status.as_deref() {
        tags.push(("status", status));
    }
    crate::core::observability::report_error(message.as_str(), "skills", "install_fetch", &tags);
}

/// The caller-facing message for a failed `SKILL.md` fetch from a URL.
pub(crate) fn install_fetch_error(
    error: &RegistryError,
    fetch_url: &str,
    timeout_secs: u64,
) -> String {
    report_install_fetch_failure(error, Some(&redact_url(fetch_url)));
    match error {
        _ if error.is_timeout() => format!("fetch timed out after {timeout_secs}s"),
        RegistryError::Transport(transport) => format!("fetch failed: {transport}"),
        RegistryError::TransportContract { .. } => format!("fetch failed: {error}"),
        RegistryError::RateLimited { retry_after } => match retry_after {
            Some(delay) => format!(
                "{RATE_LIMITED_ERROR_PREFIX} by {fetch_url}: retry after {}s",
                delay.as_secs()
            ),
            None => format!("{RATE_LIMITED_ERROR_PREFIX} by {fetch_url}: retry shortly"),
        },
        RegistryError::Unavailable { status } => {
            format!("fetch failed: {fetch_url} returned status {status}")
        }
        RegistryError::UnsafeUrl(inner) => inner.to_string(),
        RegistryError::InvalidDocument(inner) => inner.to_string(),
        other => other.to_string(),
    }
}

pub(crate) async fn install_workflow_from_url_with_home(
    workspace_dir: &Path,
    params: InstallWorkflowFromUrlParams,
    home: Option<&Path>,
    allow_local_http: bool,
    acknowledgement: ScanAcknowledgement,
) -> Result<SkillInstallOutcome, String> {
    let raw_url = params.url.trim().to_string();
    validate_install_url_with_config(&raw_url, allow_local_http)?;

    let timeout_secs = params
        .timeout_secs
        .unwrap_or(DEFAULT_INSTALL_TIMEOUT_SECS)
        .clamp(1, MAX_INSTALL_TIMEOUT_SECS);

    let fetch_url = normalize_install_url(&raw_url)?;

    tracing::debug!(
        raw_url = %redact_url(&raw_url),
        fetch_url = %redact_url(&fetch_url),
        workspace = %workspace_dir.display(),
        timeout_secs = timeout_secs,
        "[skills] install_workflow_from_url: entry"
    );

    let mut policy = FetchPolicy::default();
    policy.allow_loopback_http = allow_local_http;
    policy.user_agent = format!("openhuman-core/{}", env!("CARGO_PKG_VERSION"));
    let mut timeouts = crate::skills::catalog::registry_timeouts();
    timeouts.document = Duration::from_secs(timeout_secs);
    let mut limits = RegistryLimits::default();
    limits.max_document_bytes = MAX_WORKFLOW_MD_BYTES as u64;

    let transport: Arc<ReqwestTransport> = Arc::new(ReqwestTransport::new());
    let fetched = fetch_scanned(&raw_url, &acknowledgement, || {
        fetch_skill_document(
            transport.clone(),
            Arc::new(SystemResolver),
            &fetch_url,
            &policy,
            &timeouts,
            &limits,
        )
    })
    .await
    .map_err(|error| install_fetch_error(&error, &fetch_url, timeout_secs))?;

    gate_install(&raw_url, &acknowledgement, fetched, |document| {
        install_validated_document(workspace_dir, home, &raw_url, &fetch_url, document.document)
    })
}

/// Write a validated `SKILL.md` into the user skills root, re-discover, and
/// announce the change. An existing `SKILL.md` for the same slug is an
/// idempotent success with no new skills.
pub(crate) fn install_validated_document(
    workspace_dir: &Path,
    home: Option<&Path>,
    source_url: &str,
    fetched_from: &str,
    document: FetchedDocument,
) -> Result<InstallWorkflowFromUrlOutcome, String> {
    let redacted_source = redact_url(source_url);
    let redacted_fetched = redact_url(fetched_from);
    let slug = document.slug;
    let content = document.content;
    let parse_warnings = document.warnings;

    let trusted_before = is_workspace_trusted(workspace_dir);
    let before: std::collections::HashSet<String> =
        discover_workflows_inner(home, Some(workspace_dir), trusted_before)
            .into_iter()
            .map(|s| s.name)
            .collect();

    let skills_root = crate::skills::write_root::user_skill_install_root(workspace_dir, home)
        .ok_or_else(|| "write failed: unable to resolve home directory".to_string())?;

    let target_file =
        match write_installed_document(&skills_root, &slug, &content).map_err(|e| e.to_string())? {
            DocumentWrite::Installed(path) => path,
            DocumentWrite::AlreadyInstalled(target_file) => {
                tracing::info!(
                    source_url = %redacted_source,
                    fetched_from = %redacted_fetched,
                    slug = %slug,
                    target = %target_file.display(),
                    "[skills] install: already installed"
                );

                return Ok(InstallWorkflowFromUrlOutcome {
                    url: source_url.to_owned(),
                    stdout: format!(
                        "Skill {slug:?} is already installed at {}",
                        target_file.display()
                    ),
                    stderr: parse_warnings.join("\n"),
                    new_skills: Vec::new(),
                });
            }
        };

    let trusted_after = is_workspace_trusted(workspace_dir);
    let after = discover_workflows_inner(home, Some(workspace_dir), trusted_after);
    let new_skills: Vec<String> = after
        .into_iter()
        .map(|s| s.name)
        .filter(|name| !before.contains(name))
        .collect();

    tracing::info!(
        source_url = %redacted_source,
        fetched_from = %redacted_fetched,
        slug = %slug,
        bytes = content.len(),
        new_count = new_skills.len(),
        "[skills] install: completed"
    );

    let stdout = format!(
        "Fetched {} bytes from {fetched_from}\nInstalled to {}",
        content.len(),
        target_file.display()
    );
    let stderr = parse_warnings.join("\n");

    crate::skills::ops_discover::invalidate_workflow_metadata_cache();
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::WorkflowsChanged {
        reason: "install".to_string(),
    });

    Ok(InstallWorkflowFromUrlOutcome {
        url: source_url.to_owned(),
        stdout,
        stderr,
        new_skills,
    })
}
