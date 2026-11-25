//! OCI Generative AI cloud slug (`auth_style = "oci"`): maps the non-secret
//! [`OciProviderSettings`] on a `cloud_providers` entry, plus the optional
//! keyring API key, onto a crate-native [`OciConfig`] and builds the model.
//!
//! Secrets: only `api_key` mode carries one, and it arrives as the `key`
//! argument (read from the credential store by `lookup_key_for_slug`). The
//! profile modes read key material from the OCI config directory when the
//! model is built and never copy it into OpenHuman config. Nothing here logs
//! a key, a token, or a key-file's contents.

use super::*;
use crate::config::schema::cloud_providers::{
    builtin_cloud_provider, CloudProviderCreds, OciAuthMode, OciProviderSettings, OciTransportKind,
};
use std::path::PathBuf;
use tinyinference_llm::providers::oci::{
    build_oci_model, default_config_path, load_profile, OciApiFormat, OciAuth, OciConfig,
    OciOpenAiBase, OciServing, OciTransport,
};

/// Whether `entry` needs a key in the credential store to be usable.
///
/// `Bearer` / `Anthropic` always do; `Oci` only in `api_key` mode (an entry
/// with no `oci` block defaults to `api_key`). Drives the implicit-fallback
/// missing-key diagnostic in `resolve_cloud_slug`.
pub(super) fn entry_requires_stored_key(entry: &CloudProviderCreds) -> bool {
    match entry.auth_style {
        AuthStyle::Bearer | AuthStyle::Anthropic => true,
        AuthStyle::Oci => entry
            .oci
            .as_ref()
            .map_or(OciAuthMode::default(), |s| s.auth_mode)
            .needs_stored_key(),
        AuthStyle::OpenhumanJwt | AuthStyle::None => false,
    }
}

fn non_empty(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// Expands a leading `~`, `~/` or `~\` from `$HOME`, or `%USERPROFILE%` on
/// Windows, where `HOME` is usually unset. (Core code does not call
/// `dirs::home_dir()`: see `scripts/ci/check-saas-ambient.mjs`.)
fn expand_home(path: &str) -> PathBuf {
    let rest = path
        .strip_prefix("~/")
        .or_else(|| path.strip_prefix("~\\"))
        .or((path == "~").then_some(""));
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|home| !home.is_empty()))
        .map(PathBuf::from);
    match (rest, home) {
        (Some(""), Some(home)) => home,
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(path),
    }
}

fn config_path(slug: &str, settings: &OciProviderSettings) -> Result<PathBuf, String> {
    if let Some(path) = non_empty(&settings.config_file) {
        return Ok(expand_home(&path));
    }
    default_config_path().ok_or_else(|| {
        format!(
            "OCI provider '{slug}': cannot locate the OCI config file (no HOME); \
             set oci.config_file on the cloud_providers entry"
        )
    })
}

/// Resolves credentials for `settings.auth_mode` into an [`OciAuth`] plus the
/// region the credential implies (profile region), if any.
fn resolve_auth(
    slug: &str,
    settings: &OciProviderSettings,
    key: &str,
) -> Result<(OciAuth, Option<String>), String> {
    let mode = settings.auth_mode;
    match mode {
        OciAuthMode::ApiKey => {
            let key = key.trim();
            if key.is_empty() {
                return Err(format!(
                    "OCI provider '{slug}' (auth_mode=api_key) has no API key stored; \
                     add the OCI Generative AI API key for this provider in Settings, \
                     or switch oci.auth_mode to config_profile / session_token / a principal"
                ));
            }
            Ok((
                OciAuth::ApiKey {
                    bearer: key.to_string(),
                },
                None,
            ))
        }
        OciAuthMode::ConfigProfile | OciAuthMode::SessionToken => {
            let path = config_path(slug, settings)?;
            let profile_name =
                non_empty(&settings.profile).unwrap_or_else(|| "DEFAULT".to_string());
            log::debug!(
                "[providers][oci] loading profile slug={slug} auth_mode={} profile={profile_name} config_file={}",
                mode.as_str(),
                path.display()
            );
            let profile = load_profile(&path, &profile_name)
                .map_err(|e| format!("OCI provider '{slug}' (auth_mode={}): {e}", mode.as_str()))?;
            let is_session = matches!(profile.auth, OciAuth::SessionTokenFile { .. });
            match (mode, is_session) {
                (OciAuthMode::SessionToken, false) => Err(format!(
                    "OCI provider '{slug}': profile '{profile_name}' has no security_token_file, \
                     so it is not a session-token profile; run `oci session authenticate` or \
                     set oci.auth_mode to config_profile"
                )),
                (OciAuthMode::ConfigProfile, true) => Err(format!(
                    "OCI provider '{slug}': profile '{profile_name}' is a session-token profile; \
                     set oci.auth_mode to session_token"
                )),
                _ => Ok((profile.auth, profile.region)),
            }
        }
        OciAuthMode::ResourcePrincipal => Ok((OciAuth::ResourcePrincipal, None)),
        OciAuthMode::InstancePrincipal | OciAuthMode::OkeWorkloadIdentity => {
            #[cfg(not(feature = "oci-principals"))]
            {
                Err(format!(
                    "OCI provider '{slug}': auth_mode={} is not supported in this build; \
                     rebuild with the `oci-principals` cargo feature (needs OpenSSL), \
                     or use api_key / config_profile / session_token / resource_principal",
                    mode.as_str()
                ))
            }
            #[cfg(feature = "oci-principals")]
            {
                Ok((
                    if mode == OciAuthMode::InstancePrincipal {
                        OciAuth::InstancePrincipal
                    } else {
                        OciAuth::OkeWorkloadIdentity
                    },
                    None,
                ))
            }
        }
    }
}

/// Maps an entry's OCI settings (and key, for `api_key` mode) to an
/// [`OciConfig`], reporting the first missing field by name.
pub(super) fn oci_config_from_entry(
    slug: &str,
    settings: &OciProviderSettings,
    key: &str,
    model: &str,
) -> Result<OciConfig, String> {
    let transport = match settings.transport {
        OciTransportKind::Openai => OciTransport::OpenAiCompat {
            base: OciOpenAiBase::OpenAi,
        },
        OciTransportKind::Actions => OciTransport::OpenAiCompat {
            base: OciOpenAiBase::Actions,
        },
        OciTransportKind::NativeGeneric => OciTransport::Native {
            api_format: OciApiFormat::Generic,
        },
        OciTransportKind::NativeCohere => OciTransport::Native {
            api_format: OciApiFormat::Cohere,
        },
    };
    let compartment_id = non_empty(&settings.compartment_id);
    // A GenAI API key carries its own compartment; IAM-signed calls must name
    // one on every transport, `/openai/v1` included.
    let needs_compartment =
        settings.transport != OciTransportKind::Openai || settings.auth_mode != OciAuthMode::ApiKey;
    if needs_compartment && compartment_id.is_none() {
        return Err(format!(
            "OCI provider '{slug}': transport '{}' with auth_mode={} requires oci.compartment_id (the compartment OCID)",
            settings.transport.as_str(),
            settings.auth_mode.as_str()
        ));
    }

    // OCI's OpenAI-compatible `/openai/v1` base serves requests in a
    // Generative AI project, sent as `OpenAI-Project`.
    let project_id = non_empty(&settings.project_id);
    if settings.transport == OciTransportKind::Openai && project_id.is_none() {
        return Err(format!(
            "OCI provider '{slug}': transport 'openai' requires oci.project_id (the Generative AI project OCID)"
        ));
    }

    let (auth, profile_region) = resolve_auth(slug, settings, key)?;
    let region = non_empty(&settings.region).or(profile_region);
    if region.is_none() && matches!(settings.auth_mode, OciAuthMode::ApiKey) {
        return Err(format!(
            "OCI provider '{slug}' (auth_mode=api_key) requires oci.region (e.g. us-chicago-1)"
        ));
    }

    let serving = match non_empty(&settings.dedicated_endpoint_id) {
        Some(endpoint_id) => OciServing::Dedicated { endpoint_id },
        None => OciServing::OnDemand,
    };

    let mut config = OciConfig::new(auth, model);
    config.region = region;
    config.transport = transport;
    config.compartment_id = compartment_id;
    config.project_id = project_id;
    config.serving = serving;
    Ok(config)
}

/// Applies the entry's `endpoint` as the service host override: other OCI
/// realms, a private endpoint, or a local capture proxy (loopback http only).
/// A blank endpoint keeps the region-derived host, and so does the built-in
/// preset's URL: it is a display default (the `us-chicago-1` host), and
/// honouring it would send every region's requests to Chicago.
fn with_entry_endpoint(slug: &str, mut config: OciConfig, entry: &CloudProviderCreds) -> OciConfig {
    let preset = builtin_cloud_provider("oci").map(|preset| preset.endpoint.trim_end_matches('/'));
    config.endpoint = non_empty(&Some(entry.endpoint.clone()))
        .filter(|endpoint| Some(endpoint.trim_end_matches('/')) != preset);
    if config.endpoint.is_some() {
        log::debug!("[providers][oci] slug={slug} using the entry endpoint override");
    }
    config
}

/// The complete [`OciConfig`] for an entry: its settings, the endpoint
/// override and the route's temperature override.
fn entry_oci_config(
    slug: &str,
    entry: &CloudProviderCreds,
    settings: &OciProviderSettings,
    key: &str,
    model: &str,
    temperature_override: Option<f64>,
) -> Result<OciConfig, String> {
    let mut config = with_entry_endpoint(
        slug,
        oci_config_from_entry(slug, settings, key, model)?,
        entry,
    );
    config.temperature_override = temperature_override;
    Ok(config)
}

/// Builds the OCI chat model for a resolved `oci` cloud slug, with the
/// service endpoint it calls (reported as the route's wire identity).
/// `temperature_override` is the route's `@<temperature>` suffix, which wins
/// over the request's temperature as on every other cloud slug.
pub(super) fn build_oci_chat_model(
    slug: &str,
    entry: &CloudProviderCreds,
    key: &str,
    model: &str,
    temperature_override: Option<f64>,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    let settings = entry.oci.clone().unwrap_or_default();
    log::debug!(
        "[providers][oci] building slug={slug} model={model} auth_mode={} transport={} dedicated={}",
        settings.auth_mode.as_str(),
        settings.transport.as_str(),
        settings.dedicated_endpoint_id.is_some()
    );
    let config = entry_oci_config(slug, entry, &settings, key, model, temperature_override)
        .map_err(anyhow::Error::msg)?;
    let endpoint = config
        .service_endpoint()
        .map_err(|e| anyhow::anyhow!("[chat-factory] OCI provider '{slug}': {e}"))?;
    build_oci_model(config)
        .map(|chat| (chat, endpoint))
        .map_err(|e| {
            log::warn!(
                "[providers][oci] build failed slug={slug} auth_mode={} transport={}",
                settings.auth_mode.as_str(),
                settings.transport.as_str()
            );
            anyhow::anyhow!(
                "[chat-factory] OCI provider '{slug}' (auth_mode={}, transport={}): {e}",
                settings.auth_mode.as_str(),
                settings.transport.as_str()
            )
        })
}

#[cfg(test)]
#[path = "oci_tests.rs"]
mod tests;
