//! Phase 7 Acceptance Test Harness and Scaffolding.
//!
//! Provides reusable in-process mocks, recording models, recording tools,
//! endpoint and effect counters, healthy and failed memory stubs, and progress
//! capture sinks for Gates 6 and 7 acceptance tests.

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tinyagents_harness::host::security_gate::{GateDecision, ToolCallRequest};
use tinyinference::{
    message::AssistantMessage,
    model::{ChatModel, ModelRequest, ModelResponse},
    tool::ToolCall,
    usage::Usage,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    agent::{
        goose::{GooseToolSecurity, GooseTurnAdapter, InMemoryGooseCheckpointStore},
        messages::{ChatMessage, ConversationMessage},
        primary_orchestration::{
            capability::{
                CapabilityAvailability, CapabilityBackend, CapabilityModality, CapabilityOperation,
                CapabilitySideEffect, MonetaryBoundary, ToolCapability, ToolRoute,
            },
            completion::CompletionContract,
        },
        progress::AgentProgress,
    },
    tools::traits::{PermissionLevel, Tool, ToolResult},
};

// Child module declarations for Phase 7 acceptance test suites
#[cfg(test)]
#[path = "phase7_acceptance_web_tests.rs"]
pub mod phase7_acceptance_web_tests;

#[cfg(test)]
#[path = "phase7_acceptance_action_tests.rs"]
pub mod phase7_acceptance_action_tests;

#[cfg(test)]
#[path = "phase7_acceptance_state_tests.rs"]
pub mod phase7_acceptance_state_tests;

/// In-process recording chat model that returns scripted responses and tracks invocations.
pub struct RecordingModel {
    pub requests: Arc<Mutex<Vec<ModelRequest>>>,
    pub responses: Mutex<VecDeque<ModelResponse>>,
    pub endpoint_counter: Arc<AtomicUsize>,
}

impl RecordingModel {
    pub fn new(responses: Vec<ModelResponse>, endpoint_counter: Arc<AtomicUsize>) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            responses: Mutex::new(responses.into()),
            endpoint_counter,
        }
    }

    pub fn recorded_requests(&self) -> Vec<ModelRequest> {
        self.requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

#[async_trait]
impl ChatModel<()> for RecordingModel {
    async fn invoke(
        &self,
        _state: &(),
        request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        self.endpoint_counter.fetch_add(1, Ordering::SeqCst);
        self.requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(request);
        self.responses
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pop_front()
            .ok_or_else(|| tinyinference::Error::Model("scripted model exhausted".into()))
    }
}

/// Generic recording tool tracking executions, side-effects, and arguments.
pub struct RecordingTool {
    pub name: String,
    pub description: String,
    pub operations: Vec<CapabilityOperation>,
    pub modalities: Vec<CapabilityModality>,
    pub backend: CapabilityBackend,
    pub monetary_boundary: MonetaryBoundary,
    pub side_effect: CapabilitySideEffect,
    pub permission: PermissionLevel,
    pub call_counter: Arc<AtomicUsize>,
    pub effect_counter: Arc<AtomicUsize>,
    pub recorded_arguments: Arc<Mutex<Vec<Value>>>,
    pub response_generator: Arc<dyn Fn(&Value) -> Result<ToolResult> + Send + Sync>,
}

impl RecordingTool {
    pub fn new(
        name: impl Into<String>,
        operations: Vec<CapabilityOperation>,
        modalities: Vec<CapabilityModality>,
        backend: CapabilityBackend,
        monetary_boundary: MonetaryBoundary,
        side_effect: CapabilitySideEffect,
        call_counter: Arc<AtomicUsize>,
        effect_counter: Arc<AtomicUsize>,
        response_generator: impl Fn(&Value) -> Result<ToolResult> + Send + Sync + 'static,
    ) -> Self {
        let name_str = name.into();
        Self {
            name: name_str.clone(),
            description: format!("Recording tool for {name_str}"),
            operations,
            modalities,
            backend,
            monetary_boundary,
            side_effect,
            permission: PermissionLevel::default(),
            call_counter,
            effect_counter,
            recorded_arguments: Arc::new(Mutex::new(Vec::new())),
            response_generator: Arc::new(response_generator),
        }
    }

    pub fn as_route(&self, priority: u16) -> ToolRoute {
        ToolRoute::new(ToolCapability {
            name: self.name.clone(),
            operations: self.operations.clone(),
            modalities: self.modalities.clone(),
            backend: self.backend,
            monetary_boundary: self.monetary_boundary,
            side_effect: self.side_effect,
            availability: CapabilityAvailability::Available,
            permission: self.permission,
            priority,
            registration_index: 0,
        })
    }
}

#[async_trait]
impl Tool for RecordingTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "url": { "type": "string" },
                "prompt": { "type": "string" },
                "path": { "type": "string" },
                "content": { "type": "string" },
                "key": { "type": "string" },
                "value": { "type": "string" }
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        self.call_counter.fetch_add(1, Ordering::SeqCst);
        if matches!(
            self.side_effect,
            CapabilitySideEffect::LocalWrite | CapabilitySideEffect::ExternalWrite
        ) {
            self.effect_counter.fetch_add(1, Ordering::SeqCst);
        }
        self.recorded_arguments
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(args.clone());
        (self.response_generator)(&args)
    }
}

/// In-process memory stub that supports recall and store with configurable healthy or failed state.
pub struct MemoryStubTool {
    pub name: String,
    pub is_healthy: bool,
    pub memory_store: Arc<Mutex<Vec<String>>>,
    pub call_counter: Arc<AtomicUsize>,
}

impl MemoryStubTool {
    pub fn healthy(name: impl Into<String>, initial_memories: Vec<String>) -> Self {
        Self {
            name: name.into(),
            is_healthy: true,
            memory_store: Arc::new(Mutex::new(initial_memories)),
            call_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn failed(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_healthy: false,
            memory_store: Arc::new(Mutex::new(Vec::new())),
            call_counter: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn as_route(&self, is_store: bool, priority: u16) -> ToolRoute {
        let op = if is_store {
            CapabilityOperation::StoreMemory
        } else {
            CapabilityOperation::RecallMemory
        };
        ToolRoute::new(ToolCapability {
            name: self.name.clone(),
            operations: vec![op],
            modalities: vec![CapabilityModality::Memory],
            backend: CapabilityBackend::Local,
            monetary_boundary: MonetaryBoundary::NonMetered,
            side_effect: if is_store {
                CapabilitySideEffect::LocalWrite
            } else {
                CapabilitySideEffect::LocalRead
            },
            availability: if self.is_healthy {
                CapabilityAvailability::Available
            } else {
                CapabilityAvailability::Unhealthy
            },
            permission: PermissionLevel::default(),
            priority,
            registration_index: 0,
        })
    }
}

#[async_trait]
impl Tool for MemoryStubTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        "Memory stub tool for testing recall and storage"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "content": { "type": "string" }
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        self.call_counter.fetch_add(1, Ordering::SeqCst);
        if !self.is_healthy {
            return Ok(ToolResult::error("Memory subsystem is unavailable"));
        }
        if let Some(content) = args.get("content").and_then(|c| c.as_str()) {
            self.memory_store
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(content.to_string());
            Ok(ToolResult::success(format!("Stored memory: {content}")))
        } else {
            let memories = self.memory_store.lock().unwrap_or_else(|p| p.into_inner());
            let query = args.get("query").and_then(|q| q.as_str()).unwrap_or("");
            let matching: Vec<_> = memories
                .iter()
                .filter(|m| m.contains(query))
                .cloned()
                .collect();
            Ok(ToolResult::success(json!(matching).to_string()))
        }
    }
}

/// Allow-all security gate implementation for acceptance testing.
#[derive(Default)]
pub struct MockAllowSecurity;

#[async_trait]
impl GooseToolSecurity for MockAllowSecurity {
    async fn authorize(&self, _request: &ToolCallRequest) -> Result<GateDecision> {
        Ok(GateDecision::Allow)
    }

    fn record_execution(&self, _call_id: &str, _success: bool, _error: Option<&str>) {}
}

/// Deny-all security gate implementation for acceptance testing.
pub struct MockDenySecurity {
    pub denial_reason: String,
}

impl MockDenySecurity {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            denial_reason: reason.into(),
        }
    }
}

#[async_trait]
impl GooseToolSecurity for MockDenySecurity {
    async fn authorize(&self, _request: &ToolCallRequest) -> Result<GateDecision> {
        Ok(GateDecision::Deny {
            reason: self.denial_reason.clone(),
        })
    }

    fn record_execution(&self, _call_id: &str, _success: bool, _error: Option<&str>) {}
}

/// Acceptance test harness environment bundle.
pub struct AcceptanceHarness {
    pub store: Arc<InMemoryGooseCheckpointStore>,
    pub endpoint_counter: Arc<AtomicUsize>,
    pub effect_counter: Arc<AtomicUsize>,
    pub progress_rx: mpsc::Receiver<AgentProgress>,
    pub progress_tx: mpsc::Sender<AgentProgress>,
    pub cancel_token: CancellationToken,
}

impl AcceptanceHarness {
    pub fn new() -> Self {
        let (progress_tx, progress_rx) = mpsc::channel(256);
        Self {
            store: Arc::new(InMemoryGooseCheckpointStore::default()),
            endpoint_counter: Arc::new(AtomicUsize::new(0)),
            effect_counter: Arc::new(AtomicUsize::new(0)),
            progress_rx,
            progress_tx,
            cancel_token: CancellationToken::new(),
        }
    }

    pub fn initial_messages(user_prompt: &str) -> Vec<ConversationMessage> {
        vec![
            ConversationMessage::Chat(ChatMessage::system(
                "You are OpenHuman. Answer or execute tools as appropriate.",
            )),
            ConversationMessage::Chat(ChatMessage::user(user_prompt.to_string())),
        ]
    }

    pub fn build_adapter(
        &self,
        model: Arc<RecordingModel>,
        tools: Vec<Box<dyn Tool>>,
        routes: Vec<ToolRoute>,
        security: Arc<dyn GooseToolSecurity>,
        contract: Option<CompletionContract>,
        max_primary_calls: u32,
    ) -> GooseTurnAdapter {
        GooseTurnAdapter {
            store: self.store.clone(),
            model,
            model_name: "lmstudio:qwen38-openhuman".into(),
            provider_id: "lmstudio".into(),
            durable_tools: Arc::new(tools),
            synthesized_tools: Arc::new(Vec::new()),
            routes,
            security,
            progress: Some(self.progress_tx.clone()),
            cancel: self.cancel_token.clone(),
            max_output_tokens: None,
            max_primary_calls,
            max_no_progress_calls: 2,
            contract,
        }
    }

    pub async fn drain_progress(&mut self) -> Vec<AgentProgress> {
        let mut events = Vec::new();
        while let Ok(event) = self.progress_rx.try_recv() {
            events.push(event);
        }
        events
    }
}

/// Helper to build a scripted tool call response.
pub fn make_tool_call_response(
    call_id: &str,
    tool_name: &str,
    arguments: Value,
    input_tokens: u64,
    output_tokens: u64,
) -> ModelResponse {
    let usage = Usage::new(input_tokens, output_tokens);
    ModelResponse {
        message: AssistantMessage {
            id: Some(format!("asst-{call_id}")),
            content: Vec::new(),
            tool_calls: vec![ToolCall::new(
                call_id.to_string(),
                tool_name.to_string(),
                arguments,
            )],
            usage: Some(usage),
        },
        usage: Some(usage),
        finish_reason: Some("tool_calls".into()),
        raw: None,
        resolved_model: None,
        continue_turn: Some("tool".into()),
        served_from_cache: false,
    }
}

/// Helper to build a scripted final assistant response.
pub fn make_final_response(text: &str, input_tokens: u64, output_tokens: u64) -> ModelResponse {
    ModelResponse::assistant(text).with_usage(Usage::new(input_tokens, output_tokens))
}
