//! Per-turn spend and dispatch controls, applied beneath attachment rendering
//! so admission sees the request the provider actually receives.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use tinyagents_harness::cost::{
    ControlledChatModel, ControlledDispatchError, DispatchModelOptions, ModelBudget,
    PhysicalDispatchAdmission,
};
use tinyinference_llm::model::budget::Spend;
use tinyinference_llm::model::{
    ChatModel, DeferredHandle, DeferredStatus, InputModality, InputSource, ModelProfile,
    ModelRequest, ModelResponse, ModelStream,
};

/// The monetary budget and physical-dispatch admission one turn runs under.
#[derive(Clone)]
pub(crate) struct HostControls {
    budget: Option<ModelBudget>,
    admission: Option<Arc<dyn PhysicalDispatchAdmission>>,
}

impl HostControls {
    /// `None` when the turn has neither a budget nor an admission policy.
    pub(crate) fn new(
        budget: Option<&ModelBudget>,
        admission: Option<Arc<dyn PhysicalDispatchAdmission>>,
    ) -> Option<Self> {
        if budget.is_none() && admission.is_none() {
            return None;
        }
        Some(Self {
            budget: budget.cloned(),
            admission,
        })
    }

    /// Wrap a concrete provider so each call is admitted and charged.
    pub(crate) fn wrap(&self, model: Arc<dyn ChatModel<()>>) -> Arc<dyn ChatModel<()>> {
        Arc::new(ControlledChatModel::new(
            model,
            DispatchModelOptions::new()
                .monetary_budget(self.budget.clone())
                .admission(self.admission.clone()),
        ))
    }

    /// Invoke a provider whose request carries media the budgeted wrapper
    /// cannot admit, charging one conservative call reservation to the same
    /// ledger and settling it from the response's measured usage.
    pub(crate) async fn invoke_media(
        &self,
        model: Arc<dyn ChatModel<()>>,
        mut request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        let model = ControlledChatModel::new(
            model,
            DispatchModelOptions::new().admission(self.admission.clone()),
        );
        let Some(budget) = &self.budget else {
            return model.invoke(&(), request).await;
        };
        let output_tokens = request
            .max_tokens
            .unwrap_or(budget.call.output_tokens)
            .min(budget.call.output_tokens);
        request.max_tokens = Some(output_tokens);
        let reserved = Spend {
            tokens: budget
                .call
                .input_tokens
                .saturating_add(u64::from(output_tokens)),
            cost_micros: budget.call.cost_micros,
        };
        let reservation = budget.ledger.reserve(reserved).map_err(|refusal| {
            tracing::debug!(?refusal, "[host_controls] media call refused by budget");
            tinyinference_llm::Error::BudgetExceeded(refusal)
        })?;
        match model.invoke_controlled(&(), request).await {
            Ok(response) => {
                let spent = measured_spend(&response, reserved);
                tracing::debug!(
                    tokens = spent.tokens,
                    cost_micros = spent.cost_micros,
                    "[host_controls] media call settled"
                );
                reservation.settle(spent);
                Ok(response)
            }
            Err(ControlledDispatchError::Admission(refusal)) => {
                reservation.settle(Spend::default());
                tracing::debug!(?refusal, "[host_controls] media call refused by admission");
                Err(ControlledDispatchError::Admission(refusal).into())
            }
            Err(error) => Err(error.into()),
        }
    }
}

fn measured_spend(response: &ModelResponse, reserved: Spend) -> Spend {
    if response.served_from_cache {
        return Spend::default();
    }
    let usage = response.usage;
    Spend {
        tokens: usage.map_or(reserved.tokens, |usage| {
            usage
                .input_tokens
                .saturating_add(usage.output_tokens)
                .max(usage.total_tokens)
        }),
        cost_micros: usage
            .and_then(|usage| usage.charged_amount)
            .and_then(|amount| u64::try_from(amount.micros).ok())
            .unwrap_or(reserved.cost_micros),
    }
}

/// Shared by every model of one turn; filled once when the turn is admitted.
#[derive(Clone, Default)]
pub(crate) struct HostControlSlot(Arc<OnceLock<HostControls>>);

impl HostControlSlot {
    pub(crate) fn install(&self, controls: HostControls) {
        if self.0.set(controls).is_err() {
            tracing::warn!("[host_controls] controls already installed for this turn");
        }
    }

    pub(crate) fn controls(&self) -> Option<&HostControls> {
        self.0.get()
    }

    /// Place the slot directly above a concrete provider.
    pub(crate) fn wrap(&self, inner: Arc<dyn ChatModel<()>>) -> Arc<dyn ChatModel<()>> {
        Arc::new(SlotModel {
            inner,
            slot: self.clone(),
            controlled: OnceLock::new(),
        })
    }
}

struct SlotModel {
    inner: Arc<dyn ChatModel<()>>,
    slot: HostControlSlot,
    controlled: OnceLock<Arc<dyn ChatModel<()>>>,
}

impl SlotModel {
    fn target(&self) -> &Arc<dyn ChatModel<()>> {
        match self.slot.controls() {
            Some(controls) => self
                .controlled
                .get_or_init(|| controls.wrap(self.inner.clone())),
            None => &self.inner,
        }
    }
}

#[async_trait]
impl ChatModel<()> for SlotModel {
    fn profile(&self) -> Option<&ModelProfile> {
        self.inner.profile()
    }

    fn supports_input(&self, modality: InputModality, mime: &str, source: InputSource) -> bool {
        let budgeted = self
            .slot
            .controls()
            .is_some_and(|controls| controls.budget.is_some());
        !budgeted && self.inner.supports_input(modality, mime, source)
    }

    fn cache_identity(&self) -> Option<String> {
        self.inner.cache_identity()
    }

    fn tracks_physical_attempts(&self) -> bool {
        self.target().tracks_physical_attempts()
    }

    fn has_pre_invocation_admission_gate(&self) -> bool {
        self.target().has_pre_invocation_admission_gate()
    }

    async fn invoke(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        self.target().invoke(state, request).await
    }

    async fn stream(
        &self,
        state: &(),
        request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelStream> {
        self.target().stream(state, request).await
    }

    async fn fetch_deferred(
        &self,
        handle: &DeferredHandle,
    ) -> tinyinference_llm::Result<DeferredStatus> {
        self.target().fetch_deferred(handle).await
    }
}
