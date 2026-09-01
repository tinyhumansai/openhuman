//! [`ToolMemoryStore`] — the rule store, over the contract's storage trait.
//!
//! # Why this came home (#5560)
//!
//! It was `tinycortex::memory::tool_memory::store::ToolMemoryStore`, and the
//! host reached it through a `pub use`. What it actually *is* is a naming
//! convention plus a sort order applied over an
//! [`Arc<dyn Memory>`](crate::openhuman::memory::Memory) the host already
//! holds — and every load-bearing piece of that convention is in the contract,
//! not in the engine:
//!
//! | Piece | Where it lives |
//! | --- | --- |
//! | namespace (`tool-<name>`) | [`tool_memory_namespace`] |
//! | key (`rule/<id>`) | [`ToolMemoryRule::storage_key`] |
//! | id allocation | [`ToolMemoryRule::generate_id`] |
//! | the record itself | [`ToolMemoryRule`]'s serde |
//! | "which rules go in the prompt" | [`ToolMemoryPriority::is_eager`] |
//!
//! So both ends of the wire already agree on the bytes by construction. The
//! module's own `MemoryToolMemory` implementation writes the same namespace,
//! the same key, and the same JSON — it reaches the *engine's* copy of this
//! file — and neither copy can drift on the parts that decide where a rule
//! lands, because both read them from the contract.
//!
//! ## Why not the `MemoryToolMemory` family instead
//!
//! Because of *which store* the two callers must reach. Both
//! [`capture::ToolMemoryCaptureHook`](super::capture::ToolMemoryCaptureHook)
//! and the session builder's prompt prefetch are handed a **subtree-scoped**
//! `Arc<dyn Memory>` — `DriverMemory::for_subtree(config, memory_subdir)`,
//! resolved per session so a profile with `dedicatedMemory` writes its rules
//! into `memory-<id>` and not into the shared tree. `active_memory_guard()`
//! resolves the *ambient* binding, which is the shared tree; routing either
//! caller onto it would silently merge every dedicated profile's tool rules
//! into one namespace, which is the isolation `dedicatedMemory` exists to
//! provide.
//!
//! The family reached through the session's own subtree binding
//! (`binding::for_subtree(..).provider().as_tool_memory()`) would be correct on
//! that axis, and is the right destination once the hook constructors take a
//! subtree rather than a memory object. It is not this change: both
//! constructors are called from the session builder with the `Arc<dyn Memory>`
//! it already built, and swapping the argument is a harness change rather than
//! a compile-target one. Note also that the *guarded* family is not a drop-in
//! here even then — it applies `enforce_write`, so a `readonly` autonomy tier
//! would start refusing post-turn capture, which is best-effort telemetry and
//! has never been tier-gated.
//!
//! What this file therefore is: the same convention, over the same object, with
//! nothing from the engine crate in it.
//!
//! ## The surface is narrowed to what the host actually calls
//!
//! The engine's store also carries `delete_rule` and `list_rules_json`. Neither
//! is reached from here any more — the `memory_tools_*` RPC and agent tools go
//! through `MemoryToolMemory` on the guard
//! (`memory/ops/tool_memory.rs`, `memory/tools/tool_memory/`) — so they are not
//! reimplemented. A method with no caller is a second definition waiting to
//! disagree with the first.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::openhuman::memory::api::tool_memory::{
    tool_memory_namespace, ToolMemoryPriority, ToolMemoryRule,
};
use crate::openhuman::memory::{Memory, MemoryCategory};

/// How many eager rules the prompt block may carry.
///
/// A cap on the **High**-priority remainder only: every `Critical` rule is
/// retained, so the returned set can exceed this when a tool has more than
/// thirty critical rules. That asymmetry is the point — a critical rule is a
/// safety constraint, and dropping one to fit a budget is the failure this
/// surface exists to prevent.
pub const TOOL_MEMORY_PROMPT_CAP: usize = 30;

/// Namespace prefix every tool-scoped rule namespace carries.
///
/// Only used to *recognise* one in [`ToolMemoryStore::list_tool_names`];
/// namespaces are always **built** with [`tool_memory_namespace`], never with
/// this constant, so the trim-and-lowercase rule stays in one place.
const TOOL_NAMESPACE_PREFIX: &str = "tool-";

/// The tool name a user edict is filed under when no tool ran in the turn.
///
/// Excluded from prompt prefetch: such a rule is not permanently associated
/// with any real tool, so injecting it into an arbitrary session's prompt would
/// pin guidance against a tool the user never mentioned.
const UNSCOPED_TOOL: &str = "__unscoped__";

/// Serialises the read-modify-write inside [`ToolMemoryStore::put_rule`].
///
/// The engine's store held a process-wide lock here for the same reason: the
/// `created_at`-preserving upsert is a `get` followed by a `store`, and two
/// interleaved upserts of the same rule id can otherwise resurrect the older
/// `created_at`. Process-wide rather than per-store because a `ToolMemoryStore`
/// is cheap to clone and several live handles routinely front one backend.
fn rule_mutation_lock() -> &'static Mutex<()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// High-level store for tool-scoped memory rules.
///
/// All methods operate on a single shared [`Arc<dyn Memory>`] backend. Cheap to
/// clone — the backend is reference-counted.
#[derive(Clone)]
pub struct ToolMemoryStore {
    memory: Arc<dyn Memory>,
}

impl ToolMemoryStore {
    /// Build a new store over the given memory backend.
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        Self { memory }
    }

    /// Upsert a rule and return the stored copy, with `updated_at` refreshed.
    ///
    /// A rule with the same `(tool_name, id)` keeps its original `created_at`.
    /// `tool_name` is sourced from the rule itself rather than from a separate
    /// argument, so the namespace it is stored under and the name it displays
    /// as cannot skew; it is trimmed and lower-cased before the write, which is
    /// the same normalisation [`tool_memory_namespace`] applies, so a read-back
    /// with the caller's raw name hits the same namespace. Legacy mixed-case
    /// rows are left alone until something rewrites them.
    ///
    /// # Errors
    ///
    /// A blank `tool_name` or rule body, a serialisation failure, or any
    /// backend failure. Reported as `String` rather than as a typed error
    /// because that is what both callers log — neither branches on the cause.
    pub async fn put_rule(&self, mut rule: ToolMemoryRule) -> Result<ToolMemoryRule, String> {
        if rule.tool_name.trim().is_empty() {
            return Err("tool_name is required".to_string());
        }
        if rule.rule.trim().is_empty() {
            return Err("rule body is required".to_string());
        }
        if rule.id.trim().is_empty() {
            rule.id = ToolMemoryRule::generate_id();
        }
        rule.tool_name = rule.tool_name.trim().to_lowercase();

        let _guard = rule_mutation_lock().lock().await;

        let namespace = tool_memory_namespace(&rule.tool_name);
        let key = ToolMemoryRule::storage_key(&rule.id);

        if let Some(existing) = self.fetch_rule(&namespace, &key).await? {
            rule.created_at = existing.created_at;
        }
        rule.updated_at = chrono::Utc::now().to_rfc3339();

        let content = serde_json::to_string(&rule).map_err(|e| e.to_string())?;
        self.memory
            .store(
                &namespace,
                &key,
                &content,
                MemoryCategory::Custom("tool_memory".into()),
                None,
            )
            .await
            .map_err(|e| format!("store tool rule: {e:#}"))?;
        Ok(rule)
    }

    /// Build a rule from caller-supplied fields and persist it.
    ///
    /// The id is minted by [`ToolMemoryRule::new`], so this is always an
    /// insert — the `created_at` preservation in [`Self::put_rule`] is
    /// unreachable from here.
    ///
    /// # Errors
    ///
    /// As [`Self::put_rule`].
    pub async fn record(
        &self,
        tool_name: &str,
        rule_body: &str,
        priority: ToolMemoryPriority,
        source: crate::openhuman::memory::api::tool_memory::ToolMemorySource,
        tags: Vec<String>,
    ) -> Result<ToolMemoryRule, String> {
        let mut rule = ToolMemoryRule::new(tool_name, rule_body, priority, source);
        rule.tags = tags;
        self.put_rule(rule).await
    }

    /// Every rule for one tool, highest priority first and freshest first
    /// within a priority.
    ///
    /// A row that will not deserialise is skipped rather than failing the list:
    /// one corrupt entry must not hide every other rule for that tool from the
    /// prompt, and there is nothing a caller could do with the error but log it.
    ///
    /// # Errors
    ///
    /// Backend failures only; a tool with no rules yields an empty vector.
    pub async fn list_rules(&self, tool_name: &str) -> Result<Vec<ToolMemoryRule>, String> {
        let namespace = tool_memory_namespace(tool_name);
        let entries = self
            .memory
            .list(Some(&namespace), None, None)
            .await
            .map_err(|e| format!("list tool rules: {e:#}"))?;

        let mut rules: Vec<ToolMemoryRule> = entries
            .into_iter()
            .filter(|entry| entry.key.starts_with("rule/"))
            .filter_map(|entry| serde_json::from_str::<ToolMemoryRule>(&entry.content).ok())
            .collect();

        rules.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });

        Ok(rules)
    }

    /// Fetch a single rule by `(tool_name, rule_id)`.
    ///
    /// `Ok(None)` means the rule is absent, which is not an error: the RPC
    /// surface answers a missing rule with a null body rather than a failure.
    ///
    /// # Errors
    ///
    /// Backend failures only.
    pub async fn get_rule(
        &self,
        tool_name: &str,
        rule_id: &str,
    ) -> Result<Option<ToolMemoryRule>, String> {
        let namespace = tool_memory_namespace(tool_name);
        let key = ToolMemoryRule::storage_key(rule_id);
        self.fetch_rule(&namespace, &key).await
    }

    /// Delete a rule. Returns `true` when the rule existed.
    ///
    /// # Errors
    ///
    /// Backend failures only; deleting an absent rule is `Ok(false)`.
    pub async fn delete_rule(&self, tool_name: &str, rule_id: &str) -> Result<bool, String> {
        let namespace = tool_memory_namespace(tool_name);
        let key = ToolMemoryRule::storage_key(rule_id);
        self.memory
            .forget(&namespace, &key)
            .await
            .map_err(|e| format!("forget tool rule: {e:#}"))
    }

    /// Render one tool's rules as JSON for an RPC envelope, priority
    /// descending.
    ///
    /// The shape is `serde_json::to_value` over the same `Vec<ToolMemoryRule>`
    /// [`Self::list_rules`] returns, so `tool_rules_json` stays byte-compatible
    /// with what the dashboard already parses.
    ///
    /// # Errors
    ///
    /// Backend failures, or a serialisation failure that cannot occur for this
    /// type but is surfaced rather than unwrapped.
    pub async fn list_rules_json(&self, tool_name: &str) -> Result<serde_json::Value, String> {
        let rules = self.list_rules(tool_name).await?;
        serde_json::to_value(rules).map_err(|e| e.to_string())
    }

    /// The rules that must be surfaced eagerly (Critical + High), grouped by
    /// tool name.
    ///
    /// `tools` constrains which tool namespaces to inspect; an empty slice
    /// scans every known tool namespace via [`Self::list_tool_names`].
    ///
    /// Sorted Critical-first then freshest-first, and truncated at
    /// [`TOOL_MEMORY_PROMPT_CAP`] exactly as the engine did — which means a
    /// Critical rule CAN fall off once that many fresher Critical+High rules
    /// exist. That is the engine's documented trade-off, preserved.
    ///
    /// # Errors
    ///
    /// Backend failures only.
    pub async fn rules_for_prompt(
        &self,
        tools: &[String],
    ) -> Result<HashMap<String, Vec<ToolMemoryRule>>, String> {
        let tool_names = if tools.is_empty() {
            self.list_tool_names().await?
        } else {
            tools
                .iter()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect()
        };

        let mut collected: Vec<ToolMemoryRule> = Vec::new();
        for tool in &tool_names {
            let rules = self.list_rules(tool).await?;
            collected.extend(rules.into_iter().filter(|r| r.priority.is_eager()));
        }

        // Critical first, then High; within a priority, freshest first.
        collected.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });
        // The engine enforced this cap with a plain `truncate` after sorting —
        // its own doc on `TOOL_MEMORY_PROMPT_CAP` admits a Critical rule can be
        // excluded once that many fresher Critical+High rules exist. The port
        // briefly "improved" this to keep every Critical, which was a silent
        // behaviour change in a behaviour-pinned move; the engine's semantics
        // are restored, and the cap doc no longer overpromises.
        collected.truncate(TOOL_MEMORY_PROMPT_CAP);

        let mut out: HashMap<String, Vec<ToolMemoryRule>> = HashMap::new();
        for rule in collected {
            out.entry(rule.tool_name.clone()).or_default().push(rule);
        }
        Ok(out)
    }

    /// Every tool that has at least one stored rule.
    ///
    /// # Errors
    ///
    /// Backend failures only.
    pub async fn list_tool_names(&self) -> Result<Vec<String>, String> {
        let summaries = self
            .memory
            .namespace_summaries()
            .await
            .map_err(|e| format!("list tool namespaces: {e:#}"))?;
        let mut out = Vec::new();
        for summary in summaries {
            if let Some(tool) = summary.namespace.strip_prefix(TOOL_NAMESPACE_PREFIX) {
                if !tool.is_empty() && tool != UNSCOPED_TOOL {
                    out.push(tool.to_string());
                }
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// One rule by `(namespace, key)`, or `None`.
    ///
    /// A malformed row reads as absent so a corrupt entry cannot block the
    /// upsert that would replace it.
    async fn fetch_rule(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<ToolMemoryRule>, String> {
        let entry = self
            .memory
            .get(namespace, key)
            .await
            .map_err(|e| format!("get tool rule: {e:#}"))?;
        Ok(entry.and_then(|entry| serde_json::from_str::<ToolMemoryRule>(&entry.content).ok()))
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
