//! Which model answers, and where the request goes.
//!
//! # Why this is a per-turn route and not a config write
//!
//! The obvious way to point a harness at an endpoint — writing `inference_url` /
//! `api_key` / `cloud_providers` through the config — is wrong for a library:
//! those fields are **persisted**, so a caller borrowing an endpoint for its own
//! turns would repoint the operator's whole install, and a crash between the
//! write and the restore would leave it repointed for good.
//!
//! So [`Provider`] compiles down to the core's
//! [`EphemeralRoute`](crate::openhuman::config::schema::EphemeralRoute) — a
//! `#[serde(skip)]` field that has no place in `config.toml` to be saved into,
//! carried per call. The route pins only the four roles a turn actually runs on
//! (chat, reasoning, agentic, coding) and deliberately leaves the background
//! roles — memory, embeddings, heartbeat, learning — alone, because those run
//! tier-specific models a chat endpoint generally cannot serve.

use crate::embed::agent::Route;

/// Where a harness sends its inference.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Provider {
    route: Option<Route>,
    model: Option<String>,
}

impl Provider {
    /// Run on an OpenAI-compatible endpoint with the given bearer.
    ///
    /// `base_url` is spelled as a provider endpoint would be —
    /// `/chat/completions` is appended to it, so pass the API root
    /// (`https://host/v1`), not the completions path.
    ///
    /// Both halves are required: the core ignores a route with only one, and
    /// taking them together here means a partial route cannot be expressed.
    pub fn openai_compatible(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            route: Some(Route::openai_compatible(base_url, api_key)),
            model: None,
        }
    }

    /// Use whatever inference the machine's own configuration resolves to —
    /// the account's managed backend, or a local Ollama / LM Studio, or a BYOK
    /// provider.
    ///
    /// Pair with [`Workspace::Inherit`](super::Workspace) to run exactly as the
    /// installed app would.
    pub fn inherit() -> Self {
        Self::default()
    }

    /// Pin the model id.
    ///
    /// **Advisory, not enforced.** A model no configured provider serves is not
    /// an error — the core falls back to its default rather than failing — so do
    /// not use this as a guarantee about which weights answered. It is also
    /// load-bearing for a route: the route pins its roles to
    /// `"<slug>:<model>"`, and with no model resolved it registers nothing and
    /// logs that it ignored the route.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        let model = model.into();
        self.model = (!model.trim().is_empty()).then_some(model);
        self
    }

    /// The endpoint this provider routes to, if it is not inheriting.
    pub fn route(&self) -> Option<&Route> {
        self.route.as_ref()
    }

    /// The pinned model id, if any.
    pub fn model_id(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// Whether this provider states an endpoint of its own.
    pub fn is_routed(&self) -> bool {
        self.route.is_some()
    }

    /// Whether the route would survive [`EphemeralRoute::from_params`]'s
    /// normalization. Kept separate from [`Self::is_routed`], which describes
    /// the caller's syntactic choice for diagnostics.
    pub(super) fn has_usable_route(&self) -> bool {
        self.route.as_ref().is_some_and(|route| {
            !route.base_url.trim().is_empty() && !route.api_key.trim().is_empty()
        })
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
