//! Core type definitions for the TokenJuice reduction engine.
//!
//! These types mirror the upstream TypeScript shapes so that upstream rule JSON
//! files can be loaded without modification.  All public types use
//! `#[serde(rename_all = "camelCase")]` and `#[serde(default)]` on optional
//! fields for maximum compatibility with the upstream schema.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Rule origin
// ---------------------------------------------------------------------------

/// Which configuration layer a rule was loaded from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuleOrigin {
    Builtin,
    User,
    Project,
}

// ---------------------------------------------------------------------------
// Rule sub-types
// ---------------------------------------------------------------------------

/// Matching criteria for a rule.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleMatch {
    /// Match when `toolName` is one of these values.
    #[serde(default)]
    pub tool_names: Option<Vec<String>>,
    /// Match when `argv[0]` is one of these values.
    #[serde(default)]
    pub argv0: Option<Vec<String>>,
    /// All of these groups must each appear somewhere in `argv`.
    #[serde(default)]
    pub argv_includes: Option<Vec<Vec<String>>>,
    /// At least one of these groups must appear in `argv`.
    #[serde(default)]
    pub argv_includes_any: Option<Vec<Vec<String>>>,
    /// All of these strings must appear in `command`.
    #[serde(default)]
    pub command_includes: Option<Vec<String>>,
    /// At least one of these strings must appear in `command`.
    #[serde(default)]
    pub command_includes_any: Option<Vec<String>>,
}

/// Line-level filter patterns.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleFilters {
    /// Lines matching any pattern are removed.
    #[serde(default)]
    pub skip_patterns: Option<Vec<String>>,
    /// Only lines matching at least one pattern are kept (if any match).
    #[serde(default)]
    pub keep_patterns: Option<Vec<String>>,
}

/// Output transformation flags.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleTransforms {
    #[serde(default)]
    pub strip_ansi: Option<bool>,
    #[serde(default)]
    pub trim_empty_edges: Option<bool>,
    #[serde(default)]
    pub dedupe_adjacent: Option<bool>,
    #[serde(default)]
    pub pretty_print_json: Option<bool>,
}

/// Head/tail summarisation parameters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleSummarize {
    #[serde(default)]
    pub head: Option<usize>,
    #[serde(default)]
    pub tail: Option<usize>,
}

/// A pattern-based line counter.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleCounter {
    pub name: String,
    pub pattern: String,
    /// Regex flags (e.g. `"i"` for case-insensitive). `u` is always added.
    #[serde(default)]
    pub flags: Option<String>,
}

/// Map output patterns to canned messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleOutputMatch {
    pub pattern: String,
    pub message: String,
    #[serde(default)]
    pub flags: Option<String>,
}

/// Failure-mode overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleFailure {
    #[serde(default)]
    pub preserve_on_failure: Option<bool>,
    #[serde(default)]
    pub head: Option<usize>,
    #[serde(default)]
    pub tail: Option<usize>,
}

// ---------------------------------------------------------------------------
// JsonRule — the raw deserialized form
// ---------------------------------------------------------------------------

/// A rule as parsed from a JSON file (upstream `JsonRule`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonRule {
    pub id: String,
    pub family: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
    /// Message to return when output is empty after filtering.
    #[serde(default)]
    pub on_empty: Option<String>,
    #[serde(default)]
    pub match_output: Option<Vec<RuleOutputMatch>>,
    /// Whether counters run before or after keep-pattern filtering.
    /// Upstream default is `"postKeep"`.
    #[serde(default)]
    pub counter_source: Option<CounterSource>,
    pub r#match: RuleMatch,
    #[serde(default)]
    pub filters: Option<RuleFilters>,
    #[serde(default)]
    pub transforms: Option<RuleTransforms>,
    #[serde(default)]
    pub summarize: Option<RuleSummarize>,
    #[serde(default)]
    pub counters: Option<Vec<RuleCounter>>,
    #[serde(default)]
    pub failure: Option<RuleFailure>,
}

/// When to sample lines for counters — before or after keep-pattern filtering.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum CounterSource {
    PostKeep,
    PreKeep,
}

// ---------------------------------------------------------------------------
// CompiledRule — regex patterns pre-built
// ---------------------------------------------------------------------------

/// A compiled counter entry with the pattern pre-built.
#[derive(Debug, Clone)]
pub struct CompiledCounter {
    pub name: String,
    pub pattern: regex::Regex,
}

/// A compiled output-match entry.
#[derive(Debug, Clone)]
pub struct CompiledOutputMatch {
    pub pattern: regex::Regex,
    pub message: String,
}

/// The compiled form of a rule (regex patterns pre-built at load time).
#[derive(Debug, Clone)]
pub struct CompiledParts {
    pub skip_patterns: Vec<regex::Regex>,
    pub keep_patterns: Vec<regex::Regex>,
    pub counters: Vec<CompiledCounter>,
    pub output_matches: Vec<CompiledOutputMatch>,
}

/// A `JsonRule` paired with its pre-compiled regex patterns plus provenance.
#[derive(Debug, Clone)]
pub struct CompiledRule {
    pub rule: JsonRule,
    pub source: RuleOrigin,
    /// Filesystem path (or `"builtin:<id>"` for embedded rules).
    pub path: String,
    pub compiled: CompiledParts,
}

// ---------------------------------------------------------------------------
// ToolExecutionInput
// ---------------------------------------------------------------------------

/// Describes a tool invocation whose output is to be reduced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecutionInput {
    pub tool_name: String,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub argv: Option<Vec<String>>,
    #[serde(default)]
    pub args: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub partial: Option<bool>,
    #[serde(default)]
    pub stdout: Option<String>,
    #[serde(default)]
    pub stderr: Option<String>,
    #[serde(default)]
    pub combined_text: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub started_at: Option<f64>,
    #[serde(default)]
    pub finished_at: Option<f64>,
    #[serde(default)]
    pub duration_ms: Option<f64>,
    #[serde(default)]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

// ---------------------------------------------------------------------------
// ReduceOptions
// ---------------------------------------------------------------------------

/// Options for the `reduce_execution` pipeline.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReduceOptions {
    /// Force a specific rule ID instead of auto-classification.
    #[serde(default)]
    pub classifier: Option<String>,
    /// Maximum inline character count (default: 1200).
    #[serde(default)]
    pub max_inline_chars: Option<usize>,
    /// Return raw text without reduction.
    #[serde(default)]
    pub raw: Option<bool>,
    /// Working directory for project-layer rule discovery.
    #[serde(default)]
    pub cwd: Option<String>,
}

// ---------------------------------------------------------------------------
// CompactResult
// ---------------------------------------------------------------------------

/// Statistics produced by the reduction pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReductionStats {
    pub raw_chars: usize,
    pub reduced_chars: usize,
    pub ratio: f64,
}

/// The classification decision made during reduction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassificationResult {
    pub family: String,
    pub confidence: f64,
    #[serde(default)]
    pub matched_reducer: Option<String>,
}

/// The output of `reduce_execution`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactResult {
    /// The compacted text to inline into LLM context.
    pub inline_text: String,
    /// A shorter preview (the intermediate summary before clamping).
    #[serde(default)]
    pub preview_text: Option<String>,
    /// Named counts extracted by counters.
    #[serde(default)]
    pub facts: Option<HashMap<String, usize>>,
    pub stats: ReductionStats,
    pub classification: ClassificationResult,
}

/// Per-agent TokenJuice profile.
///
/// `Auto` is resolved by the agent definition layer. TokenJuice itself treats
/// `Auto` like `Full` so non-agent callers keep the global `[tokenjuice]`
/// behaviour unless they explicitly pass a narrower profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentTokenjuiceCompression {
    /// Let the agent definition/runtime choose. Coding agents resolve this to
    /// [`Self::Light`]; other agents resolve to [`Self::Full`].
    #[default]
    Auto,
    /// Use the process-global TokenJuice configuration unchanged.
    Full,
    /// Keep only non-lossy reductions; disables CCR-backed lossy compaction.
    Light,
    /// Bypass TokenJuice for this agent's tool results.
    Off,
}

impl AgentTokenjuiceCompression {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Full => "full",
            Self::Light => "light",
            Self::Off => "off",
        }
    }
}

// ---------------------------------------------------------------------------
// Content Router (TokenJuice 2.0) — content-kind detection + compressor dispatch
// ---------------------------------------------------------------------------

/// The kind of content a blob holds, as decided by the detector. Drives which
/// [`crate::openhuman::tokenjuice::compressors::Compressor`] the router picks.
///
/// Inspired by Headroom's content router: each kind has a specialised
/// compressor tuned to preserve the signal that kind carries (errors in logs,
/// changed hunks in diffs, signatures in code, …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContentKind {
    /// JSON array/object payload → tabular SmartCrusher.
    Json,
    /// Source code → AST/heuristic signature keeper.
    Code,
    /// Build / test / lint log → keep failures, drop passing noise.
    Log,
    /// grep / ripgrep style `path:line:content` matches → relevance rank.
    Search,
    /// Unified git diff / patch → keep changed hunks, collapse context.
    Diff,
    /// HTML document → strip markup to readable text.
    Html,
    /// Anything else → ML text compressor (if enabled) or pass-through.
    PlainText,
}

impl ContentKind {
    /// Stable lower-case label for logs / RPC / stats.
    pub fn as_str(self) -> &'static str {
        match self {
            ContentKind::Json => "json",
            ContentKind::Code => "code",
            ContentKind::Log => "log",
            ContentKind::Search => "search",
            ContentKind::Diff => "diff",
            ContentKind::Html => "html",
            ContentKind::PlainText => "plain_text",
        }
    }
}

/// A caller-supplied prior about a blob's content, so the detector doesn't have
/// to work from scratch. Any field may be `None`; the detector resolves what it
/// can and falls back to structural heuristics. An `explicit` kind is a hard
/// override and skips detection entirely.
#[derive(Debug, Clone, Default)]
pub struct ContentHint {
    /// MIME type if known (`text/html`, `application/json`, …).
    pub mime: Option<String>,
    /// File extension without the dot (`rs`, `ts`, `py`, `json`, `html`, `diff`).
    pub extension: Option<String>,
    /// The agent-level tool that produced the content (`grep`, `run_tests`, …).
    pub source_tool: Option<String>,
    /// A search/query string associated with the content, when known (used by
    /// the search compressor to rank matches by query-term density).
    pub query: Option<String>,
    /// Hard override — when set, detection returns this kind verbatim.
    pub explicit: Option<ContentKind>,
}

impl ContentHint {
    /// Convenience: a hint carrying only the producing tool name.
    pub fn for_tool(tool_name: impl Into<String>) -> Self {
        Self {
            source_tool: Some(tool_name.into()),
            ..Default::default()
        }
    }
}

/// Which compressor actually produced an output. Recorded in stats / logs so a
/// human (or the debug controller) can see what the router chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CompressorKind {
    /// JSON array→table crusher.
    SmartCrusher,
    /// AST/heuristic code-signature keeper.
    Code,
    /// Log keep-failures compressor (and the rule engine for command output).
    Log,
    /// Search relevance ranker.
    Search,
    /// Unified-diff context collapser.
    Diff,
    /// HTML→text extractor.
    Html,
    /// ML (Python/ModernBERT) plain-text compressor.
    MlText,
    /// Line-oriented head/tail fallback.
    Generic,
    /// No compressor fired — pass-through.
    None,
}

impl CompressorKind {
    /// Stable lower-case label for stats / logs / RPC.
    pub fn as_str(self) -> &'static str {
        match self {
            CompressorKind::SmartCrusher => "smartcrusher",
            CompressorKind::Code => "code",
            CompressorKind::Log => "log",
            CompressorKind::Search => "search",
            CompressorKind::Diff => "diff",
            CompressorKind::Html => "html",
            CompressorKind::MlText => "ml_text",
            CompressorKind::Generic => "generic",
            CompressorKind::None => "none",
        }
    }
}

/// Input handed to a [`crate::openhuman::tokenjuice::compressors::Compressor`].
/// Borrows the content to avoid copies on the hot path.
#[derive(Debug, Clone)]
pub struct CompressInput<'a> {
    /// The raw content to compress.
    pub content: &'a str,
    /// The detected (or hinted) content kind.
    pub kind: ContentKind,
    /// The original caller hint (carries `query`, `source_tool`, argv-derived
    /// command for the log/command path, …).
    pub hint: &'a ContentHint,
    /// Process exit code if this is command output — enables failure-preserving
    /// behaviour in the log compressor.
    pub exit_code: Option<i32>,
    /// Derived shell command (joined argv) for the log/command rule path, if any.
    pub command: Option<String>,
    /// Derived argv for the log/command rule path, if any.
    pub argv: Option<Vec<String>>,
    /// Original byte length (== `content.len()`; cached for convenience).
    pub original_bytes: usize,
}

/// Output of a compressor. `text` is the compacted body **without** any CCR
/// retrieval footer — the router ([`crate::openhuman::tokenjuice::compress`])
/// adds the marker after offloading the original.
#[derive(Debug, Clone)]
pub struct CompressOutput {
    /// The compacted body.
    pub text: String,
    /// True when data was dropped (vs. a faithful reformat). Changes the footer
    /// wording and whether the original is mandatory for fidelity.
    pub lossy: bool,
    /// Which compressor produced this.
    pub kind: CompressorKind,
    /// Optional named counts (e.g. error/warning tallies).
    pub facts: Option<HashMap<String, usize>>,
}

impl CompressOutput {
    /// A faithful reformat — every value preserved, only layout changed.
    pub fn reformatted(text: String, kind: CompressorKind) -> Self {
        Self {
            text,
            lossy: false,
            kind,
            facts: None,
        }
    }

    /// A lossy view — data was dropped; the original must be offloaded for recovery.
    pub fn lossy(text: String, kind: CompressorKind) -> Self {
        Self {
            text,
            lossy: true,
            kind,
            facts: None,
        }
    }
}

/// Knobs for the router and compressors, built by the caller from the
/// `[tokenjuice]` config block. TokenJuice stays decoupled from the config
/// schema crate by taking this plain struct rather than `Config`.
#[derive(Debug, Clone)]
pub struct CompressOptions {
    /// Master switch — when false, [`crate::openhuman::tokenjuice::compress_content`]
    /// is a pass-through.
    pub router_enabled: bool,
    /// Whether to offload originals to CCR and emit retrieval markers.
    pub ccr_enabled: bool,
    /// Per-compressor toggles.
    pub search_enabled: bool,
    pub code_enabled: bool,
    pub html_enabled: bool,
    /// Whether the ML plain-text compressor may be used (further gated at
    /// runtime by Python/runtime_python_server availability).
    pub ml_text_enabled: bool,
    /// Outputs below this many bytes are never compressed.
    pub min_bytes_to_compress: usize,
    /// CCR only fires (offload original + lossy compression) when the input is
    /// estimated to be at least this many tokens. Below it, the result passes
    /// through (lossless reformats may still apply without offload). Lets small
    /// tool results skip the cache entirely.
    pub ccr_min_tokens: usize,
    /// Maximum inline character count for the generic/rule fallback path.
    pub max_inline_chars: Option<usize>,
}

impl Default for CompressOptions {
    fn default() -> Self {
        Self {
            router_enabled: true,
            ccr_enabled: true,
            search_enabled: true,
            code_enabled: true,
            html_enabled: true,
            ml_text_enabled: false,
            min_bytes_to_compress: 2048,
            ccr_min_tokens: 500,
            max_inline_chars: None,
        }
    }
}

/// The result of the universal [`crate::openhuman::tokenjuice::compress_content`]
/// entry point: the compacted text (with any CCR footer already appended), plus
/// metadata for callers/stats.
#[derive(Debug, Clone)]
pub struct CompressedOutput {
    /// Final text to inline into context (includes the retrieval footer when lossy).
    pub text: String,
    /// The detected content kind.
    pub content_kind: ContentKind,
    /// Which compressor fired (`None` ⇒ pass-through).
    pub compressor: CompressorKind,
    /// Whether the output dropped data.
    pub lossy: bool,
    /// True if the router actually changed the content.
    pub applied: bool,
    /// CCR token for the offloaded original, if one was stored.
    pub ccr_token: Option<String>,
    /// Original byte length.
    pub original_bytes: usize,
    /// Compacted byte length (of `text`).
    pub compacted_bytes: usize,
}

impl CompressedOutput {
    /// Build a pass-through result that didn't change `content`.
    pub fn passthrough(content: String, kind: ContentKind) -> Self {
        let len = content.len();
        Self {
            text: content,
            content_kind: kind,
            compressor: CompressorKind::None,
            lossy: false,
            applied: false,
            ccr_token: None,
            original_bytes: len,
            compacted_bytes: len,
        }
    }
}

// ---------------------------------------------------------------------------
// RuleFixture — used by integration tests
// ---------------------------------------------------------------------------

/// A test fixture mirroring the upstream `RuleFixture` shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleFixture {
    pub input: ToolExecutionInput,
    pub expected_output: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub options: Option<ReduceOptions>,
}
