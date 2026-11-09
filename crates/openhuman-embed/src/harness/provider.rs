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
//! [`EphemeralRoute`](openhuman_core::config::schema::EphemeralRoute) — a
//! `#[serde(skip)]` field that has no place in `config.toml` to be saved into,
//! carried per call. The route pins only the four roles a turn actually runs on
//! (chat, reasoning, agentic, coding) and deliberately leaves the background
//! roles — memory, embeddings, learning — alone, because those run
//! tier-specific models a chat endpoint generally cannot serve.

use crate::turn::Route;

/// Where a harness sends its inference.
#[derive(Clone, Default)]
pub struct Provider {
    route: Option<Route>,
    model: Option<String>,
    custom: Option<std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>>>,
    roles: std::collections::BTreeMap<
        String,
        std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>>,
    >,
}

impl Provider {
    /// Use a native custom provider (including non-OpenAI transports).
    pub fn custom(model: std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>>) -> Self {
        Self {
            custom: Some(model),
            ..Self::default()
        }
    }

    /// Try native providers in order when inference fails before any output.
    /// Validation errors and failures after streamed content never switch providers.
    pub fn custom_with_fallback(
        primary: std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>>,
        fallbacks: Vec<std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>>>,
    ) -> Self {
        Self::custom(std::sync::Arc::new(
            tinyinference_llm::model::FallbackModel::new(primary, fallbacks),
        ))
    }

    /// Select a native provider for a workload role; unlisted roles keep the primary.
    pub fn role(
        mut self,
        role: impl Into<String>,
        model: std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>>,
    ) -> Self {
        self.roles.insert(role.into(), model);
        self
    }

    pub(crate) fn custom_model(
        &self,
    ) -> Option<std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>>> {
        self.custom.clone()
    }
    pub(crate) fn role_models(
        &self,
    ) -> &std::collections::BTreeMap<
        String,
        std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>>,
    > {
        &self.roles
    }

    /// Run on an OpenAI-compatible endpoint with the given bearer.
    ///
    /// `base_url` is spelled as a provider endpoint would be —
    /// `/chat/completions` is appended to it, so pass the API root
    /// (`https://host/v1`), not the completions path.
    ///
    /// Both halves are required: the core ignores a route with only one, and
    /// taking them together here means a partial route cannot be expressed.
    pub fn openai_compatible(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self::routed(Route::openai_compatible(base_url, api_key))
    }

    /// Use a route including its gateway attribution headers.
    pub fn routed(route: Route) -> Self {
        Self {
            route: Some(route),
            model: None,
            custom: None,
            roles: Default::default(),
        }
    }

    /// Set provider-specific defaults on this routed provider. Requests may
    /// override these values by key.
    pub fn provider_options(mut self, options: serde_json::Value) -> Self {
        if let Some(route) = self.route.as_mut() {
            route.provider_options = Some(options);
        }
        self
    }

    /// Set the reasoning effort used for this provider's model, including
    /// runtime-owned scheduled work.
    pub fn reasoning_effort(mut self, effort: tinyinference_llm::model::ReasoningEffort) -> Self {
        if let Some(route) = self.route.as_mut() {
            route.reasoning_effort = Some(effort);
        }
        self
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
    pub(crate) fn has_usable_route(&self) -> bool {
        self.custom.is_some()
            || self.route.as_ref().is_some_and(|route| {
                !route.base_url.trim().is_empty() && !route.api_key.trim().is_empty()
            })
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;

impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Provider")
            .field("route", &self.route)
            .field("model", &self.model)
            .field("custom", &self.custom.is_some())
            .field("roles", &self.roles.keys().collect::<Vec<_>>())
            .finish()
    }
}
impl PartialEq for Provider {
    fn eq(&self, other: &Self) -> bool {
        self.route == other.route
            && self.model == other.model
            && match (&self.custom, &other.custom) {
                (Some(a), Some(b)) => std::sync::Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
            && self.roles.len() == other.roles.len()
            && self.roles.iter().all(|(k, v)| {
                other
                    .roles
                    .get(k)
                    .is_some_and(|w| std::sync::Arc::ptr_eq(v, w))
            })
    }
}
impl Eq for Provider {}
