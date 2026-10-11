//! Business logic for the skill registry: thin adapters over the tinyskills
//! [`SkillRegistry`] for browse, search, facets, detail and install.
//!
//! The registry owns fetching, caching, stale-while-revalidate, single-flight
//! refresh, ranking and `SKILL.md` resolution. This module maps OpenHuman's
//! queries onto it, shapes the results for RPC and tools, and applies the
//! host's install and reporting policy.

use std::path::Path;

use tinyskills::{
    EntryKey, ReadPolicy, RegistryError, RegistryErrorKind, SkillDetail, SkillQuery, SkillRegistry,
    SkillSummary,
};

use super::registry::skill_registry;
use crate::skills::ops_install::{
    fetch_scanned, gate_install, ScanAcknowledgement, SkillInstallOutcome,
};
use {
    super::types::CatalogDetail, super::types::CatalogPage, super::types::CatalogQuery,
    super::types::RegistryCatalogEntry, super::types::DEFAULT_PAGE_SIZE,
    super::types::MAX_PAGE_SIZE, tinyskills::CatalogEntry, tinyskills::RegistryFacets,
};

const REFRESH_ON_BOOT_ENV: &str = "OPENHUMAN_SKILL_REGISTRY_REFRESH_ON_BOOT";
/// Prefix of every registry error a caller sees, followed by the
/// upper-cased [`RegistryErrorKind`] and `: `.
pub const REGISTRY_ERROR_PREFIX: &str = "SKILL_REGISTRY_";

/// Start a one-shot background warm-up of the skill registry.
///
/// Loads the stored catalog (or fetches one when none is stored) without
/// making core readiness depend on registry availability. Set
/// `OPENHUMAN_SKILL_REGISTRY_REFRESH_ON_BOOT=0` to disable it.
pub fn start_boot_catalog_refresh() {
    static STARTED: std::sync::Once = std::sync::Once::new();

    STARTED.call_once(|| {
        if !refresh_on_boot_enabled(std::env::var(REFRESH_ON_BOOT_ENV).ok().as_deref()) {
            tracing::info!(
                env = REFRESH_ON_BOOT_ENV,
                "[skill_registry] boot catalog refresh disabled"
            );
            return;
        }

        tracing::info!("[skill_registry] scheduling boot catalog warm-up");
        tokio::spawn(async {
            let started = std::time::Instant::now();
            let statuses = skill_registry().warm().await;
            for status in statuses {
                match &status.last_error {
                    None => tracing::info!(
                        registry = %status.id,
                        entries = status.entry_count,
                        freshness = ?status.freshness,
                        elapsed_ms = started.elapsed().as_millis(),
                        "[skill_registry] boot warm-up complete"
                    ),
                    Some(error) => {
                        observe_summary("warm", error);
                    }
                }
            }
        });
    });
}

fn refresh_on_boot_enabled(raw: Option<&str>) -> bool {
    let Some(raw) = raw else { return true };
    let value = raw.trim();
    !(value == "0"
        || value.eq_ignore_ascii_case("false")
        || value.eq_ignore_ascii_case("no")
        || value.eq_ignore_ascii_case("off"))
}

/// The caller-facing text of a registry error:
/// `SKILL_REGISTRY_<KIND>: <message>`.
pub fn registry_error_message(error: &RegistryError) -> String {
    let mut message = format!(
        "{REGISTRY_ERROR_PREFIX}{}: {error}",
        error.kind().as_str().to_ascii_uppercase()
    );
    if let RegistryError::NoDirectDownload {
        source_url: Some(url),
        ..
    } = error
    {
        message.push_str(&format!(". View it at {url}"));
    }
    message
}

/// Whether a catalog-read failure is a defect worth reporting, as opposed to
/// an upstream outage, throttling, or a caller asking for something absent.
/// Install fetches report through `report_install_fetch_failure`.
fn is_reportable(kind: RegistryErrorKind, catalog_read: bool) -> bool {
    catalog_read
        && matches!(
            kind,
            RegistryErrorKind::TransportContract | RegistryErrorKind::Malformed
        )
}

fn observe(operation: &'static str, error: &RegistryError, catalog_read: bool) {
    let kind = error.kind();
    if is_reportable(kind, catalog_read) {
        crate::core::observability::report_error(
            error,
            "skills",
            "registry",
            &[("failure", kind.as_str()), ("registry_op", operation)],
        );
    } else if error.is_unavailable() || kind == RegistryErrorKind::TooLarge {
        tracing::warn!(
            operation,
            kind = kind.as_str(),
            error = %error,
            "[skill_registry] upstream unavailable"
        );
    } else {
        tracing::debug!(
            operation,
            kind = kind.as_str(),
            error = %error,
            "[skill_registry] request refused"
        );
    }
}

fn observe_summary(operation: &'static str, error: &tinyskills::RegistryErrorSummary) {
    if is_reportable(error.kind, true) {
        crate::core::observability::report_error(
            error.message.as_str(),
            "skills",
            "registry",
            &[("failure", error.kind.as_str()), ("registry_op", operation)],
        );
    } else {
        tracing::warn!(
            operation,
            kind = error.kind.as_str(),
            error = %error.message,
            "[skill_registry] source has no fresh catalog"
        );
    }
}

fn entry_from_summary(summary: SkillSummary) -> RegistryCatalogEntry {
    RegistryCatalogEntry {
        entry: CatalogEntry {
            id: summary.id,
            name: summary.name,
            description: summary.description,
            source: summary.upstream,
            category: summary.category,
            author: summary.author,
            version: summary.version,
            tags: summary.tags,
            platforms: summary.platforms,
            download_url: String::new(),
            source_url: summary.source_url,
            docs_path: None,
            commands: Vec::new(),
            env_vars: Vec::new(),
            license: None,
        },
        registry: summary.registry,
        installable: summary.installable,
        category_label: summary.category_label,
    }
}

fn entry_from_detail(detail: SkillDetail) -> RegistryCatalogEntry {
    let mut shaped = entry_from_summary(detail.summary);
    shaped.entry.download_url = detail.download_url;
    shaped.entry.docs_path = detail.docs_path;
    shaped.entry.commands = detail.commands;
    shaped.entry.env_vars = detail.env_vars;
    shaped.entry.license = detail.license;
    shaped
}

fn skill_query(query: &CatalogQuery) -> SkillQuery {
    let mut skill = SkillQuery::text(query.text.trim());
    skill.upstreams = query.upstreams.clone();
    skill.categories = query.categories.clone();
    skill.read = ReadPolicy::AllowStale;
    if query.is_paged() {
        skill.page = query.page.unwrap_or(1).max(1);
        skill.page_size = query
            .page_size
            .unwrap_or(DEFAULT_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE);
    } else {
        skill.page = 1;
        skill.page_size = usize::MAX;
    }
    skill
}

/// One page of matches from the process registry.
pub async fn catalog_page(query: &CatalogQuery) -> Result<CatalogPage, RegistryError> {
    catalog_page_in(&skill_registry(), query).await
}

pub(crate) async fn catalog_page_in(
    registry: &SkillRegistry,
    query: &CatalogQuery,
) -> Result<CatalogPage, RegistryError> {
    tracing::debug!(
        text = %query.text,
        upstreams = query.upstreams.len(),
        categories = query.categories.len(),
        page = ?query.page,
        page_size = ?query.page_size,
        force_refresh = query.force_refresh,
        "[skill_registry] catalog_page"
    );
    if query.force_refresh {
        let statuses = registry.refresh(None, true).await?;
        for status in &statuses {
            if let Some(error) = &status.last_error {
                tracing::warn!(
                    registry = %status.id,
                    kind = error.kind.as_str(),
                    error = %error.message,
                    "[skill_registry] forced refresh failed; serving what is held"
                );
            }
        }
    }
    let page = registry
        .search(&skill_query(query))
        .await
        .inspect_err(|error| observe("search", error, true))?;

    let mut entries = Vec::with_capacity(page.items.len());
    for summary in page.items {
        if !query.is_paged() {
            entries.push(entry_from_summary(summary));
            continue;
        }
        let key = EntryKey::in_registry(summary.registry.clone(), summary.id.clone());
        match registry.detail(&key).await {
            Ok(detail) => entries.push(entry_from_detail(detail)),
            Err(error) => {
                tracing::debug!(
                    id = %summary.id,
                    error = %error,
                    "[skill_registry] detail unavailable for a hit; returning its summary"
                );
                entries.push(entry_from_summary(summary));
            }
        }
    }

    let refreshing = page.sources.iter().any(|source| source.refreshing);
    let last_error = page
        .sources
        .iter()
        .find_map(|source| source.last_error.clone());
    tracing::debug!(
        total = page.total,
        returned = entries.len(),
        page = page.page,
        freshness = ?page.freshness,
        refreshing,
        has_error = last_error.is_some(),
        "[skill_registry] catalog_page result"
    );
    Ok(CatalogPage {
        entries,
        total: page.total,
        page: page.page,
        page_size: page.page_size,
        total_pages: page.total_pages,
        freshness: page.freshness,
        fetched_at: page.fetched_at,
        refreshing,
        last_error,
    })
}

/// Upstream and category facets across every source.
pub async fn catalog_facets() -> Result<RegistryFacets, RegistryError> {
    catalog_facets_in(&skill_registry()).await
}

pub(crate) async fn catalog_facets_in(
    registry: &SkillRegistry,
) -> Result<RegistryFacets, RegistryError> {
    registry
        .facets(None)
        .await
        .inspect_err(|error| observe("facets", error, true))
}

/// Everything known about one entry, by id or unique name.
pub async fn catalog_detail(entry_id: &str) -> Result<CatalogDetail, RegistryError> {
    catalog_detail_in(&skill_registry(), entry_id).await
}

pub(crate) async fn catalog_detail_in(
    registry: &SkillRegistry,
    entry_id: &str,
) -> Result<CatalogDetail, RegistryError> {
    let detail = registry
        .detail(&EntryKey::new(entry_id))
        .await
        .inspect_err(|error| observe("detail", error, true))?;
    let overview = detail.overview.clone();
    let install_identifier = detail.install_identifier.clone();
    Ok(CatalogDetail {
        entry: entry_from_detail(detail),
        overview,
        install_identifier,
    })
}

/// Why a catalog install failed.
#[derive(Debug)]
pub enum CatalogInstallError {
    /// The registry could not locate or fetch the entry's `SKILL.md`.
    Registry(RegistryError),
    /// The document was fetched but could not be installed.
    Install(String),
}

impl CatalogInstallError {
    pub fn kind(&self) -> Option<RegistryErrorKind> {
        match self {
            Self::Registry(error) => Some(error.kind()),
            Self::Install(_) => None,
        }
    }
}

impl std::fmt::Display for CatalogInstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Registry(error) => f.write_str(&registry_error_message(error)),
            Self::Install(message) => f.write_str(message),
        }
    }
}

/// Install a catalog entry, by id or unique name, into the user skills root.
///
/// The document goes through the supply-chain scan gate: a blocking scan or
/// a failed fetch is retried once, and a document that still blocks comes
/// back as [`SkillInstallOutcome::ScanBlocked`] unless `acknowledgement` names
/// that document's digest.
pub async fn install_from_catalog(
    workspace_dir: &Path,
    entry_id: &str,
    acknowledgement: ScanAcknowledgement,
) -> Result<SkillInstallOutcome, CatalogInstallError> {
    install_from_catalog_in(
        &skill_registry(),
        workspace_dir,
        dirs::home_dir().as_deref(),
        entry_id,
        acknowledgement,
    )
    .await
}

pub(crate) async fn install_from_catalog_in(
    registry: &SkillRegistry,
    workspace_dir: &Path,
    home: Option<&Path>,
    entry_id: &str,
    acknowledgement: ScanAcknowledgement,
) -> Result<SkillInstallOutcome, CatalogInstallError> {
    tracing::info!(
        entry_id = %entry_id,
        acknowledged = acknowledgement.is_given(),
        "[skill_registry] installing from catalog"
    );
    let key = EntryKey::new(entry_id);
    let document = fetch_scanned(entry_id, &acknowledgement, || registry.fetch_document(&key))
        .await
        .map_err(|error| {
            observe("install", &error, false);
            crate::skills::ops_install::report_install_fetch_failure(&error, None);
            CatalogInstallError::Registry(error)
        })?;
    gate_install(entry_id, &acknowledgement, document, |document| {
        crate::skills::ops_install::install_validated_document(
            workspace_dir,
            home,
            &document.fetched_from,
            &document.fetched_from,
            document.document,
        )
    })
    .map_err(CatalogInstallError::Install)
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ops_install_tests.rs"]
mod install_tests;
