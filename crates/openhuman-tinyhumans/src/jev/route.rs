//! Which Jev endpoint the ranker talks to, and the credential for it.
//!
//! Jev is reachable three ways: the TinyHumans backend's proxy (a TinyHumans
//! credential), TypeSafe's own API (`TYPESAFE_API_KEY`), or OpenRouter's System
//! One API (an OpenRouter key). The route is an operator decision
//! (`agent.tool_search.jev_route`), because "use whichever credential is lying
//! around" would silently send a benchmark's — or a BYOK user's — decision
//! calls somewhere they did not choose. `auto` keeps the historical order and
//! only reaches for the other two when there is no TinyHumans credential.

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::credentials::backend_credential::resolve_backend_credential;
use openhuman_embed::__host::inference::provider::factory::lookup_key_for_slug;
use tinyjevclient::ClientConfig;
use tinytools::RankError;

use crate::backend::url::effective_backend_api_url;

/// Env var holding a direct TypeSafe API key.
pub const TYPESAFE_API_KEY_ENV: &str = "TYPESAFE_API_KEY";
/// Env var holding an OpenRouter API key. The stored `openrouter` BYOK key is
/// the fallback when it is unset.
pub const OPENROUTER_API_KEY_ENV: &str = "OPENROUTER_API_KEY";
/// `lookup_key_for_slug` slug of the user's OpenRouter BYOK provider.
const OPENROUTER_SLUG: &str = "openrouter";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JevRoute {
    Auto,
    TinyHumans,
    TypeSafe,
    OpenRouter,
}

impl JevRoute {
    /// Parses `agent.tool_search.jev_route`. An unknown spelling is `Auto` with
    /// a warning rather than an error: a typo must not turn off tool ranking.
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Self::Auto,
            "tinyhumans" => Self::TinyHumans,
            "typesafe" => Self::TypeSafe,
            "openrouter" => Self::OpenRouter,
            other => {
                log::warn!("[tool-search] unknown jev_route `{other}`; using `auto`");
                Self::Auto
            }
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::TinyHumans => "tinyhumans",
            Self::TypeSafe => "typesafe",
            Self::OpenRouter => "openrouter",
        }
    }
}

/// A route resolved to a ready client configuration.
pub struct ResolvedRoute {
    /// Which concrete route won (never `Auto`).
    pub route: JevRoute,
    pub client: ClientConfig,
    /// The secret the client was built with, for the client cache fingerprint
    /// only. Never logged.
    pub secret: String,
}

/// Source of the two env-provided keys. A seam so tests do not touch the
/// process environment.
pub type EnvLookup<'a> = &'a (dyn Fn(&str) -> Option<String> + Send + Sync);

pub fn process_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

async fn tinyhumans(config: &Config) -> Result<ResolvedRoute, String> {
    let credential = resolve_backend_credential(config)
        .await
        .map_err(|reason| format!("no TinyHumans credential ({reason})"))?;
    let secret = credential.into_secret();
    let mut client = ClientConfig::tinyhumans_openrouter(secret.clone());
    client.base_url = effective_backend_api_url(&config.api_url);
    Ok(ResolvedRoute {
        route: JevRoute::TinyHumans,
        client,
        secret,
    })
}

fn typesafe(config: &Config, env: EnvLookup<'_>) -> Result<ResolvedRoute, String> {
    let secret = env(TYPESAFE_API_KEY_ENV)
        .ok_or_else(|| format!("no TypeSafe key ({TYPESAFE_API_KEY_ENV} is unset)"))?;
    let mut client = ClientConfig::new(secret.clone());
    if let Some(base) = base_override(config) {
        client.base_url = base;
    }
    Ok(ResolvedRoute {
        route: JevRoute::TypeSafe,
        client,
        secret,
    })
}

async fn openrouter(config: &Config, env: EnvLookup<'_>) -> Result<ResolvedRoute, String> {
    let secret = match env(OPENROUTER_API_KEY_ENV) {
        Some(secret) => Some(secret),
        None => lookup_key_for_slug(OPENROUTER_SLUG, config)
            .await
            .ok()
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty()),
    }
    .ok_or_else(|| {
            format!("no OpenRouter key ({OPENROUTER_API_KEY_ENV} is unset and no `openrouter` provider key is stored)")
        })?;
    let mut client = ClientConfig::openrouter(secret.clone());
    if let Some(base) = base_override(config) {
        client.base_url = base;
    }
    Ok(ResolvedRoute {
        route: JevRoute::OpenRouter,
        client,
        secret,
    })
}

fn base_override(config: &Config) -> Option<String> {
    config
        .agent
        .tool_search
        .jev_base_url
        .as_deref()
        .map(|b| b.trim().trim_end_matches('/').to_owned())
        .filter(|b| !b.is_empty())
}

/// Resolve the configured route. The error names every gap (never a secret) so
/// the BM25 fallback's log line says what to set.
pub async fn resolve(config: &Config, env: EnvLookup<'_>) -> Result<ResolvedRoute, RankError> {
    let requested = JevRoute::parse(&config.agent.tool_search.jev_route);
    let outcome = match requested {
        JevRoute::TinyHumans => tinyhumans(config).await,
        JevRoute::TypeSafe => typesafe(config, env),
        JevRoute::OpenRouter => openrouter(config, env).await,
        JevRoute::Auto => match tinyhumans(config).await {
            Ok(route) => Ok(route),
            Err(th) => match typesafe(config, env) {
                Ok(route) => Ok(route),
                Err(ts) => openrouter(config, env)
                    .await
                    .map_err(|or| format!("{th}; {ts}; {or}")),
            },
        },
    };
    outcome.map_err(|reason| RankError::Backend {
        reason: format!("jev route `{}` unavailable: {reason}", requested.label()),
    })
}

#[cfg(test)]
#[path = "route_tests.rs"]
mod tests;
