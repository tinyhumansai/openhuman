//! Public types for doctor checks and their versioned report.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::config::schema::load::ProvenanceSource;
use crate::config::schema::profile_layout::LayoutVersion;
use crate::config::Config;
use crate::core::runtime::{ComponentSet, CoreContext};

/// The maximum cost a caller permits for a doctor run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorTier {
    /// Local configuration and filesystem inspection.
    Static,
    /// Bounded, read-only connectivity checks. This is the default tier.
    Connect,
    /// Opt-in checks that can incur a cost or write disposable test data.
    Live,
}

impl DoctorTier {
    pub(crate) const fn timeout(self) -> Duration {
        match self {
            Self::Static => Duration::from_secs(5),
            Self::Connect => Duration::from_secs(10),
            Self::Live => Duration::from_secs(60),
        }
    }

    pub(crate) const fn includes(self, check_tier: Self) -> bool {
        match self {
            Self::Static => matches!(check_tier, Self::Static),
            Self::Connect => matches!(check_tier, Self::Static | Self::Connect),
            Self::Live => true,
        }
    }
}

/// A component whose disabled state prevents a check from running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentId {
    Flows,
    Tinyflows,
    Memes,
    Documents,
    Search,
    Mcp,
    Connectors,
    Hosting,
    Juice,
    Memory,
    Wallet,
    Computer,
    Sandbox,
    Channels,
    Skills,
    Voice,
    Eventqueues,
}

impl ComponentId {
    pub(crate) fn enabled(self, components: &ComponentSet) -> bool {
        match self {
            Self::Flows => components.flows,
            Self::Tinyflows => components.tinyflows,
            Self::Memes => components.memes,
            Self::Documents => components.documents,
            Self::Search => components.search,
            Self::Mcp => components.mcp,
            Self::Connectors => components.connectors,
            Self::Hosting => components.hosting,
            Self::Juice => components.juice,
            Self::Memory => components.memory,
            Self::Wallet => components.wallet,
            Self::Computer => components.computer,
            Self::Sandbox => components.sandbox,
            Self::Channels => components.channels,
            Self::Skills => components.skills,
            Self::Voice => components.voice,
            Self::Eventqueues => components.eventqueues,
        }
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Flows => "flows",
            Self::Tinyflows => "tinyflows",
            Self::Memes => "memes",
            Self::Documents => "documents",
            Self::Search => "search",
            Self::Mcp => "mcp",
            Self::Connectors => "connectors",
            Self::Hosting => "hosting",
            Self::Juice => "juice",
            Self::Memory => "memory",
            Self::Wallet => "wallet",
            Self::Computer => "computer",
            Self::Sandbox => "sandbox",
            Self::Channels => "channels",
            Self::Skills => "skills",
            Self::Voice => "voice",
            Self::Eventqueues => "eventqueues",
        }
    }
}

/// Finding severity in the v2 wire contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Pass,
    Info,
    Warn,
    Error,
    Skipped,
}

/// The non-secret origin of a resolved configuration value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueSource {
    Cli,
    Env,
    Dotenv,
    File,
    Default,
}

/// The redacted source kind for the effective configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorConfigSource {
    Profile,
    Tenant,
}

/// Result of validating a selected desktop profile before other checks run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DoctorProfileStatus {
    Valid,
    Missing,
    UnknownLayoutMarker,
    InvalidConfig,
}

/// One operator-facing result from a check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub check_id: String,
    pub severity: FindingSeverity,
    pub message: String,
    pub fix_hint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_source: Option<ValueSource>,
    pub duration_ms: u64,
    pub affects: Vec<String>,
}

impl Finding {
    pub fn new(
        check_id: impl Into<String>,
        severity: FindingSeverity,
        message: impl Into<String>,
        fix_hint: impl Into<String>,
    ) -> Self {
        Self {
            check_id: check_id.into(),
            severity,
            message: message.into(),
            fix_hint: fix_hint.into(),
            config_path: None,
            value_source: None,
            duration_ms: 0,
            affects: Vec::new(),
        }
    }

    pub fn pass(check_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(
            check_id,
            FindingSeverity::Pass,
            message,
            "No action required.",
        )
    }

    pub fn skipped(
        check_id: impl Into<String>,
        message: impl Into<String>,
        fix_hint: impl Into<String>,
    ) -> Self {
        Self::new(check_id, FindingSeverity::Skipped, message, fix_hint)
    }

    pub fn error(
        check_id: impl Into<String>,
        message: impl Into<String>,
        fix_hint: impl Into<String>,
    ) -> Self {
        Self::new(check_id, FindingSeverity::Error, message, fix_hint)
    }
}

/// Counts findings by v2 severity.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorSummary {
    pub pass: usize,
    pub info: usize,
    pub warn: usize,
    pub error: usize,
    pub skipped: usize,
}

impl DoctorSummary {
    pub(crate) fn from_findings(findings: &[Finding]) -> Self {
        let mut summary = Self::default();
        for finding in findings {
            match finding.severity {
                FindingSeverity::Pass => summary.pass += 1,
                FindingSeverity::Info => summary.info += 1,
                FindingSeverity::Warn => summary.warn += 1,
                FindingSeverity::Error => summary.error += 1,
                FindingSeverity::Skipped => summary.skipped += 1,
            }
        }
        summary
    }
}

/// Versioned, ordered report shared by RPC and every host surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    pub schema_version: u8,
    pub profile: Option<String>,
    pub config_source: DoctorConfigSource,
    /// On-disk desktop profile layout, when the selected context has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_layout_version: Option<u8>,
    /// Stable source label that selected the config directory, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_resolution_source: Option<String>,
    pub tier: DoctorTier,
    pub checks_run: Vec<String>,
    pub checks_skipped: Vec<String>,
    pub findings: Vec<Finding>,
    pub summary: DoctorSummary,
    pub ok: bool,
}

/// Inputs shared by selected checks in one doctor run.
#[derive(Debug, Clone)]
pub struct DoctorCtx {
    config: Arc<Config>,
    components: ComponentSet,
    profile: Option<String>,
    config_source: DoctorConfigSource,
    profile_layout_version: Option<LayoutVersion>,
    profile_resolution_source: Option<String>,
    profile_status: Option<DoctorProfileStatus>,
}

impl DoctorCtx {
    /// Creates a check context from the effective config and active core
    /// context. An unscoped local configuration reports `profile` as its
    /// source; a scoped SaaS context reports `tenant`.
    pub fn from_config(config: Config) -> Self {
        let (profile_layout_version, profile_resolution_source) = config.doctor_profile_metadata();
        let core = CoreContext::current();
        let profile = core
            .as_ref()
            .and_then(|context| context.profile().map(str::to_owned));
        let config_source = if profile.is_some() {
            DoctorConfigSource::Tenant
        } else {
            DoctorConfigSource::Profile
        };
        let components = core
            .as_ref()
            .map_or_else(ComponentSet::all, |context| context.components());
        Self {
            config: Arc::new(config),
            components,
            profile,
            config_source,
            profile_layout_version,
            profile_resolution_source,
            profile_status: None,
        }
    }

    /// Creates a context from a config and explicit component set without
    /// consulting the ambient [`CoreContext`]. Use this when the caller has
    /// already selected and loaded the profile context.
    pub fn from_config_with_components(config: Config, components: ComponentSet) -> Self {
        let (profile_layout_version, profile_resolution_source) = config.doctor_profile_metadata();
        Self {
            config: Arc::new(config),
            components,
            profile: None,
            config_source: DoctorConfigSource::Profile,
            profile_layout_version,
            profile_resolution_source,
            profile_status: None,
        }
    }

    /// Pins this run to an explicit profile or tenant id and source.
    pub fn with_profile(mut self, profile: impl Into<String>, source: DoctorConfigSource) -> Self {
        self.profile = Some(profile.into());
        self.config_source = source;
        self
    }

    /// Adds profile layout and directory-resolution provenance for the
    /// `config/source` check.
    pub fn with_profile_metadata(
        mut self,
        layout_version: Option<LayoutVersion>,
        resolution_source: Option<String>,
    ) -> Self {
        self.profile_layout_version = layout_version;
        self.profile_resolution_source = resolution_source;
        self
    }

    pub(crate) fn with_profile_status(mut self, status: DoctorProfileStatus) -> Self {
        self.profile_status = Some(status);
        self
    }

    pub fn with_components(mut self, components: ComponentSet) -> Self {
        self.components = components;
        self
    }

    /// The effective configuration inspected by this run.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Returns the non-secret source for a logical config field path.
    ///
    /// Array indexes are normalized to the loader's `[]` notation. Filesystem
    /// paths are rejected because they identify a source location, not a
    /// resolved config value.
    pub(crate) fn value_source_for(&self, logical_path: &str) -> Option<ValueSource> {
        let path = normalize_logical_config_path(logical_path)?;
        let source = self.config.load_provenance().field(&path).source;
        Some(match source {
            ProvenanceSource::Cli => ValueSource::Cli,
            ProvenanceSource::Env => ValueSource::Env,
            ProvenanceSource::Dotenv => ValueSource::Dotenv,
            ProvenanceSource::File => ValueSource::File,
            ProvenanceSource::Default => ValueSource::Default,
        })
    }

    /// The component switches for this run.
    pub fn components(&self) -> ComponentSet {
        self.components
    }

    /// The effective profile or tenant id, when the active context has one.
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    /// On-disk profile layout selected for this run, when applicable.
    pub fn profile_layout_version(&self) -> Option<LayoutVersion> {
        self.profile_layout_version
    }

    /// Directory-selection provenance for the effective profile, when known.
    pub fn profile_resolution_source(&self) -> Option<&str> {
        self.profile_resolution_source.as_deref()
    }

    pub(crate) fn config_source(&self) -> DoctorConfigSource {
        self.config_source
    }

    pub(crate) fn profile_status(&self) -> Option<DoctorProfileStatus> {
        self.profile_status
    }
}

fn normalize_logical_config_path(path: &str) -> Option<String> {
    if path.is_empty()
        || path.starts_with('.')
        || path.contains('/')
        || path.contains('\\')
        || path.ends_with(".toml")
        || path.ends_with(".env")
    {
        return None;
    }

    let mut normalized = String::with_capacity(path.len());
    let mut chars = path.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '[' => {
                while chars.peek().is_some_and(char::is_ascii_digit) {
                    chars.next();
                }
                if chars.next()? != ']' {
                    return None;
                }
                normalized.push_str("[]");
            }
            ']' => return None,
            character
                if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') =>
            {
                normalized.push(character);
            }
            _ => return None,
        }
    }

    if normalized.starts_with('.') || normalized.ends_with('.') || normalized.contains("..") {
        return None;
    }
    Some(normalized)
}

/// Exact-id and tier filters for one report run.
#[derive(Debug, Clone)]
pub struct DoctorOptions {
    pub tier: DoctorTier,
    pub only: Vec<String>,
    pub skip: Vec<String>,
    /// Reserved for the profile loaders in #490/#491. The framework accepts
    /// the wire input; the host must resolve it before constructing `DoctorCtx`.
    pub profile: Option<String>,
}

impl Default for DoctorOptions {
    fn default() -> Self {
        Self {
            tier: DoctorTier::Connect,
            only: Vec::new(),
            skip: Vec::new(),
            profile: None,
        }
    }
}

/// A single registered doctor check.
#[async_trait]
pub trait DoctorCheck: Send + Sync {
    /// Stable lowercase `area/name` identifier.
    fn id(&self) -> &str;
    /// Display category for grouping in human-facing surfaces.
    fn category(&self) -> &str;
    /// Cost tier of this detector.
    fn tier(&self) -> DoctorTier;
    /// Component that must be enabled before detection can run.
    fn component_gate(&self) -> Option<ComponentId> {
        None
    }
    /// Per-check timeout. Defaults to the tier budget.
    fn timeout(&self) -> Duration {
        self.tier().timeout()
    }
    /// Whether this detector has a synchronous body that belongs on the
    /// blocking pool. Implement `detect_blocking` when this returns true.
    fn is_blocking(&self) -> bool {
        false
    }
    /// Run an asynchronous check.
    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding>;
    /// Run a blocking check on a Tokio blocking worker.
    fn detect_blocking(&self, _ctx: &DoctorCtx) -> Option<Vec<Finding>> {
        None
    }
}
