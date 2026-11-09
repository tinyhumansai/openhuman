use super::*;

impl OpenhumanEventBridge {
    pub(super) fn project_model_completed(&self, event: &AgentEvent) {
        if let AgentEvent::ModelCompleted {
            usage,
            input,
            output,
            ..
        } = event
        {
            let iteration = self.iteration();
            let usage = usage.unwrap_or_default();
            // Prefer the figures `record_usage` resolved for this call
            // (charged>estimate cost + carried cache/reasoning breakdown —
            // `UsageRecorded` fires before `ModelCompleted`), so the
            // generation telemetry matches the wallet accounting exactly;
            // fall back to a bare tier-aware estimate when absent.
            let resolved = self
                .resolved_calls
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(&iteration);
            let call_cost = match resolved.as_ref() {
                Some(r) => r.cost_usd,
                None => Self::estimate_call_cost(&self.model, &usage),
            };
            let cache_creation_tokens = resolved
                .as_ref()
                .map(|r| r.cache_creation_tokens)
                .unwrap_or(usage.cache_creation_tokens);
            let reasoning_tokens = resolved
                .as_ref()
                .map(|r| r.reasoning_tokens)
                .unwrap_or(usage.reasoning_tokens);
            log::debug!(
                "[tinyagents][usage] model_call_completed model={} provider={} iteration={} \
                     child={} in={} out={} cache_read={} cache_write={} reasoning={} \
                     cost_usd={:?} input_captured={} output_captured={}",
                self.model,
                self.provider_id,
                iteration,
                self.scope.is_some(),
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens,
                cache_creation_tokens,
                reasoning_tokens,
                call_cost,
                input.is_some(),
                output.is_some(),
            );
            self.send(AgentProgress::ModelCallCompleted {
                model: self.model.clone(),
                provider_id: self.provider_id.clone(),
                subagent_task_id: self.scope.as_ref().map(|s| s.task_id.clone()),
                input: input.clone(),
                output: output.clone(),
                iteration,
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_input_tokens: usage.cache_read_tokens,
                cache_creation_tokens,
                reasoning_tokens,
                cost_usd: call_cost,
            });
        }
    }
}
