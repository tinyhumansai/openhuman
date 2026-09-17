//! Typed capability vocabulary and plan validation for primary tool orchestration.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::tools::traits::PermissionLevel;

/// Closed vocabulary of primary tool operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityOperation {
    SearchWeb,
    FetchUrl,
    RetrieveImage,
    GenerateImage,
    RecallMemory,
    StoreMemory,
    ReadWorkspace,
    WriteWorkspace,
    ExecuteCommand,
    Schedule,
    Integration,
    Delegate,
}

/// Modality boundaries handled by primary tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityModality {
    Text,
    WebPage,
    Image,
    Audio,
    Video,
    File,
    Code,
    Memory,
    Schedule,
    Integration,
}

/// Execution backend providing the capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityBackend {
    Local,
    LocalBrowser,
    DirectNetwork,
    Byok,
    Managed,
}

impl CapabilityBackend {
    pub fn backend_rank(self) -> u8 {
        match self {
            Self::Local => 0,
            Self::LocalBrowser => 1,
            Self::DirectNetwork => 2,
            Self::Byok => 3,
            Self::Managed => 4,
        }
    }
}

/// Monetary boundary classification for capability consumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MonetaryBoundary {
    NonMetered,
    UserSuppliedKey,
    ManagedMetered,
}

/// Side-effect risk classification for capability invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySideEffect {
    None,
    LocalRead,
    ExternalRead,
    LocalWrite,
    ExternalWrite,
}

/// Current availability state of a tool capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAvailability {
    Available,
    Unconfigured,
    Unhealthy,
    Disabled,
}

/// Closed typed capability declared by a tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCapability {
    pub name: String,
    pub operations: Vec<CapabilityOperation>,
    pub modalities: Vec<CapabilityModality>,
    pub backend: CapabilityBackend,
    pub monetary_boundary: MonetaryBoundary,
    pub side_effect: CapabilitySideEffect,
    pub availability: CapabilityAvailability,
    pub permission: PermissionLevel,
    pub priority: u16,
    pub registration_index: usize,
}

impl ToolCapability {
    pub fn validate(&self) -> Result<(), CapabilityValidationError> {
        if self.name.trim().is_empty() {
            return Err(CapabilityValidationError::BlankName);
        }
        if self.operations.is_empty() {
            return Err(CapabilityValidationError::EmptyOperations);
        }
        if self.modalities.is_empty() {
            return Err(CapabilityValidationError::EmptyModalities);
        }
        if !is_backend_monetary_compatible(self.backend, self.monetary_boundary) {
            return Err(CapabilityValidationError::BackendMonetaryMismatch);
        }
        Ok(())
    }
}

/// Ranked routing entry binding a capability to execution order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRoute {
    pub capability: ToolCapability,
}

impl ToolRoute {
    pub fn new(capability: ToolCapability) -> Self {
        Self { capability }
    }

    pub fn backend_rank(&self) -> u8 {
        self.capability.backend.backend_rank()
    }

    pub fn sort_key(&self) -> (u8, u16, usize) {
        (
            self.backend_rank(),
            self.capability.priority,
            self.capability.registration_index,
        )
    }

    pub fn validate(&self) -> Result<(), CapabilityValidationError> {
        self.capability.validate()
    }
}

impl std::ops::Deref for ToolRoute {
    type Target = ToolCapability;

    fn deref(&self) -> &Self::Target {
        &self.capability
    }
}

impl From<ToolCapability> for ToolRoute {
    fn from(capability: ToolCapability) -> Self {
        Self { capability }
    }
}

/// Autonomous execution policy governing permitted capability properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPolicy {
    #[serde(alias = "maximum_permission")]
    pub max_permission: PermissionLevel,
    #[serde(alias = "managed_metered")]
    pub allow_managed_metered: bool,
    #[serde(alias = "byok")]
    pub allow_byok: bool,
    #[serde(alias = "external_network")]
    pub allow_external_network: bool,
}

impl CapabilityPolicy {
    pub fn new(
        max_permission: PermissionLevel,
        allow_managed_metered: bool,
        allow_byok: bool,
        allow_external_network: bool,
    ) -> Self {
        Self {
            max_permission,
            allow_managed_metered,
            allow_byok,
            allow_external_network,
        }
    }

    pub fn maximum_permission(&self) -> PermissionLevel {
        self.max_permission
    }

    pub fn allows(&self, capability: &ToolCapability) -> bool {
        if capability.permission > self.max_permission {
            return false;
        }
        if capability.monetary_boundary == MonetaryBoundary::ManagedMetered
            && !self.allow_managed_metered
        {
            return false;
        }
        if (capability.backend == CapabilityBackend::Byok
            || capability.monetary_boundary == MonetaryBoundary::UserSuppliedKey)
            && !self.allow_byok
        {
            return false;
        }
        if (capability.backend == CapabilityBackend::DirectNetwork
            || capability.side_effect == CapabilitySideEffect::ExternalRead
            || capability.side_effect == CapabilitySideEffect::ExternalWrite)
            && !self.allow_external_network
        {
            return false;
        }
        true
    }
}

/// Validated execution plan specifying ordered routes and enabled tool names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityPlan {
    pub routes: Vec<ToolRoute>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unavailable_reason: Option<String>,
}

impl CapabilityPlan {
    pub fn new(
        routes: Vec<ToolRoute>,
        unavailable_reason: Option<String>,
    ) -> Result<Self, CapabilityValidationError> {
        validate_routes(&routes)?;
        Ok(Self {
            routes,
            unavailable_reason,
        })
    }

    pub fn enabled_names(&self) -> HashSet<String> {
        Self::derive_enabled_names(&self.routes)
    }

    pub fn derive_enabled_names(routes: &[ToolRoute]) -> HashSet<String> {
        routes
            .iter()
            .filter(|route| route.capability.availability == CapabilityAvailability::Available)
            .map(|route| route.capability.name.clone())
            .collect()
    }

    pub fn validate(&self) -> Result<(), CapabilityValidationError> {
        validate_routes(&self.routes)
    }

    pub fn is_enabled(&self, name: &str) -> bool {
        self.routes.iter().any(|route| {
            route.capability.availability == CapabilityAvailability::Available
                && route.capability.name == name
        })
    }
}

/// Validation errors raised when checking capability declarations or routes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityValidationError {
    BlankName,
    EmptyOperations,
    EmptyModalities,
    DuplicateName(String),
    BackendMonetaryMismatch,
    UnsortedRoutes,
}

impl std::fmt::Display for CapabilityValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BlankName => write!(f, "capability name cannot be blank"),
            Self::EmptyOperations => write!(f, "capability operations cannot be empty"),
            Self::EmptyModalities => write!(f, "capability modalities cannot be empty"),
            Self::DuplicateName(name) => write!(f, "duplicate capability name '{name}'"),
            Self::BackendMonetaryMismatch => {
                write!(f, "backend and monetary boundary mismatch")
            }
            Self::UnsortedRoutes => write!(
                f,
                "routes must be sorted by (backend rank, priority, registration index)"
            ),
        }
    }
}

impl std::error::Error for CapabilityValidationError {}

/// Check if a capability backend matches its monetary boundary.
pub fn is_backend_monetary_compatible(
    backend: CapabilityBackend,
    monetary_boundary: MonetaryBoundary,
) -> bool {
    matches!(
        (backend, monetary_boundary),
        (CapabilityBackend::Managed, MonetaryBoundary::ManagedMetered)
            | (CapabilityBackend::Byok, MonetaryBoundary::UserSuppliedKey)
            | (CapabilityBackend::Local, MonetaryBoundary::NonMetered)
            | (
                CapabilityBackend::LocalBrowser,
                MonetaryBoundary::NonMetered
            )
            | (
                CapabilityBackend::DirectNetwork,
                MonetaryBoundary::NonMetered
            )
    )
}

/// Validate ordered routes: checking capability validity, sorting, and uniqueness.
pub fn validate_routes(routes: &[ToolRoute]) -> Result<(), CapabilityValidationError> {
    for route in routes {
        route.capability.validate()?;
    }

    for window in routes.windows(2) {
        if window[0].sort_key() > window[1].sort_key() {
            return Err(CapabilityValidationError::UnsortedRoutes);
        }
    }

    let mut seen_names = HashSet::with_capacity(routes.len());
    for route in routes {
        if !seen_names.insert(route.capability.name.as_str()) {
            return Err(CapabilityValidationError::DuplicateName(
                route.capability.name.clone(),
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
#[path = "capability_tests.rs"]
mod tests;
