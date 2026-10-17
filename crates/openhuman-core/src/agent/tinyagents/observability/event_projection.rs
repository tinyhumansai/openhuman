//! [`EventListener`] implementation for [`OpenhumanEventBridge`] — projects
//! each `tinyagents` [`AgentEvent`] onto an [`AgentProgress`] update (or a
//! grep-friendly trace line for events with no UI surface), plus the
//! prompt-cache layout diagnostic helper drained after a run.

use tinyagents_harness::cache::CacheLayoutEvent;
use tinyagents_harness::events::{AgentEvent, EventListener, EventRecord};

use crate::agent::progress::AgentProgress;
use tinytools::humanize_tool_name;

use super::event_bridge::OpenhumanEventBridge;

impl EventListener for OpenhumanEventBridge {
    fn on_event(&self, record: &EventRecord) {
        match &record.event {
            AgentEvent::ModelStarted { .. } => {
                let iteration = tinyagents_harness::events::advance_iteration(&self.cursor);
                match &self.scope {
                    None => self.send(AgentProgress::IterationStarted {
                        iteration,
                        max_iterations: self.max_iterations,
                    }),
                    Some(s) => self.send(AgentProgress::SubagentIterationStarted {
                        agent_id: s.agent_id.clone(),
                        task_id: s.task_id.clone(),
                        iteration,
                        max_iterations: self.max_iterations,
                        extended_policy: s.extended_policy,
                    }),
                }
            }
            AgentEvent::ModelDelta { delta, .. } => {
                let iteration = self.iteration();
                if !delta.text.is_empty() {
                    match &self.scope {
                        None => self.send(AgentProgress::TextDelta {
                            delta: delta.text.clone(),
                            iteration,
                        }),
                        Some(s) => self.send(AgentProgress::SubagentTextDelta {
                            agent_id: s.agent_id.clone(),
                            task_id: s.task_id.clone(),
                            delta: delta.text.clone(),
                            iteration,
                        }),
                    }
                }
                if !delta.reasoning.is_empty() {
                    match &self.scope {
                        None => self.send(AgentProgress::ThinkingDelta {
                            delta: delta.reasoning.clone(),
                            iteration,
                        }),
                        Some(s) => self.send(AgentProgress::SubagentThinkingDelta {
                            agent_id: s.agent_id.clone(),
                            task_id: s.task_id.clone(),
                            delta: delta.reasoning.clone(),
                            iteration,
                        }),
                    }
                }
                // Tool-call **start** + **argument** fragments both ride the crate
                // stream (`MessageDelta.tool_call`) now — the out-of-band
                // `ThinkingForwarder` is gone. The call-opening delta carries the
                // tool name (crate `ToolDelta::tool_name`, G2) with empty content;
                // argument fragments carry content with no name. We record the
                // name on the opening delta so subsequent fragments can be
                // labelled, and project both onto the `ToolCallArgsDelta` the UI
                // timeline consumes so the model can be shown composing the call
                // before it executes.
                if let Some(tool_call) = &delta.tool_call {
                    // Record the tool name as soon as the call opens (matching the
                    // legacy forwarder's `note_tool_call`), and emit the start
                    // marker — an empty-delta `ToolCallArgsDelta` — top-level
                    // regardless of scope, exactly as the forwarder did.
                    if let Some(name) = tool_call.tool_name.as_deref().filter(|n| !n.is_empty()) {
                        self.tool_names
                            .lock()
                            .unwrap()
                            .insert(tool_call.call_id.clone(), name.to_string());
                        if tool_call.content.is_empty() {
                            self.send(AgentProgress::ToolCallArgsDelta {
                                call_id: tool_call.call_id.clone(),
                                tool_name: name.to_string(),
                                delta: String::new(),
                                iteration,
                            });
                        }
                    }
                    // Argument fragments are parent-only: there is no `Subagent*`
                    // tool-arg variant, and an UNSCOPED top-level `ToolCallArgsDelta`
                    // emitted from a child run would render the child's argument
                    // composition as the *parent's* own timeline activity (#4467,
                    // item 6; v0.58.7 dropped child arg fragments). A child run's
                    // Started/Completed rows already carry the final arguments
                    // under the `Subagent*` scope.
                    if self.scope.is_none() && !tool_call.content.is_empty() {
                        let tool_name = tool_call
                            .tool_name
                            .as_deref()
                            .filter(|n| !n.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| {
                                self.tool_names
                                    .lock()
                                    .unwrap()
                                    .get(&tool_call.call_id)
                                    .cloned()
                                    .unwrap_or_default()
                            });
                        tracing::trace!(
                            call_id = tool_call.call_id.as_str(),
                            tool_name = tool_name.as_str(),
                            len = tool_call.content.len(),
                            "[stream] projecting crate tool-arg fragment onto ToolCallArgsDelta"
                        );
                        self.send(AgentProgress::ToolCallArgsDelta {
                            call_id: tool_call.call_id.clone(),
                            tool_name,
                            delta: tool_call.content.clone(),
                            iteration,
                        });
                    }
                }
            }
            // `UsageRecorded` carries the authoritative per-call usage and fires
            // exactly once per model call; prefer it over `ModelCompleted`'s
            // optional usage to avoid double counting.
            AgentEvent::UsageRecorded { usage } => self.record_usage(usage),
            // Per-call generation telemetry. `ModelCompleted` fires exactly once
            // per model call, after `UsageRecorded`, and is the only event
            // carrying the captured request messages (incl. the system prompt)
            // + completion (`RunPolicy.capture.model_io`, enabled in
            // `run_policy_for`). Emitted for parent AND child scopes — the
            // child call carries its owning `subagent_task_id` so the trace
            // exporter nests the generation under the subagent span (this is
            // what makes the Context Scout's model calls visible in Langfuse).
            AgentEvent::ModelCompleted {
                usage,
                input,
                output,
                ..
            } => {
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
            AgentEvent::CostRecorded { cost } => {
                tracing::debug!(
                    cost = ?cost,
                    "[tinyagents] cost event observed without OpenHuman accounting side effect"
                );
            }
            AgentEvent::BudgetReserved {
                estimated_input_tokens,
            } => {
                tracing::debug!(
                    estimated_input_tokens,
                    "[tinyagents] budget reserved estimated input tokens"
                );
            }
            AgentEvent::BudgetReconciled {
                estimated_input_tokens,
                actual_input_tokens,
            } => {
                tracing::debug!(
                    estimated_input_tokens,
                    actual_input_tokens,
                    "[tinyagents] budget reservation reconciled"
                );
            }
            AgentEvent::BudgetWarning { reason } => {
                tracing::debug!(
                    reason,
                    "[tinyagents] budget warning observed without run interruption"
                );
            }
            AgentEvent::BudgetExceeded { reason, blocked } => {
                tracing::debug!(
                    reason,
                    blocked,
                    "[tinyagents] budget exceeded event observed"
                );
            }
            AgentEvent::Steered {
                command_kind,
                accepted,
            } => {
                // Grep-friendly `[steering]` projection of every drained steering
                // command (issue #4249, 07.3). A rejected command means the run's
                // `SteeringPolicy` refused the kind and the crate is aborting the
                // run with `TinyAgentsError::Steering`, so surface it louder. The
                // web-channel queue routing stays host-owned (gated: followup /
                // parallel still need a local owner); UI
                // projection of this event remains pending.
                if *accepted {
                    tracing::debug!(
                        command_kind = command_kind.as_str(),
                        accepted,
                        "[steering] command applied at safe boundary"
                    );
                } else {
                    tracing::warn!(
                        command_kind = command_kind.as_str(),
                        accepted,
                        "[steering] command rejected by run policy"
                    );
                }
            }
            AgentEvent::ToolsFiltered {
                by,
                excluded,
                remaining,
                ..
            } => {
                tracing::debug!(
                    policy = by.as_str(),
                    excluded_tools = ?excluded,
                    remaining,
                    "[tinyagents] model-visible tools filtered"
                );
            }
            AgentEvent::Compressed {
                from_tokens,
                to_tokens,
            } => {
                tracing::debug!(
                    from_tokens,
                    to_tokens,
                    saved_tokens = from_tokens.saturating_sub(*to_tokens),
                    "[tinyagents] context compressed before model call"
                );
            }
            AgentEvent::Compacted { .. } => {
                tinyagents_harness::events::log_compacted(&record.event)
            }
            AgentEvent::UnknownToolCall {
                call_id,
                requested_name,
                arguments,
                recovery,
            } => {
                tracing::debug!(
                    call_id = call_id.as_str(),
                    requested_tool = requested_name.as_str(),
                    recovery = recovery.as_str(),
                    arguments = %arguments,
                    "[tinyagents] recovered unknown tool call without executing a tool"
                );
                // #4118 surfaced the attempted unavailable tool on the timeline
                // by synthesising a Started/Completed pair here. Since TOOL-11
                // the crate answers the call through `recover_tool_call`, which
                // emits an ordinary `ToolStarted`/`ToolCompleted` pair under the
                // same call id carrying the corrective error text — so a pair
                // projected here as well doubled every unknown-tool error in
                // Langfuse (one span with null output, one with the text).
                // Remember the call instead; the crate's pair carries the
                // "unavailable" label and the `NotFound` class (#6277). A
                // `rewrite:` recovery runs the rewrite target under its own
                // name, so it is not an unavailable call.
                if !recovery.starts_with("rewrite:") {
                    self.unknown_calls
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .insert(call_id.as_str().to_string(), requested_name.clone());
                }
            }
            AgentEvent::ToolsAdvertised {
                direct,
                deferred,
                schema_bytes,
            } => {
                tracing::info!(
                    direct,
                    deferred,
                    schema_bytes,
                    "[tool-search] tools advertised for run (deferred reachable via tool_search)"
                );
            }
            AgentEvent::DeferredToolCall { call_id, tool_name } => {
                // The following `ToolStarted` names the same tool; this only
                // records that it was not on the wire and was called by name.
                tracing::debug!(
                    call_id = call_id.as_str(),
                    tool = tool_name.as_str(),
                    "[tool-search] deferred tool invoked by name"
                );
            }
            AgentEvent::ToolSearched {
                call_id,
                query,
                matched,
                ranker,
                top_confidence,
                fallback,
                shadow_matched,
                latency_ms,
            } => {
                // The harness answers `tool_search` itself, so no
                // `ToolStarted`/`ToolCompleted` pair exists for it. Project a
                // synthetic pair so the timeline shows the search and the trace
                // gets a `tool.tool_search` span carrying the ranking facts —
                // that span is how a Jev-vs-BM25 comparison is read off live
                // traffic. Never the query text in a log line; it is user
                // content. The arguments ride the progress event, which is
                // content-gated at the collector like every tool's.
                tracing::info!(
                    call_id = call_id.as_str(),
                    matched,
                    ranker = ranker.as_str(),
                    top_confidence = ?top_confidence,
                    fallback = ?fallback,
                    shadow_agrees = ?shadow_matched.as_ref().map(|_| ()),
                    latency_ms,
                    "[tool-search] answered"
                );
                let iteration = self.iteration();
                let tool_name = tinyagents_harness::tool::discover::TOOL_SEARCH_NAME.to_string();
                let arguments = serde_json::json!({ "query": query });
                let output = serde_json::json!({
                    "matched": matched,
                    "ranker": ranker,
                    "top_confidence": top_confidence,
                    "fallback": fallback,
                    "shadow_matched": shadow_matched,
                    "latency_ms": latency_ms,
                })
                .to_string();
                let output_chars = output.chars().count();
                match &self.scope {
                    None => {
                        self.send(AgentProgress::ToolCallStarted {
                            call_id: call_id.as_str().to_string(),
                            tool_name: tool_name.clone(),
                            arguments: arguments.clone(),
                            iteration,
                            display_label: Some("Searching tools".to_string()),
                            display_detail: None,
                        });
                        self.send(AgentProgress::ToolCallCompleted {
                            call_id: call_id.as_str().to_string(),
                            tool_name,
                            success: true,
                            output_chars,
                            output,
                            arguments: Some(arguments),
                            elapsed_ms: *latency_ms,
                            iteration,
                            failure: None,
                            display_label: Some("Searching tools".to_string()),
                            display_detail: None,
                            structured: None,
                        });
                    }
                    Some(s) => {
                        self.send(AgentProgress::SubagentToolCallStarted {
                            agent_id: s.agent_id.clone(),
                            task_id: s.task_id.clone(),
                            call_id: call_id.as_str().to_string(),
                            tool_name: tool_name.clone(),
                            arguments: arguments.clone(),
                            iteration,
                            display_label: Some("Searching tools".to_string()),
                            display_detail: None,
                        });
                        self.send(AgentProgress::SubagentToolCallCompleted {
                            agent_id: s.agent_id.clone(),
                            task_id: s.task_id.clone(),
                            call_id: call_id.as_str().to_string(),
                            tool_name,
                            success: true,
                            output_chars,
                            output,
                            arguments: Some(arguments),
                            elapsed_ms: *latency_ms,
                            iteration,
                            failure: None,
                            display_label: Some("Searching tools".to_string()),
                            display_detail: None,
                            structured: None,
                        });
                    }
                }
            }
            AgentEvent::ToolStarted {
                call_id,
                tool_name,
                input,
                ..
            } => {
                // Unknown tool calls reach this arm too: the crate's
                // `UnknownToolPolicy::ReturnToolError` path emits
                // `AgentEvent::UnknownToolCall` (handled above) and then answers
                // the call through `recover_tool_call`, which emits this
                // `ToolStarted` under the same call id and the requested name.
                // The call id recorded above gives it the "unavailable" label.
                let iteration = self.iteration();
                // Stamp the start instant so the completion event carries a real
                // elapsed_ms (the crate's ToolCompleted has no timing payload).
                self.tool_started_at
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(call_id.as_str().to_string(), std::time::Instant::now());
                // The harness start event now carries the captured call
                // input when payload capture is on (tinyagents#211); fall
                // back to `Value::Null` when it's off, same as before. A
                // tool whose label doesn't depend on its arguments (the
                // common case: a policy label, or a name-derived default)
                // already reads correctly with real input; one whose detail
                // DOES depend on args (e.g. a search query) is recomputed
                // with the real arguments on `ToolCallCompleted` below and
                // forwarded on the wire as
                // `tool_display_label`/`tool_display_detail` there too.
                let arguments = input.clone().unwrap_or(serde_json::Value::Null);
                let (display_label, display_detail) = match self.unknown_tool_display(call_id) {
                    Some(display) => display,
                    None => self.resolve_display(tool_name, &arguments),
                };
                match &self.scope {
                    None => self.send(AgentProgress::ToolCallStarted {
                        call_id: call_id.as_str().to_string(),
                        tool_name: tool_name.clone(),
                        arguments,
                        iteration,
                        display_label,
                        display_detail,
                    }),
                    Some(s) => self.send(AgentProgress::SubagentToolCallStarted {
                        agent_id: s.agent_id.clone(),
                        task_id: s.task_id.clone(),
                        call_id: call_id.as_str().to_string(),
                        tool_name: tool_name.clone(),
                        arguments,
                        iteration,
                        display_label,
                        display_detail,
                    }),
                }
            }
            AgentEvent::ToolCompleted {
                call_id,
                tool_name,
                input,
                output,
                error,
                // `started_at_ms`/`duration_ms`/`output_bytes` ride the crate
                // event, but the bridge reads the outcome-capture side channel
                // below: it also carries the classified failure and the tool's
                // structured metadata, which the crate event does not.
                ..
            } => {
                let iteration = self.iteration();
                // Read what the outcome-capture middleware classified for this
                // call. Absent → the middleware did not see it (e.g. a call the
                // harness answered without running a tool); fall back to the
                // crate event's own `error`.
                let outcome = self
                    .failure_map
                    .lock()
                    .ok()
                    .and_then(|mut m| m.remove(call_id.as_str()));
                let unknown_display = self.unknown_tool_display(call_id);
                let was_unknown = self
                    .unknown_calls
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(call_id.as_str())
                    .is_some();
                let success = !was_unknown
                    && outcome
                        .as_ref()
                        .map(|(ok, ..)| *ok)
                        .unwrap_or(error.is_none());
                // Real execution duration + output size the capture middleware
                // recorded off the `ToolResult` (#4467, item 4). Fall back to
                // the bridge's own ToolStarted stamp for duration, and to the
                // captured payload for size, when the middleware ran late.
                let stamped_elapsed = self
                    .tool_started_at
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .remove(call_id.as_str())
                    .map(|t| t.elapsed().as_millis() as u64)
                    .unwrap_or(0);
                let elapsed_ms = outcome
                    .as_ref()
                    .map(|(_, _, e, ..)| *e)
                    .filter(|e| *e > 0)
                    .unwrap_or(stamped_elapsed);
                // Tool result text, captured by the harness when
                // `RunPolicy.capture.tool_io` is on (the loop emits it as a
                // JSON string). Empty when capture is off.
                let output_text = match output {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(v) => v.to_string(),
                    None => String::new(),
                };
                // An unknown-tool answer's output is the corrective text; when
                // payload capture is off it still rides `error`.
                let output_text = match (&unknown_display, output_text.is_empty(), error) {
                    (Some(_), true, Some(text)) => text.clone(),
                    _ => output_text,
                };
                let output_chars = outcome
                    .as_ref()
                    .map(|(_, _, _, c, _)| *c)
                    .filter(|c| *c > 0)
                    .unwrap_or_else(|| output_text.chars().count());
                // Structured, tool-specific result payload the middleware
                // copied from `ToolResult.metadata` (e.g. web search results).
                let structured = outcome.as_ref().and_then(|(.., s)| s.clone());
                // Carry the classified failure onto whichever completion event
                // this projects — main-agent OR sub-agent (#4459). Previously
                // the sub-agent branch dropped it on the floor.
                // An unknown tool is `NotFound` from the typed event itself
                // (#6277), whatever the text sniff made of the echoed names.
                let failure = if was_unknown {
                    Some(tinytools::status::describe(
                        tinytools::status::ToolFailureClass::NotFound,
                    ))
                } else {
                    match outcome {
                        Some((_, f, ..)) => f,
                        None => error
                            .as_deref()
                            .map(|text| tinytools::status::classify(text, false)),
                    }
                };
                // Recompute the label/detail with the REAL call arguments
                // (unlike `ToolCallStarted`, this event's `input` is the
                // actual arguments the harness captured), so a tool whose
                // detail depends on its args — a search query, a target
                // email — surfaces it here even when the started event
                // couldn't.
                let args_for_display = input.clone().unwrap_or(serde_json::Value::Null);
                let (display_label, display_detail) = match unknown_display {
                    Some(display) => display,
                    None => self.resolve_display(tool_name, &args_for_display),
                };
                tracing::debug!(
                    call_id = call_id.as_str(),
                    tool_name = tool_name.as_str(),
                    success,
                    elapsed_ms,
                    has_structured = structured.is_some(),
                    "[tool-presentation] projecting ToolCallCompleted with resolved label/detail"
                );
                match &self.scope {
                    None => self.send(AgentProgress::ToolCallCompleted {
                        call_id: call_id.as_str().to_string(),
                        tool_name: tool_name.clone(),
                        success,
                        output_chars,
                        output: output_text,
                        arguments: input.clone(),
                        elapsed_ms,
                        iteration,
                        failure,
                        display_label,
                        display_detail,
                        structured,
                    }),
                    Some(s) => self.send(AgentProgress::SubagentToolCallCompleted {
                        agent_id: s.agent_id.clone(),
                        task_id: s.task_id.clone(),
                        call_id: call_id.as_str().to_string(),
                        tool_name: tool_name.clone(),
                        success,
                        output_chars,
                        output: output_text,
                        arguments: input.clone(),
                        elapsed_ms,
                        iteration,
                        failure,
                        display_label,
                        display_detail,
                        structured,
                    }),
                }
            }
            // Response-cache accounting (issue #4249, 03.2). A hit means the
            // harness served this model call from its local `ResponseCache`
            // without invoking the provider (deterministic internal runs only —
            // interactive chat never attaches a cache). Counters are additive; the
            // cost-footer DTO wiring is a follow-up (workstream 06).
            AgentEvent::ToolProgressDetail {
                call_id, message, ..
            } => self.forward_tool_progress(call_id.as_str(), message),
            AgentEvent::CacheHit { call_id, key } => {
                {
                    let mut s = self.state.lock().unwrap();
                    s.cache_hits += 1;
                }
                tracing::debug!(
                    model = %self.model,
                    call_id = call_id.as_str(),
                    key = key.as_str(),
                    "[cache] response-cache hit — provider call skipped"
                );
            }
            AgentEvent::CacheMiss { call_id, key } => {
                {
                    let mut s = self.state.lock().unwrap();
                    s.cache_misses += 1;
                }
                tracing::debug!(
                    model = %self.model,
                    call_id = call_id.as_str(),
                    key = key.as_str(),
                    "[cache] response-cache miss — invoking provider and storing result"
                );
            }
            // Retry/fallback parity (issue #4249, Workstream 02.2). These surface the
            // SDK-owned reliability decisions on the observability bridge so they are
            // no longer silently dropped by the catch-all below. `RetryScheduled` is
            // emitted by the crate's model-retry loop; with the retry pin at a single
            // attempt (`RunPolicy.retry.max_attempts = 1`, pending `ReliableProvider`
            // removal) it will not fire on the live path yet, but the bridge is wired
            // for when it does. `FallbackSelected` is emitted by
            // [`FallbackObserverMiddleware`](super::routes::FallbackObserverMiddleware)
            // whenever the harness fails over to a sibling workload-tier route.
            AgentEvent::RetryScheduled { call_id, attempt } => {
                tracing::info!(
                    model = %self.model,
                    call_id = call_id.as_str(),
                    attempt,
                    "[models] SDK scheduled a model-call retry after a retryable provider error"
                );
            }
            AgentEvent::FallbackSelected { from, to } => {
                tracing::info!(
                    model = %self.model,
                    from = from.as_str(),
                    to = to.as_str(),
                    "[fallback] SDK failed over to a cross-route fallback model"
                );
            }
            other => {
                // Not projected into `AgentProgress` (run lifecycle, sub-agent
                // boundaries — reconstructed from the orchestration tools'
                // manual emits — middleware, workspace, memory, limits). Trace
                // the kind so a dropped-event hypothesis is checkable from
                // logs instead of reading this match.
                tracing::trace!(
                    model = %self.model,
                    kind = ?std::mem::discriminant(other),
                    "[tinyagents:bridge] event observed but not forwarded to UI progress"
                );
            }
        }
    }
}

impl OpenhumanEventBridge {
    /// The "unavailable" label/detail pair for a call the crate answered as an
    /// unknown tool, or `None` for an ordinary call.
    fn unknown_tool_display(
        &self,
        call_id: &tinyagents_harness::ids::CallId,
    ) -> Option<(Option<String>, Option<String>)> {
        let calls = self.unknown_calls.lock().unwrap_or_else(|p| p.into_inner());
        calls.get(call_id.as_str()).map(|requested| {
            (
                Some(format!("{} (unavailable)", humanize_tool_name(requested))),
                Some("tool not available".to_string()),
            )
        })
    }
}

/// Surface the crate `PromptCacheGuardMiddleware`'s recorded
/// [`CacheLayoutEvent`]s as structured `[cache]` warnings (issue #4249, 03.2).
///
/// The guard records a layout event whenever the cacheable prompt prefix changes
/// between turns (volatile content — a timestamp, uuid, injected memory, etc. —
/// silently busting the provider KV-cache prefix). This is the crate-native
/// replacement for the deleted `CacheAlignMiddleware` free-text warn-log:
/// instead of a token-pattern heuristic it reports the exact before/after
/// cacheable segment ids. Drained by the turn loop after the run and logged
/// here. The warn-only `CacheAlignMiddleware` shadow was deleted in C3; this
/// guard is now the sole owner of KV-cache-prefix drift detection.
pub(crate) fn surface_cache_layout_events(model: &str, events: &[CacheLayoutEvent]) {
    for event in events {
        tracing::warn!(
            model,
            changed_prefix = event.changed_prefix,
            volatile_only = event.volatile_only,
            segments_before = ?event.segment_ids_before,
            segments_after = ?event.segment_ids_after,
            "[cache] prompt-cache prefix changed across turns — KV-cache prefix may not hit; keep dynamic content out of the system prompt / stable tool set"
        );
    }
}
