//! [`TurnModelResolver`] — the host resolver over a turn's pre-built models.

use std::sync::Arc;

use async_trait::async_trait;

use super::model::TurnChatModel;
use super::turn_models::TurnModels;
use tinyagents_harness::host::{ModelResolveRequest, ModelResolver};
use tinyinference_llm::model::ErrorSlotModel;

/// Host resolver for one live invocation. It exposes the exact pre-built
/// primary and fallback route models already selected by OpenHuman, rather
/// than constructing a fresh config-routed model during hosted preparation.
///
/// # Who wins: the turn's selection or the definition's pin
///
/// The **primary** is the model OpenHuman already chose for this turn — the
/// user's per-thread `model_override`, else `config.default_model` — and for
/// the turn's lead (the depth-0 agent, `is_team_lead`) it always wins. The
/// harness forwards exact model pins and typed routing hints separately.
/// Both are advisory, so neither may replace the lead's stronger selection.
/// Sub-agents resolve an exact pin first, otherwise `hint:<name>`, against
/// prebuilt tier routes; an unavailable route falls back to the primary.
///
/// Only the lead's model records into the turn's error slot (#6724). Sub-agents
/// resolve through this same resolver, possibly in parallel, so they get the
/// unwrapped models: a child's attempt must neither clear the lead's recorded
/// failure nor leave its own failure to be re-surfaced as the lead's.
pub(crate) struct TurnModelResolver {
    lead: TurnChatModel,
    primary: TurnChatModel,
    routes: std::collections::HashMap<String, TurnChatModel>,
}

impl TurnModelResolver {
    pub(crate) fn from_turn_models(models: &TurnModels) -> Self {
        let mut resolver = Self::new(
            models.primary.clone(),
            models.routes.iter().cloned().collect(),
        );
        resolver.lead = Arc::new(ErrorSlotModel::new(
            models.primary.clone(),
            models.error_slot.clone(),
        ));
        resolver
    }

    pub(crate) fn new(
        primary: TurnChatModel,
        routes: std::collections::HashMap<String, TurnChatModel>,
    ) -> Self {
        Self {
            lead: primary.clone(),
            primary,
            routes,
        }
    }
}

#[async_trait]
impl ModelResolver<()> for TurnModelResolver {
    async fn resolve(
        &self,
        request: &ModelResolveRequest,
    ) -> tinyagents_harness::Result<TurnChatModel> {
        let hint_route = request
            .model_hint
            .as_deref()
            .map(str::trim)
            .filter(|hint| !hint.is_empty())
            .map(|hint| format!("hint:{hint}"));
        let pin = request.model_pin().or(hint_route.as_deref());
        if request.is_team_lead {
            if let Some(pin) = pin.filter(|pin| self.routes.contains_key(*pin)) {
                tracing::debug!(
                    target: "tinyagents",
                    agent_id = %request.agent_id,
                    pin,
                    "[models][resolver] lead keeps the turn's selected primary; definition model pin ignored"
                );
            }
            return Ok(self.lead.clone());
        }
        Ok(pin
            .and_then(|name| self.routes.get(name))
            .cloned()
            .unwrap_or_else(|| self.primary.clone()))
    }
}
