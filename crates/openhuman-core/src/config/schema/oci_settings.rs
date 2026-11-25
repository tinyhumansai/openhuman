//! Non-secret settings for the OCI Generative AI cloud provider
//! ([`AuthStyle::Oci`](super::cloud_providers::AuthStyle::Oci)).
//!
//! These live on the `cloud_providers` entry (`CloudProviderCreds::oci`) and
//! are persisted in `config.toml`. The only secret an OCI entry can own is the
//! Generative AI API key used by [`OciAuthMode::ApiKey`]; it stays in the
//! credential store under `provider:<slug>`. Key material for the profile and
//! principal modes is never copied here: it is read from the OCI config
//! directory or the runtime environment when the model is built.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How requests to OCI Generative AI authenticate.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OciAuthMode {
    /// Generative AI API key (`Authorization: Bearer`), stored in the keyring.
    #[default]
    ApiKey,
    /// API-signing-key profile from the OCI config file (`~/.oci/config`).
    ConfigProfile,
    /// Session-token profile (`oci session authenticate`); token refreshes are
    /// picked up without a restart.
    SessionToken,
    /// OCI resource principal (Functions, Container Instances, ...).
    ResourcePrincipal,
    /// OCI instance principal (compute instances). Needs `oci-principals`.
    InstancePrincipal,
    /// OKE workload identity. Needs `oci-principals`.
    OkeWorkloadIdentity,
}

impl OciAuthMode {
    /// Stable wire / log name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::ConfigProfile => "config_profile",
            Self::SessionToken => "session_token",
            Self::ResourcePrincipal => "resource_principal",
            Self::InstancePrincipal => "instance_principal",
            Self::OkeWorkloadIdentity => "oke_workload_identity",
        }
    }

    /// Whether this mode needs a key stored in the credential store.
    pub fn needs_stored_key(&self) -> bool {
        matches!(self, Self::ApiKey)
    }

    /// Whether this mode reads a profile from the OCI config file.
    pub fn uses_config_file(&self) -> bool {
        matches!(self, Self::ConfigProfile | Self::SessionToken)
    }
}

/// Which wire protocol to speak to OCI Generative AI.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OciTransportKind {
    /// OpenAI-compatible `/openai/v1`.
    #[default]
    Openai,
    /// OpenAI-compatible `/20231130/actions/v1`.
    Actions,
    /// Native chat API, `apiFormat: GENERIC` (Meta, xAI, Google, ...).
    NativeGeneric,
    /// Native chat API, `apiFormat: COHERE`.
    NativeCohere,
}

impl OciTransportKind {
    /// Stable wire / log name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Actions => "actions",
            Self::NativeGeneric => "native_generic",
            Self::NativeCohere => "native_cohere",
        }
    }

    /// Whether the transport is the native chat API (requires a compartment).
    pub fn is_native(&self) -> bool {
        matches!(self, Self::NativeGeneric | Self::NativeCohere)
    }
}

/// OCI-specific settings for an `auth_style = "oci"` provider entry.
///
/// Every field is optional on the wire; the factory reports the first missing
/// required field by name when the model is built.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(default)]
pub struct OciProviderSettings {
    /// Authentication mode. Defaults to `api_key`.
    pub auth_mode: OciAuthMode,
    /// OCI config file for the profile modes. Defaults to `$OCI_CLI_CONFIG_FILE`
    /// or `~/.oci/config`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_file: Option<String>,
    /// Profile name inside the config file. Defaults to `DEFAULT`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// OCI region, e.g. `us-chicago-1`. Falls back to the profile's region.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    /// Compartment OCID. Required for every transport except `openai` with
    /// `api_key` auth, whose key already names one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compartment_id: Option<String>,
    /// Generative AI project OCID, sent as `OpenAI-Project` on `/openai/v1`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Wire protocol. Defaults to `openai`.
    pub transport: OciTransportKind,
    /// Dedicated AI cluster endpoint OCID; when set the model is addressed
    /// through it instead of on-demand by model id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedicated_endpoint_id: Option<String>,
}

#[cfg(test)]
#[path = "oci_settings_tests.rs"]
mod tests;
