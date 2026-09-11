//! Tool dialects — OpenHuman's adapter over
//! [`tinyagents_harness::tool_calling::dialect`].
//!
//! The dialects themselves moved to the crate: how a model is told to ask for a
//! tool, how it is parsed when it does, how results are rendered back, and how
//! a transcript is replayed onto the provider wire so the next iteration is
//! well-formed. None of that is OpenHuman-specific, and keeping a second copy
//! of it here is how a catalogue and its parser drift apart.
//!
//! # What stayed
//!
//! Two things, both of them vocabulary.
//!
//! First, the **types**. [`ParsedToolCall`] and [`ToolExecutionResult`] are
//! named and shaped for ~190 call sites across the harness, and
//! [`ConversationMessage`] is the durable JSONL record existing installations
//! already have on disk. The crate speaks its own thin
//! [`TranscriptEntry`](tinyagents_harness::tool_calling::dialect::TranscriptEntry)
//! instead, so the conversions below are the seam — a handful of field-wise
//! maps that keep the wire bytes identical while the logic lives upstream.
//!
//! Second, the **[`Tool`] trait object**. The crate takes
//! [`ToolSchema`](tinyinference::tool::ToolSchema)s, never a host's tool
//! type, for the reason the parse seam already documents: a crate that depended
//! on OpenHuman's `Tool` could not be used by a second host. So
//! [`ToolDispatcher::prompt_instructions`] reads names and schemas off the
//! slice and hands the crate exactly what it needs.
//!
//! # What did not move, and will not
//!
//! Executing a tool. The security policy, the approval gate, the sandbox, the
//! per-call timeout, the progress events — those are OpenHuman's, and a dialect
//! never decides what is *allowed to happen*, only what the model reads and
//! writes. That line is what keeps the policy auditable in one place.

use crate::openhuman::agent::context::prompt::ToolCallFormat;
use crate::openhuman::agent::messages::{ChatMessage, ConversationMessage, ToolResultMessage};
use crate::openhuman::agent::pformat::PFormatRegistry;
use crate::openhuman::inference::provider::{ChatResponse, ToolCall};
use crate::openhuman::tools::{Tool, ToolSpec};
use serde_json::Value;
use tinyagents_harness::tool_calling::dialect::{
    DialectMessage, DialectResponse, DialectRole, NativeDialect, NativeToolCall, PFormatDialect,
    ToolDialect, ToolOutcome, ToolResultEntry, TranscriptEntry, XmlDialect,
};
use tinyinference::tool::ToolSchema;

/// A parsed tool call representation after being extracted from an LLM response.
#[derive(Debug, Clone)]
pub struct ParsedToolCall {
    /// The name of the tool to be invoked.
    pub name: String,
    /// The arguments passed to the tool, as a JSON object.
    pub arguments: Value,
    /// An optional unique identifier for the tool call, provided by native APIs.
    pub tool_call_id: Option<String>,
}

/// The result of executing a tool call, formatted for the LLM.
#[derive(Debug, Clone)]
pub struct ToolExecutionResult {
    /// The name of the tool that was executed.
    pub name: String,
    /// The output of the tool execution as a string.
    pub output: String,
    /// Whether the tool execution was successful.
    pub success: bool,
    /// The tool call ID that generated this result.
    pub tool_call_id: Option<String>,
}

/// Trait defining how an agent interacts with an LLM for tool use.
///
/// Different LLMs have different "dialects" for calling tools. The dispatcher
/// abstracts these differences, allowing the agent loop to remain agnostic of
/// the specific formatting required by the provider.
///
/// Each implementation below is a thin projection of one
/// [`ToolDialect`] into OpenHuman's own vocabulary.
pub trait ToolDispatcher: Send + Sync {
    /// Parse the LLM response to extract narrative text and any tool calls.
    fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ParsedToolCall>);

    /// Format tool execution results into a message suitable for the next LLM turn.
    fn format_results(&self, results: &[ToolExecutionResult]) -> ConversationMessage;

    /// Provide instructions for the system prompt on how the model should call tools.
    fn prompt_instructions(&self, tools: &[Box<dyn Tool>]) -> String;

    /// Provide instructions from already-filtered tool specs when a dispatcher
    /// embeds the tool catalogue in its prompt protocol.
    fn prompt_instructions_for_specs(&self, _specs: &[ToolSpec]) -> Option<String> {
        None
    }

    /// Convert internal conversation history into provider-specific messages.
    fn to_provider_messages(&self, history: &[ConversationMessage]) -> Vec<ChatMessage>;

    /// Whether the dispatcher requires tool specifications to be sent in the API request.
    fn should_send_tool_specs(&self) -> bool;

    /// Tell the prompt builder how to render each tool entry in the
    /// `## Tools` section. Defaults to [`ToolCallFormat::Json`] for
    /// dispatchers that haven't opted in.
    fn tool_call_format(&self) -> ToolCallFormat {
        ToolCallFormat::Json
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The seam: OpenHuman vocabulary ↔ crate vocabulary
//
// Every function here is field-wise and lossless in the direction it is used.
// If one of them ever needs a judgement call, that judgement belongs in the
// crate — a conversion that decides something is a second implementation
// wearing a disguise.
// ─────────────────────────────────────────────────────────────────────────────

/// Project a provider response into the shape a dialect reads.
fn to_dialect_response(response: &ChatResponse) -> DialectResponse {
    DialectResponse {
        text: response.text.clone(),
        tool_calls: response.tool_calls.iter().map(to_native_call).collect(),
    }
}

fn to_native_call(call: &ToolCall) -> NativeToolCall {
    NativeToolCall {
        id: call.id.clone(),
        name: call.name.clone(),
        arguments: call.arguments.clone(),
        extra_content: call.extra_content.clone(),
    }
}

fn from_native_call(call: &NativeToolCall) -> ToolCall {
    ToolCall {
        id: call.id.clone(),
        name: call.name.clone(),
        arguments: call.arguments.clone(),
        extra_content: call.extra_content.clone(),
    }
}

fn from_parsed_calls(
    calls: Vec<tinyagents_harness::tool_calling::ParsedToolCall>,
) -> Vec<ParsedToolCall> {
    calls
        .into_iter()
        .map(|call| ParsedToolCall {
            name: call.name,
            arguments: call.arguments,
            tool_call_id: call.id,
        })
        .collect()
}

fn to_outcomes(results: &[ToolExecutionResult]) -> Vec<ToolOutcome> {
    results
        .iter()
        .map(|result| ToolOutcome {
            name: result.name.clone(),
            output: result.output.clone(),
            success: result.success,
            tool_call_id: result.tool_call_id.clone(),
        })
        .collect()
}

/// Project one durable OpenHuman record onto the crate's transcript entry.
fn to_transcript_entry(message: &ConversationMessage) -> TranscriptEntry {
    match message {
        ConversationMessage::Chat(chat) => TranscriptEntry::Chat(to_dialect_message(chat)),
        ConversationMessage::AssistantToolCalls {
            text,
            tool_calls,
            reasoning_content,
            extra_metadata,
        } => TranscriptEntry::AssistantToolCalls {
            text: text.clone(),
            tool_calls: tool_calls.iter().map(to_native_call).collect(),
            reasoning_content: reasoning_content.clone(),
            extra_metadata: extra_metadata.clone(),
        },
        ConversationMessage::ToolResults(results) => TranscriptEntry::ToolResults(
            results
                .iter()
                .map(|result| ToolResultEntry {
                    tool_call_id: result.tool_call_id.clone(),
                    content: result.content.clone(),
                })
                .collect(),
        ),
    }
}

fn to_transcript(history: &[ConversationMessage]) -> Vec<TranscriptEntry> {
    history.iter().map(to_transcript_entry).collect()
}

fn from_transcript_entry(entry: TranscriptEntry) -> ConversationMessage {
    match entry {
        TranscriptEntry::Chat(message) => ConversationMessage::Chat(from_dialect_message(message)),
        TranscriptEntry::AssistantToolCalls {
            text,
            tool_calls,
            reasoning_content,
            extra_metadata,
        } => ConversationMessage::AssistantToolCalls {
            text,
            tool_calls: tool_calls.iter().map(from_native_call).collect(),
            reasoning_content,
            extra_metadata,
        },
        TranscriptEntry::ToolResults(results) => ConversationMessage::ToolResults(
            results
                .into_iter()
                .map(|result| ToolResultMessage {
                    tool_call_id: result.tool_call_id,
                    content: result.content,
                })
                .collect(),
        ),
    }
}

fn to_dialect_message(message: &ChatMessage) -> DialectMessage {
    // `role` is a free-form string in the durable record; anything the crate
    // does not model is a `user` turn, which is what every provider treats an
    // unrecognized role as anyway.
    let role = match message.role.as_str() {
        "system" => DialectRole::System,
        "assistant" => DialectRole::Assistant,
        "tool" => DialectRole::Tool,
        _ => DialectRole::User,
    };
    DialectMessage {
        role,
        content: message.content.clone(),
        extra_metadata: message.extra_metadata.clone(),
    }
}

fn from_dialect_message(message: DialectMessage) -> ChatMessage {
    ChatMessage {
        id: None,
        role: message.role.as_str().to_string(),
        content: message.content,
        extra_metadata: message.extra_metadata,
    }
}

fn to_schemas(specs: &[ToolSpec]) -> Vec<ToolSchema> {
    specs
        .iter()
        .map(|spec| ToolSchema::new(&spec.name, &spec.description, spec.parameters.clone()))
        .collect()
}

fn schemas_from_tools(tools: &[Box<dyn Tool>]) -> Vec<ToolSchema> {
    tools
        .iter()
        .map(|tool| ToolSchema::new(tool.name(), tool.description(), tool.parameters_schema()))
        .collect()
}

fn from_format(
    format: tinyagents_harness::tool_calling::dialect::ToolCallFormat,
) -> ToolCallFormat {
    use tinyagents_harness::tool_calling::dialect::ToolCallFormat as Crate;
    match format {
        Crate::PFormat => ToolCallFormat::PFormat,
        Crate::Json => ToolCallFormat::Json,
        Crate::Native => ToolCallFormat::Native,
    }
}

/// Shared body of every `ToolDispatcher` impl below: convert in, delegate,
/// convert out. Written once as functions rather than repeated per dispatcher
/// so a new dialect is a `impl ToolDispatcher` of five one-line forwards.
fn dispatch_parse(
    dialect: &dyn ToolDialect,
    response: &ChatResponse,
) -> (String, Vec<ParsedToolCall>) {
    let (text, calls) = dialect.parse_response(&to_dialect_response(response));
    (text, from_parsed_calls(calls))
}

fn dispatch_format_results(
    dialect: &dyn ToolDialect,
    results: &[ToolExecutionResult],
) -> ConversationMessage {
    from_transcript_entry(dialect.format_results(&to_outcomes(results)))
}

fn dispatch_provider_messages(
    dialect: &dyn ToolDialect,
    history: &[ConversationMessage],
) -> Vec<ChatMessage> {
    dialect
        .to_provider_messages(&to_transcript(history))
        .into_iter()
        .map(from_dialect_message)
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// The three dispatchers
// ─────────────────────────────────────────────────────────────────────────────

/// Legacy dispatcher using XML-style tags (`<tool_call>`) with JSON bodies.
///
/// This is robust and works well with models that aren't natively trained for
/// tool calling but can follow instructions in a system prompt.
#[derive(Default)]
pub struct XmlToolDispatcher;

impl XmlToolDispatcher {
    /// Extract serializable specs for all tools in the registry.
    pub fn tool_specs(tools: &[Box<dyn Tool>]) -> Vec<ToolSpec> {
        tools.iter().map(|tool| tool.spec()).collect()
    }

    /// Render the protocol block plus the full-schema catalogue.
    pub fn prompt_instructions_from_specs(specs: &[ToolSpec]) -> String {
        XmlDialect::instructions(&to_schemas(specs))
    }

    /// Internal helper to extract tool calls from a raw text string.
    #[cfg(test)]
    pub(crate) fn parse_tool_calls_from_text(response: &str) -> (String, Vec<ParsedToolCall>) {
        let (text, calls) = XmlDialect::parse_text(response);
        (text, from_parsed_calls(calls))
    }
}

impl ToolDispatcher for XmlToolDispatcher {
    fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ParsedToolCall>) {
        dispatch_parse(&XmlDialect, response)
    }

    fn format_results(&self, results: &[ToolExecutionResult]) -> ConversationMessage {
        dispatch_format_results(&XmlDialect, results)
    }

    fn prompt_instructions(&self, tools: &[Box<dyn Tool>]) -> String {
        XmlDialect::instructions(&schemas_from_tools(tools))
    }

    fn prompt_instructions_for_specs(&self, specs: &[ToolSpec]) -> Option<String> {
        Some(Self::prompt_instructions_from_specs(specs))
    }

    fn to_provider_messages(&self, history: &[ConversationMessage]) -> Vec<ChatMessage> {
        dispatch_provider_messages(&XmlDialect, history)
    }

    fn should_send_tool_specs(&self) -> bool {
        XmlDialect.should_send_tool_specs()
    }

    fn tool_call_format(&self) -> ToolCallFormat {
        from_format(XmlDialect.tool_call_format())
    }
}

/// Text-based dispatcher that emits and parses **P-Format** ("Parameter
/// Format") tool calls — the compact `tool_name[arg1|arg2|...]` syntax.
///
/// P-format is designed to significantly reduce token usage compared to JSON.
/// It uses positional arguments based on an alphabetical sort of the tool's
/// parameters, and falls back to the JSON-in-tag parser per tag so a model that
/// mixes the two forms is still understood.
pub struct PFormatToolDispatcher {
    dialect: PFormatDialect,
}

impl PFormatToolDispatcher {
    /// Create a new P-Format dispatcher with the given tool registry.
    pub fn new(registry: PFormatRegistry) -> Self {
        Self {
            dialect: PFormatDialect::new(registry),
        }
    }
}

impl ToolDispatcher for PFormatToolDispatcher {
    fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ParsedToolCall>) {
        dispatch_parse(&self.dialect, response)
    }

    fn format_results(&self, results: &[ToolExecutionResult]) -> ConversationMessage {
        dispatch_format_results(&self.dialect, results)
    }

    fn prompt_instructions(&self, _tools: &[Box<dyn Tool>]) -> String {
        // Protocol description ONLY — the catalogue is rendered by the upstream
        // `ToolsSection`, from the same schemas this dialect parses against.
        PFormatDialect::instructions()
    }

    fn to_provider_messages(&self, history: &[ConversationMessage]) -> Vec<ChatMessage> {
        dispatch_provider_messages(&self.dialect, history)
    }

    fn should_send_tool_specs(&self) -> bool {
        self.dialect.should_send_tool_specs()
    }

    fn tool_call_format(&self) -> ToolCallFormat {
        from_format(self.dialect.tool_call_format())
    }
}

/// Dispatcher for models with native, structured tool-calling support (e.g., OpenAI, Anthropic).
///
/// This dispatcher leverages the provider's built-in APIs for identifying and
/// reporting tool calls, which is generally more reliable than text-based
/// parsing. It still supports a text-based fallback for robustness against
/// models that might "forget" to use the structured API, and drops half-finished
/// tool cycles while serializing so a bisected transcript cannot trip the
/// provider's 400 (TAURI-RUST-7) — see
/// [`pair_tool_cycles`](tinyagents_harness::tool_calling::dialect::pair_tool_cycles).
pub struct NativeToolDispatcher;

impl ToolDispatcher for NativeToolDispatcher {
    fn parse_response(&self, response: &ChatResponse) -> (String, Vec<ParsedToolCall>) {
        dispatch_parse(&NativeDialect, response)
    }

    fn format_results(&self, results: &[ToolExecutionResult]) -> ConversationMessage {
        dispatch_format_results(&NativeDialect, results)
    }

    fn prompt_instructions(&self, _tools: &[Box<dyn Tool>]) -> String {
        NativeDialect.prompt_instructions(&[])
    }

    fn to_provider_messages(&self, history: &[ConversationMessage]) -> Vec<ChatMessage> {
        dispatch_provider_messages(&NativeDialect, history)
    }

    fn should_send_tool_specs(&self) -> bool {
        NativeDialect.should_send_tool_specs()
    }

    fn tool_call_format(&self) -> ToolCallFormat {
        from_format(NativeDialect.tool_call_format())
    }
}

#[cfg(test)]
#[path = "dispatcher_tests.rs"]
mod tests;
