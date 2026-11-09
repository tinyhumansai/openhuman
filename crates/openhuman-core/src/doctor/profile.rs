//! Read-only loading and existence diagnostics for desktop profiles.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;

use crate::config::schema::load::ConfigResolutionSource;
use crate::config::schema::profile_layout::LayoutVersion;
use crate::config::Config;
use crate::doctor::contract::{DoctorCheck, DoctorCtx, DoctorProfileStatus, DoctorTier, Finding};
use crate::profiles::ProfileLayout;

/// A desktop profile config loaded without activating or repairing it.
#[derive(Debug, Clone)]
pub(crate) struct DesktopProfile {
    pub id: String,
    pub layout: ProfileLayout,
    pub config: Config,
}

/// The effective desktop config and provenance resolved without initializing
/// or repairing a profile.
#[derive(Debug, Clone)]
pub struct DesktopProfileSelection {
    /// Effective config loaded for the selected profile or current desktop.
    pub config: Config,
    /// Profile id when the selected config lives under `users/<id>`.
    pub profile_id: Option<String>,
    /// Profile layout when the config belongs to a desktop profile.
    pub layout_version: Option<LayoutVersion>,
    /// Source that chose the config directory, or `--profile` for an explicit id.
    pub resolution_source: String,
    pub(crate) profile_status: Option<DoctorProfileStatus>,
}

/// Safe failure categories for a selected desktop profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesktopProfileError {
    InvalidId,
    Missing,
    UnknownLayoutMarker,
    InvalidConfig,
    Unavailable,
}

impl DesktopProfileError {
    pub fn message(self) -> &'static str {
        match self {
            Self::InvalidId => "The selected profile id is invalid.",
            Self::Missing => "The selected profile does not exist.",
            Self::UnknownLayoutMarker => "The selected profile has an unsupported layout marker.",
            Self::InvalidConfig => "The selected profile config is missing or invalid.",
            Self::Unavailable => "Profile selection is unavailable for this host.",
        }
    }
}

/// Resolves either an explicit desktop profile or the effective desktop
/// config using the normal directory precedence, without running
/// `ProfileLayout::resolve`, `Config::load_or_init`, or persistent migrations.
pub async fn resolve_desktop_profile_for_doctor(
    root: &Path,
    requested_profile: Option<&str>,
) -> Result<DesktopProfileSelection, DesktopProfileError> {
    if let Some(id) = requested_profile {
        let selected = load_desktop_profile(root, id).await?;
        let mut config = selected.config;
        config.set_doctor_profile_metadata(Some(selected.layout.version), Some("--profile".into()));
        return Ok(DesktopProfileSelection {
            config,
            profile_id: Some(selected.id),
            layout_version: Some(selected.layout.version),
            resolution_source: "--profile".into(),
            profile_status: Some(DoctorProfileStatus::Valid),
        });
    }

    let (config_dir, workspace_dir, source, profile_id, layout_version) =
        resolve_desktop_config_dirs_read_only(root).await?;
    let (mut config, config_exists) =
        load_config_read_only(&config_dir.join("config.toml"), &workspace_dir).await?;
    config.set_doctor_profile_metadata(layout_version, Some(source.as_str().into()));
    let profile_status = if profile_id.is_some() {
        if config.load_diagnostics().parse_error.is_some() {
            Some(DoctorProfileStatus::InvalidConfig)
        } else if config_exists {
            Some(DoctorProfileStatus::Valid)
        } else {
            match tokio::fs::symlink_metadata(&config_dir).await {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Some(DoctorProfileStatus::Missing)
                }
                _ => Some(DoctorProfileStatus::InvalidConfig),
            }
        }
    } else {
        None
    };
    Ok(DesktopProfileSelection {
        config,
        profile_status,
        profile_id,
        layout_version,
        resolution_source: source.as_str().into(),
    })
}

/// Loads one profile beneath `root` without changing active-user state,
/// creating directories, writing a layout marker, or recovering from a
/// backup. It applies the same in-memory load transforms as the regular
/// snapshot loader but does not run persistent schema migrations. Process
/// environment config overrides still apply after the selected profile file
/// is parsed, except that the workspace override cannot replace the selected
/// profile's workspace.
pub(crate) async fn load_desktop_profile(
    root: &Path,
    id: &str,
) -> Result<DesktopProfile, DesktopProfileError> {
    if !is_profile_path_segment(id) {
        return Err(DesktopProfileError::InvalidId);
    }

    let dir = ProfileLayout::new(root, id).dir;
    let metadata = tokio::fs::metadata(&dir).await.map_err(|error| {
        if matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
        ) {
            DesktopProfileError::Missing
        } else {
            DesktopProfileError::InvalidConfig
        }
    })?;
    if !metadata.is_dir() {
        return Err(DesktopProfileError::Missing);
    }

    let version = read_layout_version(&dir).await?;
    let layout = ProfileLayout::with_version(dir, version);
    let (config, config_exists) =
        load_config_read_only(&layout.config_path, &layout.workspace_dir).await?;
    if !config_exists {
        return Err(DesktopProfileError::InvalidConfig);
    }
    if config.load_diagnostics().parse_error.is_some() {
        return Err(DesktopProfileError::InvalidConfig);
    }

    Ok(DesktopProfile {
        id: id.to_owned(),
        layout,
        config,
    })
}

async fn load_config_read_only(
    config_path: &Path,
    workspace_dir: &Path,
) -> Result<(Config, bool), DesktopProfileError> {
    let config_text = match tokio::fs::metadata(config_path).await {
        Ok(metadata) if metadata.is_file() => Some(
            tokio::fs::read_to_string(config_path)
                .await
                .map_err(|_| DesktopProfileError::InvalidConfig)?,
        ),
        Ok(_) => return Err(DesktopProfileError::InvalidConfig),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => return Err(DesktopProfileError::InvalidConfig),
    };

    let mut config = match config_text.as_deref() {
        Some(contents) => match crate::config::schema::config_from_toml_str(contents) {
            Ok(config) => config,
            Err(error) => invalid_config_snapshot(
                config_path,
                workspace_dir,
                crate::config::schema::load::ConfigParseError::from_error(
                    config_path.to_path_buf(),
                    &error,
                ),
            ),
        },
        None => Config::default(),
    };
    config.config_path = config_path.to_path_buf();
    config.workspace_dir = workspace_dir.to_path_buf();
    config.action_dir = crate::config::resolve_action_dir(&config.action_dir_override);
    if let Some(contents) = config_text.as_deref() {
        if config.load_diagnostics().parse_error.is_none() {
            crate::config::schema::load::apply_loaded_config_migrations(&mut config, contents);
        }
    }
    config.apply_profile_env_overrides();
    if config.load_diagnostics().parse_error.is_none() {
        crate::config::enrichers::run_enrichers(&mut config);
    }
    Ok((config, config_text.is_some()))
}

fn invalid_config_snapshot(
    config_path: &Path,
    workspace_dir: &Path,
    parse_error: crate::config::schema::load::ConfigParseError,
) -> Config {
    let mut config = Config {
        config_path: config_path.to_path_buf(),
        workspace_dir: workspace_dir.to_path_buf(),
        ..Config::default()
    };
    let diagnostics = config.load_diagnostics_mut();
    diagnostics.parse_error = Some(parse_error);
    diagnostics.fallback = crate::config::schema::load::ConfigLoadFallback::Defaults;
    config
}

async fn profile_paths_for_resolved_dirs(
    config_dir: PathBuf,
    workspace_dir: PathBuf,
) -> Result<(PathBuf, PathBuf, Option<String>, Option<LayoutVersion>), DesktopProfileError> {
    if let Some(profile_id) = profile_id_for_dir(&config_dir)? {
        let layout = read_profile_layout(config_dir).await?;
        return Ok((
            layout.dir.clone(),
            layout.workspace_dir.clone(),
            Some(profile_id),
            Some(layout.version),
        ));
    }
    if workspace_dir
        .file_name()
        .is_some_and(|name| name == crate::config::schema::profile_layout::PROFILE_FILES_DIR)
    {
        if let Some(profile_dir) = workspace_dir.parent() {
            if let Some(profile_id) = profile_id_for_dir(profile_dir)? {
                let layout = read_profile_layout(profile_dir.to_path_buf()).await?;
                return Ok((
                    layout.dir.clone(),
                    layout.workspace_dir.clone(),
                    Some(profile_id),
                    Some(layout.version),
                ));
            }
        }
    }
    if let Some(layout) = ProfileLayout::of_workspace(&workspace_dir) {
        let id = profile_id_for_dir(&layout.dir)?;
        return Ok((
            layout.dir.clone(),
            layout.workspace_dir.clone(),
            id,
            Some(layout.version),
        ));
    }
    Ok((config_dir, workspace_dir, None, None))
}

async fn resolve_desktop_config_dirs_read_only(
    root: &Path,
) -> Result<
    (
        PathBuf,
        PathBuf,
        ConfigResolutionSource,
        Option<String>,
        Option<LayoutVersion>,
    ),
    DesktopProfileError,
> {
    if let Some(workspace) = std::env::var_os("OPENHUMAN_WORKSPACE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        let (config_dir, workspace_dir) =
            crate::config::resolve_config_dir_for_workspace(&workspace);
        let (config_dir, workspace_dir, profile_id, layout_version) =
            profile_paths_for_resolved_dirs(config_dir, workspace_dir).await?;
        return Ok((
            config_dir,
            workspace_dir,
            ConfigResolutionSource::EnvWorkspace,
            profile_id,
            layout_version,
        ));
    }

    if let Some(profile_id) = crate::config::schema::load::read_active_user_id_checked_async(root)
        .await
        .map_err(|_| DesktopProfileError::InvalidConfig)?
    {
        if !is_profile_path_segment(&profile_id) {
            return Err(DesktopProfileError::InvalidId);
        }
        let dir = ProfileLayout::new(root, &profile_id).dir;
        let layout = read_profile_layout(dir).await?;
        return Ok((
            layout.dir.clone(),
            layout.workspace_dir.clone(),
            ConfigResolutionSource::ActiveUser,
            Some(profile_id),
            Some(layout.version),
        ));
    }

    if let Some((config_dir, workspace_dir)) =
        crate::config::schema::load::load_persisted_workspace_dirs(root)
            .await
            .map_err(|_| DesktopProfileError::InvalidConfig)?
    {
        let (config_dir, workspace_dir, profile_id, layout_version) =
            profile_paths_for_resolved_dirs(config_dir, workspace_dir).await?;
        return Ok((
            config_dir,
            workspace_dir,
            ConfigResolutionSource::ActiveWorkspaceMarker,
            profile_id,
            layout_version,
        ));
    }

    let layout = read_profile_layout(crate::config::pre_login_user_dir(root)).await?;
    Ok((
        layout.dir.clone(),
        layout.workspace_dir.clone(),
        ConfigResolutionSource::DefaultConfigDir,
        Some(crate::config::PRE_LOGIN_USER_ID.to_string()),
        Some(layout.version),
    ))
}

async fn read_profile_layout(dir: PathBuf) -> Result<ProfileLayout, DesktopProfileError> {
    match tokio::fs::metadata(&dir).await {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(DesktopProfileError::Missing),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(DesktopProfileError::InvalidConfig),
    }
    let version = read_layout_version(&dir).await?;
    Ok(ProfileLayout::with_version(dir, version))
}

fn profile_id_for_dir(dir: &Path) -> Result<Option<String>, DesktopProfileError> {
    let Some(users_dir) = dir.parent() else {
        return Ok(None);
    };
    if !users_dir.file_name().is_some_and(|name| name == "users") {
        return Ok(None);
    }
    let Some(id) = dir.file_name().and_then(|name| name.to_str()) else {
        return Err(DesktopProfileError::InvalidId);
    };
    if !is_profile_path_segment(id) {
        return Err(DesktopProfileError::InvalidId);
    }
    Ok(Some(id.to_owned()))
}

/// Derives a selected profile's root from an already-resolved config path.
/// This does not consult the active-profile markers or environment.
pub(crate) fn root_for_config(config: &Config) -> Option<PathBuf> {
    root_for_config_path(&config.config_path)
}

pub(crate) fn root_for_config_path(config_path: &Path) -> Option<PathBuf> {
    let config_dir = config_path.parent()?;
    let users_dir = config_dir.parent()?;
    if users_dir.file_name().is_some_and(|name| name == "users") {
        return users_dir.parent().map(Path::to_path_buf);
    }
    Some(config_dir.to_path_buf())
}

/// Resolves the profile root implied by `OPENHUMAN_WORKSPACE` without loading
/// a config or consulting active-profile markers.
pub(crate) fn root_for_workspace_override() -> Option<PathBuf> {
    let workspace = std::env::var_os("OPENHUMAN_WORKSPACE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)?;
    let (config_dir, _) = crate::config::resolve_config_dir_for_workspace(&workspace);
    let config = Config {
        config_path: config_dir.join("config.toml"),
        ..Config::default()
    };
    root_for_config(&config)
}

/// Profile id represented by a config path, if it is in `<root>/users/<id>`.
pub(crate) fn id_for_config(config: &Config) -> Option<String> {
    let config_dir = config.config_path.parent()?;
    let users_dir = config_dir.parent()?;
    if !users_dir.file_name().is_some_and(|name| name == "users") {
        return None;
    }
    let id = config_dir.file_name()?.to_str()?;
    is_profile_path_segment(id).then(|| id.to_owned())
}

pub(crate) fn is_profile_path_segment(id: &str) -> bool {
    if id.is_empty() || id.chars().any(char::is_control) || id.contains(['/', '\\']) {
        return false;
    }
    let mut components = Path::new(id).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

async fn read_layout_version(dir: &Path) -> Result<LayoutVersion, DesktopProfileError> {
    let marker = dir.join(".layout");
    match tokio::fs::read_to_string(&marker).await {
        Ok(value) if value.trim() == "2" => Ok(LayoutVersion::V2),
        Ok(_) => Err(DesktopProfileError::UnknownLayoutMarker),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(LayoutVersion::detect(dir))
        }
        Err(_) => Err(DesktopProfileError::UnknownLayoutMarker),
    }
}

pub(crate) fn built_in_checks() -> Vec<Arc<dyn DoctorCheck>> {
    let mut checks: Vec<Arc<dyn DoctorCheck>> = vec![Arc::new(ProfileExistsCheck)];
    checks.extend(crate::doctor::checks::inference_catalog::built_in_checks());
    checks
}

struct ProfileExistsCheck;

#[async_trait]
impl DoctorCheck for ProfileExistsCheck {
    fn id(&self) -> &str {
        "profile/exists"
    }

    fn category(&self) -> &str {
        "profile"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Static
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        if ctx.config_source() == super::contract::DoctorConfigSource::Tenant {
            return vec![Finding::skipped(
                self.id(),
                "Tenant profile readiness is reported by the hosted profile checks.",
                "No action required.",
            )];
        }
        match ctx.profile_status() {
            Some(DoctorProfileStatus::Valid) => {
                vec![Finding::pass(
                    self.id(),
                    "The selected profile exists and its config is valid.",
                )]
            }
            Some(DoctorProfileStatus::Missing) => vec![Finding::error(
                self.id(),
                DesktopProfileError::Missing.message(),
                "Select an existing desktop profile and rerun doctor.",
            )],
            Some(DoctorProfileStatus::UnknownLayoutMarker) => vec![Finding::error(
                self.id(),
                DesktopProfileError::UnknownLayoutMarker.message(),
                "Use a profile with a supported layout marker and rerun doctor.",
            )],
            Some(DoctorProfileStatus::InvalidConfig) => vec![Finding::error(
                self.id(),
                DesktopProfileError::InvalidConfig.message(),
                "Repair the selected profile config and rerun doctor.",
            )],
            None => vec![Finding::skipped(
                self.id(),
                "No desktop profile was selected for this run.",
                "Pass --profile <id> to validate a specific profile.",
            )],
        }
    }
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
