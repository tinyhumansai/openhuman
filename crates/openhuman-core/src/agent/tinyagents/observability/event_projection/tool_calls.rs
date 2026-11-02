use super::*;

impl OpenhumanEventBridge {
    pub(super) fn project_tool_started(&self, event: &AgentEvent) {
        if let AgentEvent::ToolStarted {
            call_id,
            tool_name,
            input,
            ..
        } = event
        {
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
    }

    pub(super) fn project_tool_completed(&self, event: &AgentEvent) {
        if let AgentEvent::ToolCompleted {
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
        } = event
        {
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
    }
}
