//! Shared plumbing for the tiny.place agent flow tools.
//!
//! * [`FlowTool`] — a thin, function-pointer-backed [`Tool`] so each flow is a
//!   single free `async fn` plus a one-line registration, instead of a
//!   hand-written `impl Tool` per flow.
//! * Argument helpers ([`req_str`], [`opt_str`], …) for pulling typed values out
//!   of the LLM's JSON arguments.
//! * Client / signer access ([`client`], [`agent_id`]) over the process-global
//!   [`TinyPlaceState`](super::super::state::TinyPlaceState).
//! * Result builders ([`ok_md`], [`err_md`]) and SDK-error rendering
//!   ([`sdk_error`]) so the LLM only ever sees markdown — including the
//!   fund-and-retry guidance synthesised from an x402 `402` challenge.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

use async_trait::async_trait;
use serde_json::{Map, Value};

use crate::core::all::ControllerHandler;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

use super::render::Markdown;
use super::suggest::{append_next_steps, Suggestion};

pub(super) const LOG_PREFIX: &str = "[tinyplace][flow]";

// ── FlowTool ──────────────────────────────────────────────────────────────────

/// Boxed future returned by a flow function.
pub type FlowFuture = Pin<Box<dyn Future<Output = anyhow::Result<ToolResult>> + Send>>;

/// A flow is a plain function from JSON args to a (markdown) tool result.
pub type FlowFn = fn(Value) -> FlowFuture;

/// Generic agent tool wrapping one flow function with its metadata.
///
/// Read flows (`ReadOnly`, concurrency-safe, no external effect) and write
/// flows (`Write`, external effect → routed through the approval gate) share
/// this type; only the permission knobs differ. Flows whose gating depends on
/// arguments (e.g. the raw escape hatch) implement [`Tool`] directly instead.
pub struct FlowTool {
    name: &'static str,
    description: &'static str,
    schema: Value,
    permission: PermissionLevel,
    external: bool,
    concurrency_safe: bool,
    run: FlowFn,
}

impl FlowTool {
    /// A read-only flow: no approval gate, safe to run concurrently.
    pub fn read(name: &'static str, description: &'static str, schema: Value, run: FlowFn) -> Self {
        Self {
            name,
            description,
            schema,
            permission: PermissionLevel::ReadOnly,
            external: false,
            concurrency_safe: true,
            run,
        }
    }

    /// A write flow: routed through the `ApprovalGate` (external effect) and
    /// never run concurrently with itself.
    pub fn write(
        name: &'static str,
        description: &'static str,
        schema: Value,
        run: FlowFn,
    ) -> Self {
        Self {
            name,
            description,
            schema,
            permission: PermissionLevel::Write,
            external: true,
            concurrency_safe: false,
            run,
        }
    }

    pub fn boxed(self) -> Box<dyn Tool> {
        Box::new(self)
    }
}

#[async_trait]
impl Tool for FlowTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        self.description
    }

    fn parameters_schema(&self) -> Value {
        self.schema.clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let args = match args {
            Value::Null => Value::Object(Default::default()),
            other => other,
        };
        log::debug!("{LOG_PREFIX} {} start", self.name);
        let result = (self.run)(args).await;
        match &result {
            Ok(r) if r.is_error => log::warn!("{LOG_PREFIX} {} returned error result", self.name),
            Ok(_) => log::debug!("{LOG_PREFIX} {} ok", self.name),
            Err(e) => log::warn!("{LOG_PREFIX} {} failed: {e}", self.name),
        }
        result
    }

    fn permission_level(&self) -> PermissionLevel {
        self.permission
    }

    fn external_effect(&self) -> bool {
        self.external
    }

    fn is_concurrency_safe(&self, _args: &Value) -> bool {
        self.concurrency_safe
    }

    fn supports_markdown(&self) -> bool {
        true
    }

    fn max_result_size_chars(&self) -> Option<usize> {
        Some(48 * 1024)
    }
}

// ── Argument helpers ────────────────────────────────────────────────────────

/// Required, non-empty string argument.
pub fn req_str(args: &Value, key: &str) -> anyhow::Result<String> {
    opt_str(args, key).ok_or_else(|| anyhow::anyhow!("missing required parameter '{key}'"))
}

/// Optional, trimmed, non-empty string argument.
pub fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Optional integer argument (accepts JSON numbers and numeric strings).
pub fn opt_i64(args: &Value, key: &str) -> Option<i64> {
    let v = args.get(key)?;
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Optional boolean argument (accepts JSON bools and `"true"`/`"false"`).
pub fn opt_bool(args: &Value, key: &str) -> Option<bool> {
    let v = args.get(key)?;
    v.as_bool()
        .or_else(|| match v.as_str()?.trim().to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
}

/// Optional list-of-strings argument. Accepts a JSON array of strings or a
/// single comma-separated string. Returns `None` when absent/empty.
pub fn opt_str_list(args: &Value, key: &str) -> Option<Vec<String>> {
    match args.get(key)? {
        Value::Array(items) => {
            let list: Vec<String> = items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            (!list.is_empty()).then_some(list)
        }
        Value::String(s) => {
            let list: Vec<String> = s
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            (!list.is_empty()).then_some(list)
        }
        _ => None,
    }
}

// ── Client / signer access ──────────────────────────────────────────────────

/// Process-global tiny.place client, built lazily from the wallet signer.
pub async fn client() -> anyhow::Result<&'static tinyplace::TinyPlaceClient> {
    super::super::ops::global_state()
        .client()
        .await
        .map_err(|e| anyhow::anyhow!("tiny.place client unavailable: {e}"))
}

/// The caller's own agent id, resolved from the wallet signer.
///
/// Identity is **always** taken from the signer, never from tool arguments —
/// the agent cannot act as anyone but itself (anti-spoof).
pub fn agent_id(client: &tinyplace::TinyPlaceClient) -> anyhow::Result<String> {
    client
        .http()
        .signer()
        .map(|s| s.agent_id())
        .ok_or_else(|| anyhow::anyhow!("tiny.place signer unavailable; unlock your wallet"))
}

/// The caller's own base64 public (encryption) key, from the wallet signer.
pub fn public_key(client: &tinyplace::TinyPlaceClient) -> anyhow::Result<String> {
    client
        .http()
        .signer()
        .map(|s| s.public_key_base64())
        .ok_or_else(|| anyhow::anyhow!("tiny.place signer unavailable; unlock your wallet"))
}

/// Resolve a `@handle` (or bare handle) to its wallet cryptoId, falling back to
/// the input unchanged when it is already an id or can't be resolved. Used by
/// the follow/unfollow flows so the agent can pass a friendly handle.
pub async fn resolve_agent(client: &tinyplace::TinyPlaceClient, target: &str) -> String {
    let name = target.trim().trim_start_matches('@');
    if name.is_empty() {
        return target.trim().to_string();
    }
    match client.directory.resolve(name).await {
        Ok(resolved) => resolved
            .identity
            .map(|i| i.crypto_id)
            .filter(|id| !id.is_empty())
            .or_else(|| resolved.agent.map(|a| a.agent_id))
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| target.trim().to_string()),
        Err(_) => target.trim().to_string(),
    }
}

// ── Result builders ─────────────────────────────────────────────────────────

fn md_result(markdown: String, is_error: bool) -> ToolResult {
    let base = if is_error {
        ToolResult::error(markdown.clone())
    } else {
        ToolResult::success(markdown.clone())
    };
    // Populate `markdown_formatted` too so the loop's markdown path is
    // consistent; the primary content is already markdown so the LLM sees it
    // regardless of whether `prefer_markdown` is set.
    base.with_markdown(markdown)
}

/// A successful flow result whose body is markdown.
pub fn ok_md(markdown: String) -> ToolResult {
    md_result(markdown, false)
}

/// A failed flow result whose body is markdown (the LLM still sees the text).
pub fn err_md(markdown: String) -> ToolResult {
    md_result(markdown, true)
}

/// Build a markdown document then optionally append a `## Next steps` block.
pub fn finish(mut md: Markdown, suggestions: &[Suggestion]) -> anyhow::Result<ToolResult> {
    append_next_steps(&mut md, suggestions);
    Ok(ok_md(md.build()))
}

/// Recursively collect every string value stored under `key` anywhere in a JSON
/// tree, de-duplicated in first-seen order. Used to harvest ids (e.g. `postId`,
/// `bountyId`) from a serialised SDK result so flows can pre-fill follow-up
/// suggestions.
pub fn collect_field(value: &Value, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    collect_into(value, key, &mut out);
    out
}

fn collect_into(value: &Value, key: &str, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                if k == key {
                    if let Some(s) = v.as_str() {
                        let s = s.to_string();
                        if !s.is_empty() && !out.contains(&s) {
                            out.push(s);
                        }
                    }
                }
                collect_into(v, key, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_into(item, key, out);
            }
        }
        _ => {}
    }
}

/// Render an SDK error as a markdown **string** the agent can act on.
///
/// An x402 `402` becomes a **Payment required** section with fund-and-retry
/// guidance (mirroring the CLI's `status: payment-required`); any other error
/// is surfaced through [`map_err`](super::super::ops::map_err) which carries
/// the backend's reason string.
pub fn sdk_error_text(action: &str, err: tinyplace::Error) -> String {
    if let Some(challenge) = err.payment_required() {
        let p = &challenge.payment;
        let mut md = Markdown::new();
        md.heading("Payment required");
        md.paragraph(format!(
            "{action} needs an on-chain payment before it can complete. Fund your \
             wallet with the asset below, then retry the same call."
        ));
        let pairs: Vec<(&str, String)> = [
            ("Asset", p.asset.clone()),
            ("Amount", p.amount.clone()),
            ("Network", p.network.clone()),
            ("Pay to", p.to.clone()),
        ]
        .into_iter()
        .filter_map(|(k, v)| v.filter(|s| !s.is_empty()).map(|v| (k, v)))
        .collect();
        md.kv(pairs);
        append_next_steps(
            &mut md,
            &[Suggestion::new(
                "Check your wallet balance, then retry once funded",
                "tinyplace_status",
                Value::Object(Default::default()),
            )],
        );
        return md.build();
    }

    let reason = super::super::ops::map_err(err);
    let mut md = Markdown::new();
    md.heading("Could not complete");
    md.paragraph(format!("{action} failed."));
    md.kv([("Reason", reason)]);
    md.build()
}

/// As [`sdk_error_text`], wrapped into a (failed) [`ToolResult`].
pub fn sdk_error(action: &str, err: tinyplace::Error) -> ToolResult {
    err_md(sdk_error_text(action, err))
}

/// Convenience: turn an SDK `Result` into `anyhow::Result<Value>`, rendering
/// the error as markdown carried in the `anyhow` message (the FlowTool surfaces
/// it to the LLM verbatim).
///
/// Serialization errors are **propagated**, not hidden: a response that fails to
/// deserialize is a real bug worth surfacing. Callers hitting endpoints that
/// return `null` for an *empty* collection should use [`list_or_empty`] instead,
/// which degrades only that specific case.
pub fn val_or_err<T: serde::Serialize>(
    action: &str,
    result: tinyplace::Result<T>,
) -> anyhow::Result<Value> {
    match result {
        Ok(v) => serde_json::to_value(v)
            .map_err(|e| anyhow::anyhow!("failed to serialize {action} response: {e}")),
        Err(e) => Err(anyhow::anyhow!("{}", sdk_error_text(action, e))),
    }
}

/// Like [`val_or_err`], but for the narrow set of SDK calls whose backend
/// returns `null` for an empty collection (`{"messages": null}`, empty
/// submissions) — which the typed SDK surfaces as a `Serialization` error.
/// Degrades **only** that case to an empty array; every other error, including a
/// genuine shape mismatch, is still surfaced. Mirrors the `*_degrade` handling
/// in the internal controllers, scoped to the endpoints that actually need it.
pub fn list_or_empty<T: serde::Serialize>(
    action: &str,
    result: tinyplace::Result<T>,
) -> anyhow::Result<Value> {
    match result {
        Ok(v) => serde_json::to_value(v)
            .map_err(|e| anyhow::anyhow!("failed to serialize {action} response: {e}")),
        Err(e) if is_empty_state(&e) => Ok(Value::Array(Vec::new())),
        Err(e) => Err(anyhow::anyhow!("{}", sdk_error_text(action, e))),
    }
}

/// Whether an SDK error is really just an empty backend collection that failed
/// to deserialize (e.g. a `null` array). Used only by [`list_or_empty`] and the
/// message flows, which hit endpoints known to return `null` when empty.
pub fn is_empty_state(err: &tinyplace::Error) -> bool {
    matches!(err, tinyplace::Error::Serialization(_))
}

/// Parse an optional `limit`, clamping to `default` when absent, non-numeric, or
/// `<= 0` so a bad value can't reach the SDK/GraphQL layer.
pub fn positive_limit(args: &Value, key: &str, default: i64) -> i64 {
    match opt_i64(args, key) {
        Some(v) if v > 0 => v,
        _ => default,
    }
}

// ── Controller delegation ────────────────────────────────────────────────────

/// Process-global map of tiny.place controller handlers keyed by bare function
/// name (e.g. `registry_register`). Built once from the registered controllers.
fn controller_handlers() -> &'static HashMap<&'static str, ControllerHandler> {
    static MAP: OnceLock<HashMap<&'static str, ControllerHandler>> = OnceLock::new();
    MAP.get_or_init(|| {
        crate::openhuman::tinyplace::all_tinyplace_registered_controllers()
            .into_iter()
            .map(|c| (c.schema.function, c.handler))
            .collect()
    })
}

/// Invoke an internal tiny.place controller by name. Flows use this when the
/// controller does essential work the raw SDK call does not — e.g.
/// `registry_register` performs the x402 payment retry and publishes the
/// directory Agent Card, neither of which `client.registry.register` does.
pub async fn call_controller(function: &str, params: Map<String, Value>) -> Result<Value, String> {
    let param_keys: Vec<&str> = params.keys().map(String::as_str).collect();
    log::debug!("{LOG_PREFIX} controller_call start function={function} param_keys={param_keys:?}");
    let handler = controller_handlers().get(function).ok_or_else(|| {
        log::warn!("{LOG_PREFIX} controller_call unknown function={function}");
        format!("unknown tiny.place controller '{function}'")
    })?;
    let result = handler(params).await;
    match &result {
        Ok(_) => log::debug!("{LOG_PREFIX} controller_call ok function={function}"),
        Err(e) => log::warn!("{LOG_PREFIX} controller_call failed function={function} err={e}"),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn req_str_trims_and_rejects_blank() {
        let args = json!({ "a": "  hi ", "b": "   " });
        assert_eq!(req_str(&args, "a").unwrap(), "hi");
        assert!(req_str(&args, "b").is_err());
        assert!(req_str(&args, "missing").is_err());
    }

    #[test]
    fn opt_i64_accepts_number_or_string() {
        assert_eq!(opt_i64(&json!({ "n": 5 }), "n"), Some(5));
        assert_eq!(opt_i64(&json!({ "n": "7" }), "n"), Some(7));
        assert_eq!(opt_i64(&json!({ "n": "x" }), "n"), None);
        assert_eq!(opt_i64(&json!({}), "n"), None);
    }

    #[test]
    fn opt_bool_accepts_bool_or_string() {
        assert_eq!(opt_bool(&json!({ "b": true }), "b"), Some(true));
        assert_eq!(opt_bool(&json!({ "b": "False" }), "b"), Some(false));
        assert_eq!(opt_bool(&json!({ "b": "nope" }), "b"), None);
    }

    #[test]
    fn opt_str_list_accepts_array_or_csv() {
        assert_eq!(
            opt_str_list(&json!({ "t": ["a", " b ", ""] }), "t"),
            Some(vec!["a".to_string(), "b".to_string()])
        );
        assert_eq!(
            opt_str_list(&json!({ "t": "a, b ,c" }), "t"),
            Some(vec!["a".to_string(), "b".to_string(), "c".to_string()])
        );
        assert_eq!(opt_str_list(&json!({ "t": [] }), "t"), None);
    }

    #[test]
    fn empty_state_degrades_only_serialization_errors() {
        let serde_err = serde_json::from_str::<i32>("not a number").unwrap_err();
        assert!(is_empty_state(&tinyplace::Error::Serialization(serde_err)));
        assert!(!is_empty_state(&tinyplace::Error::InvalidArgument(
            "x".to_string()
        )));
    }

    #[test]
    fn val_or_err_propagates_serialization_errors() {
        // val_or_err no longer hides serialization failures.
        let serde_err = serde_json::from_str::<i32>("not a number").unwrap_err();
        let surfaced = val_or_err::<i32>("Read", Err(tinyplace::Error::Serialization(serde_err)));
        assert!(surfaced.is_err());

        let ok = val_or_err::<i32>("Read", Ok(7)).unwrap();
        assert_eq!(ok, serde_json::json!(7));
    }

    #[test]
    fn list_or_empty_degrades_only_empty_collections() {
        // Null collection (serialization error) → empty array.
        let serde_err = serde_json::from_str::<i32>("not a number").unwrap_err();
        let degraded =
            list_or_empty::<i32>("List", Err(tinyplace::Error::Serialization(serde_err))).unwrap();
        assert_eq!(degraded, Value::Array(Vec::new()));

        // A genuine non-serialization error is still surfaced.
        let surfaced = list_or_empty::<i32>(
            "List",
            Err(tinyplace::Error::InvalidArgument("nope".into())),
        );
        assert!(surfaced.is_err());
    }

    #[test]
    fn positive_limit_clamps_non_positive() {
        assert_eq!(positive_limit(&json!({ "limit": 5 }), "limit", 10), 5);
        assert_eq!(positive_limit(&json!({ "limit": 0 }), "limit", 10), 10);
        assert_eq!(positive_limit(&json!({ "limit": -3 }), "limit", 10), 10);
        assert_eq!(positive_limit(&json!({}), "limit", 10), 10);
    }

    #[test]
    fn ok_md_and_err_md_carry_markdown() {
        let ok = ok_md("**hi**".to_string());
        assert!(!ok.is_error);
        assert_eq!(ok.markdown_formatted.as_deref(), Some("**hi**"));
        let err = err_md("**bad**".to_string());
        assert!(err.is_error);
        assert_eq!(err.markdown_formatted.as_deref(), Some("**bad**"));
    }
}
