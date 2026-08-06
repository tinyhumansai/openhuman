//! Deterministic stand-ins for a dry run.
//!
//! A dry run exercises a graph's wiring without calling a model or a provider.
//! These fill that gap: rather than echoing input back — which would let a
//! downstream binding appear to resolve when it never could — they read the
//! node's declared output schema and synthesise a value of the right shape, so
//! a mis-wired binding fails the dry run the same way it would fail a real one.

#![allow(unused_imports)]

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyflows::caps::*;
use tinyflows::error::{EngineError, Result};

use super::*;
use crate::openhuman::config::Config;

/// A **dry-run-only** [`AgentRunner`] mock that, unlike the vendored crate's
/// `tinyflows::caps::mock::MockAgentRunner`, respects an `agent` node's
/// `config.output_parser.schema` when synthesizing its echo response.
///
/// `DryRunWorkflowTool` (`flows::builder_tools`) wires this in place of the
/// vendored `MockAgentRunner` so its null-resolution check (every `tool_call`
/// arg that resolves to `null`) doesn't **false-positive** on a CORRECTLY-built
/// agent node. Without it: the vendored `MockAgentRunner` always echoes
/// `{ agent, request, connection }` regardless of schema, and the vendored
/// `agent` node's output-parser sub-port (`tinyflows::nodes::integration::schema`)
/// then fails that shape against ANY declared schema (no field matches) and
/// falls to a one-shot LLM auto-fix that the sandbox's plain `MockLlm` also
/// can't satisfy — so the whole dry run would error out even for a workflow a
/// real run (via [`OpenHumanAgentRunner`], whose completion the same sub-port
/// validates/repairs against the schema) would execute cleanly.
///
/// When `request` (the resolved node config `run_agent` receives — see
/// [`AgentRunner::run_agent`]) carries a non-null `output_parser.schema`
/// describing an object with `properties`, returns an object with every
/// declared property present, populated with a type-appropriate placeholder
/// (`string` → `""`, `number`/`integer` → `0`, `boolean` → `false`, `object` →
/// `{}`, `array` → `[]`, anything else → `null`; a property with a non-empty
/// `enum` gets its FIRST allowed value instead — see [`placeholder_for_type`])
/// — enough to satisfy the vendored validator's `type`/`required`/`enum`
/// checks (see `tinyflows::nodes::integration::schema::validate`) without a
/// real model call. With no schema, mirrors the vendored `MockAgentRunner`'s
/// default echo shape so dry-run behavior for schema-less agents is unchanged.
#[derive(Debug, Default, Clone)]
pub struct SchemaAwareMockAgentRunner;

#[async_trait]
impl AgentRunner for SchemaAwareMockAgentRunner {
    async fn run_agent(
        &self,
        agent_ref: &str,
        request: Value,
        conn: Option<&str>,
    ) -> Result<Value> {
        let schema = request
            .get("output_parser")
            .and_then(|parser| parser.get("schema"))
            .filter(|schema| !schema.is_null());
        match schema {
            Some(schema) => {
                let placeholder = placeholder_for_schema(schema);
                tracing::debug!(
                    target: "flows",
                    agent_ref,
                    "[flows] dry_run: schema-aware mock agent synthesized a placeholder \
                     matching output_parser.schema"
                );
                Ok(placeholder)
            }
            None => {
                tracing::debug!(
                    target: "flows",
                    agent_ref,
                    "[flows] dry_run: schema-aware mock agent has no output_parser.schema — \
                     mirroring the vendored MockAgentRunner echo shape"
                );
                Ok(json!({ "agent": agent_ref, "request": request, "connection": conn }))
            }
        }
    }
}

/// A **dry-run-only** [`LlmProvider`] mock that, unlike the vendored crate's
/// `tinyflows::caps::mock::MockLlm`, respects an `agent` node's
/// `config.output_parser.schema` when synthesizing its completion.
///
/// This closes the OTHER half of the same gap [`SchemaAwareMockAgentRunner`]
/// closes. The vendored `agent` node only routes to an [`AgentRunner`] when the
/// node carries a **non-empty `agent_ref`** AND the host wired an agent registry
/// (`vendor/tinyflows/src/nodes/integration/agent.rs`, `run_turn`:
/// `(Some(agent_ref), Some(runner)) => runner.run_agent(...)`); **every other
/// case** — and builder-generated agent nodes carry NO `agent_ref` — falls back
/// to `ctx.caps.llm.complete(cfg.clone(), conn)`. So in the sandbox those plain
/// agent nodes never reach `SchemaAwareMockAgentRunner` at all: they hit the
/// `llm` slot, which (with the vendored `MockLlm`) echoes
/// `{ "completion": <config>, "connection": <conn> }`. The agent node's
/// output-parser sub-port then validates that echo against the declared schema
/// (`schema::parse_and_validate` — it validates the WHOLE completion value, not
/// a `.text` field), no field matches, and it falls to a one-shot LLM auto-fix
/// that the same `MockLlm` also can't satisfy — so the dry run errors with
/// `output_parser: value failed schema validation after auto-fix: missing
/// required property ...` even for a workflow a real run would execute cleanly.
/// This false-failure burned many dry-run cycles for correctly-built graphs.
///
/// When `request` (the node config the node hands to `complete` — see the
/// `_ => ctx.caps.llm.complete(cfg.clone(), conn)` arm above) carries a non-null
/// `output_parser.schema`, this returns [`placeholder_for_schema`] DIRECTLY.
/// The sub-port receives that already-schema-valid object as its `value`
/// (`validate` returns no errors), so it returns `Ok` WITHOUT ever invoking the
/// auto-fix LLM path — exactly the shape the vendored validator's
/// `type`/`required`/`enum` checks accept, with no real model call. With no
/// schema, it mirrors the vendored `MockLlm` echo shape byte-for-byte
/// (`{ "completion": request, "connection": conn }`) so schema-less agent
/// dry-run behavior — and downstream `=nodes.<agent>.item.json.completion...`
/// bindings — stay identical to today.
#[derive(Debug, Default, Clone)]
pub struct SchemaAwareMockLlm;

#[async_trait]
impl LlmProvider for SchemaAwareMockLlm {
    async fn complete(&self, request: Value, conn: Option<&str>) -> Result<Value> {
        let schema = request
            .get("output_parser")
            .and_then(|parser| parser.get("schema"))
            .filter(|schema| !schema.is_null());
        match schema {
            Some(schema) => {
                let placeholder = placeholder_for_schema(schema);
                tracing::debug!(
                    target: "flows",
                    "[flows] dry_run: schema-aware mock LLM synthesized a placeholder \
                     matching output_parser.schema (plain agent node, no agent_ref)"
                );
                Ok(placeholder)
            }
            None => {
                tracing::debug!(
                    target: "flows",
                    "[flows] dry_run: schema-aware mock LLM has no output_parser.schema — \
                     mirroring the vendored MockLlm echo shape"
                );
                Ok(json!({ "completion": request, "connection": conn }))
            }
        }
    }
}

/// Builds a placeholder JSON value satisfying `schema`'s `properties`/`type`
/// constraints, for [`SchemaAwareMockAgentRunner`]. Only the shallow, top-level
/// `properties` map is populated — enough for the minimal validator in
/// `tinyflows::nodes::integration::schema` (`type`, `required`, `properties`);
/// deeply-nested `required` constraints on a nested `object`/`array` property
/// are a documented limitation (the placeholder for those is an empty `{}`/`[]`).
pub(crate) fn placeholder_for_schema(schema: &Value) -> Value {
    match schema.get("properties").and_then(Value::as_object) {
        Some(props) => {
            let placeholders = props
                .iter()
                .map(|(key, subschema)| (key.clone(), placeholder_for_type(subschema)));
            Value::Object(placeholders.collect())
        }
        // No `properties` to enumerate (e.g. a bare `{"type": "array"}`
        // schema) — fall back to a type-only placeholder for the schema itself.
        None => placeholder_for_type(schema),
    }
}

/// The placeholder value for one property's subschema, keyed by its
/// declared JSON-Schema `type` (see [`placeholder_for_schema`]).
///
/// An `enum` constraint is honored FIRST, before falling back to the
/// type-only placeholder: the vendored validator
/// (`tinyflows::nodes::integration::schema::validate`) rejects any value not
/// listed in a schema's `enum`, and a generic type placeholder (e.g. `""` for
/// `{"type": "string", "enum": ["urgent", "normal"]}`) is essentially never
/// one of the allowed values — that would fail the dry run even though a real
/// agent, prompted with the schema, could easily satisfy it. The schema
/// author's own first listed value is always allowed by construction, so it's
/// returned as-is (whatever its JSON type).
pub(crate) fn placeholder_for_type(subschema: &Value) -> Value {
    if let Some(first_allowed) = subschema
        .get("enum")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
    {
        return first_allowed.clone();
    }
    match subschema.get("type").and_then(Value::as_str) {
        Some("string") => json!(""),
        Some("number" | "integer") => json!(0),
        Some("boolean") => json!(false),
        Some("object") => json!({}),
        Some("array") => json!([]),
        _ => Value::Null,
    }
}
