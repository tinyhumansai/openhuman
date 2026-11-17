//! JSON-RPC / CLI controller surface for diagnostics.
//!
//! Resolve the selected profile and run the native doctor check registry.

use std::collections::HashSet;
use std::path::Path;

use super::contract::DoctorProfileStatus;
use super::profile::DesktopProfileSelection;
use crate::config::Config;
use crate::core::{
    runtime::{ComponentSet, CoreContext, DispatchCtx},
    Outcome,
};
use crate::doctor::{
    DoctorConfigSource, DoctorCtx, DoctorOptions, DoctorReport, DoctorSummary, DoctorTier, Finding,
    ModelProbeReport,
};

pub async fn doctor_report(config: &Config) -> Result<Outcome<DoctorReport>, String> {
    doctor_report_with_options(config, DoctorOptions::default()).await
}

pub async fn doctor_report_with_options(
    config: &Config,
    mut options: DoctorOptions,
) -> Result<Outcome<DoctorReport>, String> {
    if options.profile.is_none() {
        if let Some(outcome) = unusable_default_config_outcome(config, &options)? {
            return Ok(outcome);
        }
    }
    if let Some(profile_id) = options.profile.take() {
        if crate::core::runtime::is_saas() {
            return Ok(profile_error_outcome(
                safe_profile_id(&profile_id),
                options.tier,
                super::profile::DesktopProfileError::Unavailable,
            ));
        }
        let Some(root) = super::profile::root_for_config(config)
            .or_else(|| crate::config::default_root_openhuman_dir().ok())
        else {
            return Ok(profile_error_outcome(
                safe_profile_id(&profile_id),
                options.tier,
                super::profile::DesktopProfileError::Missing,
            ));
        };
        return doctor_report_for_selected_profile(&root, &profile_id, options).await;
    }

    let mut ctx = DoctorCtx::from_config(config.clone());
    if ctx.profile().is_none() {
        if let Some(profile_id) = super::profile::id_for_config(config) {
            ctx = ctx.with_profile(profile_id, DoctorConfigSource::Profile);
        }
    }
    report_with_context(options, ctx).await
}

/// Runs a report under the dispatch's selected tenant context when present,
/// or resolves a desktop `--profile` request using the read-only loader.
pub async fn doctor_report_for_dispatch(
    dispatch: &DispatchCtx,
    mut options: DoctorOptions,
) -> Result<Outcome<DoctorReport>, String> {
    if crate::core::runtime::is_saas() {
        let target =
            crate::profiles::doctor::resolve_target(dispatch, options.profile.as_deref()).await?;
        if let Some(target) = target {
            options.profile = None;
            let components = ComponentSet::from_config(&target.config.components);
            let ctx = DoctorCtx::from_config_with_components(target.config.clone(), components)
                .with_profile(target.profile_id.as_str(), DoctorConfigSource::Tenant)
                .with_profile_metadata(None, Some("saas-tenant".into()));
            return CoreContext::scope(target.context, report_with_context(options, ctx)).await;
        }
    }

    if let Some(profile_id) = options.profile.take() {
        if crate::core::runtime::is_saas() {
            return Ok(profile_error_outcome(
                safe_profile_id(&profile_id),
                options.tier,
                super::profile::DesktopProfileError::Unavailable,
            ));
        }
        let root = dispatch
            .embedder_config()
            .as_ref()
            .and_then(super::profile::root_for_config)
            .or_else(super::profile::root_for_workspace_override)
            .or_else(|| crate::config::default_root_openhuman_dir().ok());
        let Some(root) = root else {
            return Ok(profile_error_outcome(
                safe_profile_id(&profile_id),
                options.tier,
                super::profile::DesktopProfileError::Missing,
            ));
        };
        return doctor_report_for_selected_profile(&root, &profile_id, options).await;
    }

    if crate::core::runtime::is_saas() {
        let config = dispatch.load_config().await?;
        return doctor_report_with_options(&config, options).await;
    }
    if let Some(config) = dispatch.embedder_config() {
        return doctor_report_with_options(&config, options).await;
    }

    // A doctor RPC is diagnostic, so resolve the desktop's effective config
    // through the read-only profile loader. `DispatchCtx::load_config()` falls
    // through to `Config::load_or_init()` when no embedder config exists; that
    // startup loader persists migrations and can rewrite config.toml during an
    // otherwise read-only report.
    let selection = desktop_selection_for_dispatch(dispatch).await?;
    doctor_report_for_desktop_selection(selection, options).await
}

/// Config resolution for non-SaaS doctor surfaces. Without an embedder-owned
/// snapshot, diagnostics use the read-only desktop profile loader instead of
/// `Config::load_or_init`, which can persist migrations.
pub(crate) async fn config_for_dispatch(dispatch: &DispatchCtx) -> Result<Config, String> {
    if crate::core::runtime::is_saas() {
        return dispatch.load_config().await;
    }
    if let Some(config) = dispatch.embedder_config() {
        return Ok(config);
    }
    Ok(desktop_selection_for_dispatch(dispatch).await?.config)
}

async fn desktop_selection_for_dispatch(
    dispatch: &DispatchCtx,
) -> Result<DesktopProfileSelection, String> {
    let process_root = dispatch.resolved().and_then(|context| {
        let (_, config_path) = context.workspace_config_binding().ok()?;
        super::profile::root_for_config_path(&config_path?)
    });
    let root = super::profile::root_for_workspace_override()
        .or(process_root)
        .or_else(|| crate::config::default_root_openhuman_dir().ok())
        .ok_or_else(|| "Could not resolve the desktop config root".to_owned())?;
    super::profile::resolve_desktop_profile_for_doctor(&root, None)
        .await
        .map_err(|error| error.message().to_owned())
}

/// Builds a desktop profile report from an explicit root and profile id.
/// This entry point does not load the active profile or initialize the core,
/// so hosts can use it before runtime bootstrap when they need a read-only
/// profile diagnostic.
pub async fn doctor_report_for_selected_profile(
    root: &Path,
    profile_id: &str,
    mut options: DoctorOptions,
) -> Result<Outcome<DoctorReport>, String> {
    options.profile = None;
    let selected = match super::profile::load_desktop_profile(root, profile_id).await {
        Ok(selected) => selected,
        Err(error) => {
            return Ok(profile_error_outcome(
                safe_profile_id(profile_id),
                options.tier,
                error,
            ));
        }
    };
    let components = ComponentSet::from_config(&selected.config.components);
    let inspection_profile_id = selected.id.clone();
    let ctx = DoctorCtx::from_config_with_components(selected.config.clone(), components)
        .with_profile(inspection_profile_id.clone(), DoctorConfigSource::Profile)
        .with_profile_metadata(Some(selected.layout.version), Some("--profile".into()))
        .with_profile_status(DoctorProfileStatus::Valid);
    let inspection_context =
        profile_inspection_context(&selected.config, Some(&inspection_profile_id));
    CoreContext::scope(inspection_context, report_with_context(options, ctx)).await
}

/// Runs a report from the effective desktop selection without consulting the
/// ambient core context. Hosts can resolve the config before boot and pass it
/// here to preserve profile identity and directory provenance.
pub async fn doctor_report_for_desktop_selection(
    selection: DesktopProfileSelection,
    options: DoctorOptions,
) -> Result<Outcome<DoctorReport>, String> {
    if let Some(outcome) = unusable_default_config_outcome(&selection.config, &options)? {
        return Ok(outcome);
    }
    let components = ComponentSet::from_config(&selection.config.components);
    let inspection_profile_id = selection.profile_id.clone();
    let mut ctx = DoctorCtx::from_config_with_components(selection.config.clone(), components)
        .with_profile_metadata(selection.layout_version, Some(selection.resolution_source));
    if let Some(profile_id) = inspection_profile_id.clone() {
        ctx = ctx.with_profile(profile_id, DoctorConfigSource::Profile);
    }
    if let Some(status) = selection.profile_status {
        ctx = ctx.with_profile_status(status);
    }
    let inspection_context =
        profile_inspection_context(&selection.config, inspection_profile_id.as_deref());
    CoreContext::scope(inspection_context, report_with_context(options, ctx)).await
}

fn profile_inspection_context(
    config: &Config,
    _profile_id: Option<&str>,
) -> std::sync::Arc<CoreContext> {
    let Some(parent) = CoreContext::current() else {
        return CoreContext::for_inspection(config.clone());
    };
    let mut overlay = crate::core::runtime::ContextOverlay::new(
        config.clone(),
        parent.domains(),
        parent.tool_groups(),
    );
    if !parent.user_skill_roots() {
        overlay = overlay.without_user_skill_roots();
    }
    parent.derive_for_inspection(overlay)
}

/// Shared safe diagnostic for a config parse failure. Keeps parser output and
/// source values out of the doctor report while preserving a typed field path
/// or TOML line when the parser supplied one.
pub(crate) fn config_parse_finding(config: &Config) -> Option<Finding> {
    let error = config.load_diagnostics().parse_error.as_ref()?;
    let mut finding = Finding::error(
        "config/parse",
        error.safe_summary(),
        "Correct the reported config syntax or field type and rerun doctor.",
    );
    finding.config_path = error.field_path.clone();
    Some(finding)
}

fn unusable_default_config_outcome(
    config: &Config,
    options: &DoctorOptions,
) -> Result<Option<Outcome<DoctorReport>>, String> {
    if config.load_diagnostics().fallback
        != crate::config::schema::load::ConfigLoadFallback::Defaults
    {
        return Ok(None);
    }
    let ctx = DoctorCtx::from_config(config.clone());
    let check_ids = super::registry::registered_checks_for(&ctx)?
        .iter()
        .map(|check| check.id().to_owned())
        .collect::<HashSet<_>>();
    let only = options.only.iter().cloned().collect::<HashSet<_>>();
    let skip = options.skip.iter().cloned().collect::<HashSet<_>>();
    let parse_selected =
        (only.is_empty() || only.contains("config/parse")) && !skip.contains("config/parse");
    let mut findings = if parse_selected {
        config_parse_finding(config).into_iter().collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let has_unknown_selector = only
        .iter()
        .chain(skip.iter())
        .any(|id| !check_ids.contains(id));
    if has_unknown_selector {
        findings.push(Finding::error(
            "doctor/selection",
            "One or more selected check ids are not registered.",
            "Use a registered exact check id from the doctor report.",
        ));
    }
    let summary = DoctorSummary::from_findings(&findings);
    let ok = summary.error == 0;
    let (layout_version, resolution_source) = config.doctor_profile_metadata();
    let report = DoctorReport {
        schema_version: 2,
        profile: super::profile::id_for_config(config),
        config_source: DoctorConfigSource::Profile,
        profile_layout_version: layout_version.map(|version| match version {
            crate::config::schema::profile_layout::LayoutVersion::V1 => 1,
            crate::config::schema::profile_layout::LayoutVersion::V2 => 2,
        }),
        config_resolution_source: resolution_source,
        tier: options.tier,
        checks_run: if parse_selected {
            vec!["config/parse".into()]
        } else {
            Vec::new()
        },
        checks_skipped: Vec::new(),
        summary,
        findings,
        ok,
    };
    Ok(Some(Outcome::single_log(
        report,
        "doctor config parse failed",
    )))
}

async fn report_with_context(
    options: DoctorOptions,
    ctx: DoctorCtx,
) -> Result<Outcome<DoctorReport>, String> {
    let report = super::runner::run(&ctx, options).await?;
    Ok(Outcome::single_log(report, "doctor report generated"))
}

fn profile_error_outcome(
    profile: Option<String>,
    tier: DoctorTier,
    error: super::profile::DesktopProfileError,
) -> Outcome<DoctorReport> {
    let findings = vec![Finding::error(
        "profile/exists",
        error.message(),
        "Select a supported desktop profile with a valid config and rerun doctor.",
    )];
    let summary = DoctorSummary::from_findings(&findings);
    Outcome::single_log(
        DoctorReport {
            schema_version: 2,
            profile,
            config_source: DoctorConfigSource::Profile,
            profile_layout_version: None,
            config_resolution_source: None,
            tier,
            checks_run: vec!["profile/exists".into()],
            checks_skipped: Vec::new(),
            findings,
            summary,
            ok: false,
        },
        "doctor profile selection failed",
    )
}

fn safe_profile_id(profile_id: &str) -> Option<String> {
    super::profile::is_profile_path_segment(profile_id).then(|| profile_id.to_owned())
}

pub async fn doctor_models(
    config: &Config,
    use_cache: bool,
) -> Result<Outcome<ModelProbeReport>, String> {
    Ok(Outcome::new(
        super::checks::inference_catalog::doctor_models(config, use_cache).await,
        Vec::new(),
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
