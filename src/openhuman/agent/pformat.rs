//! P-Format ("Parameter-Format") tool calls — compact, positional,
//! pipe-delimited tool invocations designed to slash the token cost of
//! text-based tool calling.
//!
//! # Why
//!
//! Standard JSON tool calls are heavy on tokens for what's actually a
//! simple instruction:
//!
//! ```text
//! {"name": "get_weather", "arguments": {"location": "London", "unit": "metric"}}
//! ```
//!
//! That's roughly 25 tokens. The same call in P-Format:
//!
//! ```text
//! get_weather[London|metric]
//! ```
//!
//! is ~5 tokens — an 80% reduction. Across a long agent loop with many
//! tool calls per turn, that compounds dramatically.
//!
//! # Spec
//!
//! - One call per `<tool_call>...</tool_call>` tag body.
//! - Form: `name[index|value|index|value|...]` — each argument carries the
//!   slot index it belongs to, so **only the arguments actually being sent
//!   appear**.
//! - `name` is the tool's registered name (alphanumerics + `_`).
//! - Slot indices number the parameters **required first** (in the order the
//!   schema declares them), then the optional ones alphabetically. Both halves
//!   are deterministic across rebuilds and workspaces: a JSON array preserves
//!   order, and `Map` iterates as a `BTreeMap` because this build does not
//!   enable `preserve_order`.
//! - The renderer exposes the numbering in the tool catalogue, each slot marked
//!   as a placeholder to fill:
//!   `get_weather[0|<location>|1|<unit>]`, `math[0|<verbose>|1|<x>|2|<y>]`.
//!   The brackets matter: rendered as bare names the signature reads as a call
//!   to copy, and a live model duly sent the parameter names as the argument
//!   values.
//! - Empty calls: `tool_name[]` for zero-arg tools, and for a call that sends
//!   no arguments at all.
//!
//!   ## Why indices, rather than counting empty slots
//!
//!   The form used to be bare positional — `name[arg1|arg2|...]` — with skipped
//!   arguments written as empty slots (`name[||value]`). That made the *count
//!   of leading delimiters* load-bearing, and it is the single thing models get
//!   wrong most:
//!
//!   - `GMAIL_LIST_THREADS[||50|<query>]` failed schema validation **12 times in
//!     one turn** before the turn was cut short.
//!   - A live `GMAIL_LIST_THREADS` call wrote four leading empties where three
//!     were needed, so `query` and `user_id` each landed one slot late, in
//!     `user_id` and `verbose`. The call ran with the search text as the
//!     account id.
//!
//!   Both are off-by-one on a delimiter, and both bound arguments to the wrong
//!   parameter **silently** — the tool ran, with the wrong values. Indices
//!   remove the counting: a sparse call names its slots, and there is nothing
//!   to miscount. An index that is missing, non-numeric, or out of range is
//!   **rejected** rather than guessed at, so the failure mode moves from a
//!   wrong call that succeeds to a malformed call the model is told about.
//!
//!   Required-first ordering is kept. It is why the natural minimal call — the
//!   one required value — is `name[0|value]` rather than an arbitrary index. An
//!   alphabetical layout put the optional parameters first for most tools, and
//!   a live model wrote `memory_recall[Colorado]` six times in one turn against
//!   `[limit|namespace|query]` and never got a tool to run.
//! - Escapes: `\|` → `|`, `\]` → `]`, `\\` → `\`. Other backslashes
//!   pass through verbatim so URLs and Windows paths remain readable.
//! - Type coercion: schema property `type: integer | number | boolean`
//!   triggers parsing the string into the matching JSON value. Failed
//!   coercion falls back to a string so the model still gets *something*
//!   useful into the tool argument.
//!
//! # Trade-offs
//!
//! - **Positional only** — nested objects or arrays can't be expressed
//!   directly. Tools that need rich payloads should either flatten their
//!   schema, accept a JSON-blob string parameter, or be invoked via the
//!   legacy JSON-in-tag fallback (which the dispatcher attempts when
//!   p-format parsing returns `None`).
//! - **Tool registry required at parse time** — without the schema we
//!   can't reconstruct named arguments. The dispatcher caches a
//!   pre-computed `name → params` map at construction time so this
//!   stays fast and avoids holding a reference to the live tool slice.

use crate::openhuman::tools::Tool;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// JSON-schema primitive type used for argument coercion. Anything we
/// don't recognise (objects, arrays, custom types) is treated as
/// `Other`, which preserves the raw string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PFormatParamType {
    String,
    Integer,
    Number,
    Boolean,
    Other,
}

impl PFormatParamType {
    /// Map a JSON-schema `type` value to the coercion enum. Schemas may
    /// expose `type` as either a single string (`"integer"`) or an
    /// array (`["integer", "null"]`); we accept both and pick the first
    /// non-`null` entry.
    pub fn from_schema_type(value: Option<&Value>) -> Self {
        let label = match value {
            Some(Value::String(s)) => s.as_str(),
            Some(Value::Array(items)) => items
                .iter()
                .find_map(|v| v.as_str().filter(|s| *s != "null"))
                .unwrap_or(""),
            _ => "",
        };
        match label {
            "string" => Self::String,
            "integer" => Self::Integer,
            "number" => Self::Number,
            "boolean" => Self::Boolean,
            _ => Self::Other,
        }
    }
}

/// One tool's positional parameter list, as the dispatcher needs it
/// at parse time.
#[derive(Debug, Clone)]
pub struct PFormatToolParams {
    /// Parameter names in declaration order.
    pub names: Vec<String>,
    /// Parallel slice of JSON types for coercion.
    pub types: Vec<PFormatParamType>,
}

impl PFormatToolParams {
    /// Pull the ordered parameter names + types out of a tool's
    /// JSON schema. Non-object schemas (rare, but possible for
    /// shell-style tools) return an empty list — the renderer falls
    /// back to `name[]`.
    ///
    /// Order is required-first, then optional alphabetically. The renderer
    /// always shows the resulting order in the tool catalogue so the model — and
    /// the parser — agree on the layout; both read it from here, so they cannot
    /// disagree. See the module-level docs for why required comes first.
    pub fn from_schema(schema: &Value) -> Self {
        let Some(props) = schema.get("properties").and_then(|p| p.as_object()) else {
            return Self {
                names: Vec::new(),
                types: Vec::new(),
            };
        };
        // Required parameters first, in the order the schema declares them, then
        // the optional ones alphabetically. Both halves are deterministic (a JSON
        // array preserves order; `Map` is a `BTreeMap` in this build), which is the
        // property the layout actually needs.
        let required: Vec<&str> = schema
            .get("required")
            .and_then(|r| r.as_array())
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();

        let mut ordered: Vec<&String> = Vec::with_capacity(props.len());
        for name in &required {
            if let Some((key, _)) = props.get_key_value(*name) {
                if !ordered.contains(&key) {
                    ordered.push(key);
                }
            }
        }
        for key in props.keys() {
            if !ordered.contains(&key) {
                ordered.push(key);
            }
        }

        let mut names = Vec::with_capacity(ordered.len());
        let mut types = Vec::with_capacity(ordered.len());
        for key in ordered {
            names.push(key.clone());
            types.push(PFormatParamType::from_schema_type(props[key].get("type")));
        }
        Self { names, types }
    }
}

/// Pre-computed lookup of every tool's parameter list. Built once at
/// dispatcher construction time so the parser doesn't need to hold a
/// reference to the live `Vec<Box<dyn Tool>>` (which the agent owns).
///
/// The map preserves the spec contract: the parser refuses to invent
/// argument names for an unknown tool, so an LLM can't tunnel
/// arbitrary JSON in by guessing tool names that don't exist.
pub type PFormatRegistry = HashMap<String, PFormatToolParams>;

/// Build a [`PFormatRegistry`] from the agent's tool slice. Call this
/// once at construction time, before the tools are moved into the
/// agent — the result is owned and self-contained, so it survives the
/// move without keeping a reference back to the registry.
pub fn build_registry(tools: &[Box<dyn Tool>]) -> PFormatRegistry {
    tools
        .iter()
        .map(|t| {
            (
                t.name().to_string(),
                PFormatToolParams::from_schema(&t.parameters_schema()),
            )
        })
        .collect()
}

/// Render a single tool's p-format signature, e.g. `get_weather[<location>|<unit>]`.
///
/// This signature is included in the tool catalogue within the system prompt
/// to tell the LLM exactly how to order positional arguments for a tool.
pub fn render_signature(name: &str, params: &PFormatToolParams) -> String {
    if params.names.is_empty() {
        format!("{name}[]")
    } else {
        // Each slot is wrapped in angle brackets so it reads as a placeholder to
        // fill, not as a call to copy. Bare names do get copied: live on flo the
        // model answered `memory_recall[limit|namespace|query]` — the signature
        // verbatim, the parameter names sent as the argument *values*. Backticking
        // the whole signature does not help either; that made it copy the backticks
        // (`` C `memory_recall[…]` ``) instead. `<…>` marks the slot without
        // decorating the form.
        let slots: Vec<String> = params
            .names
            .iter()
            .enumerate()
            .map(|(i, n)| format!("{i}|<{n}>"))
            .collect();
        format!("{name}[{}]", slots.join("|"))
    }
}

/// Convenience wrapper that renders a signature directly from a `Tool` implementation.
pub fn render_signature_from_tool(tool: &dyn Tool) -> String {
    let params = PFormatToolParams::from_schema(&tool.parameters_schema());
    render_signature(tool.name(), &params)
}

/// Parse a single p-format call body and reconstruct named JSON arguments.
///
/// This function:
/// 1. Locates the positional arguments within the `[...]` brackets.
/// 2. Splits them by the `|` delimiter (respecting escapes).
/// 3. Maps each positional value to its parameter name from the tool registry.
/// 4. Performs type coercion (e.g., string to integer) based on the tool's schema.
///
/// Returns `(tool_name, args_json)` on success, or `None` if the format is invalid
/// or the tool is unknown.
pub fn parse_call(body: &str, registry: &PFormatRegistry) -> Option<(String, Value)> {
    let trimmed = body.trim();

    // Locate the opening bracket. The closing bracket must be the
    // **last** character of the trimmed body — anything trailing it
    // (e.g. extra whitespace, JSON, prose) means this isn't a valid
    // p-format call and we leave it for the JSON fallback.
    let open = trimmed.find('[')?;
    if !trimmed.ends_with(']') {
        return None;
    }

    let name = trimmed[..open].trim();
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }

    let inner = &trimmed[open + 1..trimmed.len() - 1];

    // Look up the parameter spec — required so we can map positional
    // values back to named JSON keys with the correct types.
    let params = registry.get(name)?;

    let tokens = split_pipes(inner);
    // Index/value pairs, so an odd token count means the model dropped or added
    // a delimiter. Reject rather than guess: the whole point of the indices is
    // that a miscounted delimiter can no longer bind a value to the wrong
    // parameter, and silently keeping the pairs that happen to line up would
    // put that failure right back.
    if tokens.len() % 2 != 0 {
        tracing::debug!(
            tool = name,
            tokens = tokens.len(),
            "[pformat] odd token count — not index/value pairs, refusing to parse"
        );
        return None;
    }

    let mut args = Map::with_capacity(tokens.len() / 2);
    for pair in tokens.chunks_exact(2) {
        let (raw_index, raw) = (pair[0].trim(), &pair[1]);
        let Ok(slot) = raw_index.parse::<usize>() else {
            // A non-numeric index is a call in the old bare-positional form (or
            // simply malformed). Refusing is deliberate: parsing it positionally
            // would silently resurrect the off-by-one this format exists to end.
            tracing::debug!(
                tool = name,
                index = %raw_index,
                "[pformat] slot index is not a number — refusing to parse"
            );
            return None;
        };
        let Some(param_name) = params.names.get(slot) else {
            tracing::debug!(
                tool = name,
                slot,
                slots = params.names.len(),
                "[pformat] slot index out of range — refusing to parse"
            );
            return None;
        };
        // An empty value is an argument the model did not send, so the key is
        // left out entirely rather than set to `""`. Inserting `""` makes every
        // non-string parameter fail schema validation — a typed `max_results`
        // arriving as `""` means the tool never runs, and the error names a
        // field the model deliberately left blank, which it cannot satisfy.
        if raw.trim().is_empty() {
            tracing::debug!(
                tool = name,
                slot,
                param = %param_name,
                "[pformat] empty value for a named slot — argument omitted"
            );
            continue;
        }
        let coerced = coerce_value(
            raw,
            params
                .types
                .get(slot)
                .copied()
                .unwrap_or(PFormatParamType::String),
        );
        // Last write wins on a repeated slot. Rare enough not to be worth
        // rejecting the whole call over, and the later value is the model's
        // latest intent.
        args.insert(param_name.clone(), coerced);
    }

    Some((name.to_string(), Value::Object(args)))
}

/// Split a p-format argument body on unescaped `|`. Honours `\|`,
/// `\]`, and `\\` escapes. An empty body produces an empty `Vec` (NOT
/// `vec![""]`) so a tool with zero parameters parses cleanly.
fn split_pipes(input: &str) -> Vec<String> {
    if input.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.peek() {
                Some('|') => {
                    current.push('|');
                    chars.next();
                }
                Some(']') => {
                    current.push(']');
                    chars.next();
                }
                Some('\\') => {
                    current.push('\\');
                    chars.next();
                }
                _ => current.push('\\'),
            }
        } else if c == '|' {
            out.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }

    out.push(current);
    out
}

/// Coerce a raw string argument into the JSON type the schema expects.
/// Falls back to `Value::String` for any failed coercion so the model
/// still gets a usable value into the tool argument map.
fn coerce_value(raw: &str, ty: PFormatParamType) -> Value {
    match ty {
        PFormatParamType::Integer => raw
            .trim()
            .parse::<i64>()
            .map(|n| Value::Number(n.into()))
            .unwrap_or_else(|_| Value::String(raw.to_string())),
        PFormatParamType::Number => raw
            .trim()
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(raw.to_string())),
        PFormatParamType::Boolean => match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" => Value::Bool(true),
            "false" | "no" | "0" => Value::Bool(false),
            _ => Value::String(raw.to_string()),
        },
        PFormatParamType::String | PFormatParamType::Other => Value::String(raw.to_string()),
    }
}

// ──────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_registry() -> PFormatRegistry {
        let mut reg = PFormatRegistry::new();
        reg.insert(
            "get_weather".to_string(),
            PFormatToolParams::from_schema(&json!({
                "type": "object",
                "properties": {
                    "location": { "type": "string" },
                    "unit": { "type": "string" }
                }
            })),
        );
        reg.insert(
            "shell".to_string(),
            PFormatToolParams::from_schema(&json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                }
            })),
        );
        reg.insert(
            "ping".to_string(),
            PFormatToolParams::from_schema(&json!({
                "type": "object",
                "properties": {}
            })),
        );
        reg.insert(
            "math".to_string(),
            PFormatToolParams::from_schema(&json!({
                "type": "object",
                "properties": {
                    "x": { "type": "integer" },
                    "y": { "type": "number" },
                    "verbose": { "type": "boolean" }
                }
            })),
        );
        reg
    }

    #[test]
    fn renders_zero_arg_signature() {
        let reg = make_registry();
        assert_eq!(render_signature("ping", &reg["ping"]), "ping[]");
    }

    #[test]
    fn renders_multi_arg_signature() {
        let reg = make_registry();
        assert_eq!(
            render_signature("get_weather", &reg["get_weather"]),
            "get_weather[0|<location>|1|<unit>]"
        );
    }

    /// Required parameters take the leading slots, so the shortest useful call —
    /// the required values and nothing else — parses correctly. Under plain
    /// alphabetical order it did not: `memory_recall` advertises
    /// `required: ["query"]` with optional `limit`/`namespace`, so alphabetical put
    /// `limit` first and a live model's `memory_recall[Colorado]` set `limit` to a
    /// string, failing schema validation on all six of its attempts in one turn.
    #[test]
    fn required_parameters_take_the_leading_slots() {
        let params = PFormatToolParams::from_schema(&json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer" },
                "namespace": { "type": "string" },
                "query": { "type": "string" }
            },
            "required": ["query"]
        }));
        assert_eq!(params.names, vec!["query", "limit", "namespace"]);
        // Types stay aligned with the reordered names, or coercion would apply the
        // wrong rule to each slot.
        assert_eq!(
            params.types,
            vec![
                PFormatParamType::String,
                PFormatParamType::Integer,
                PFormatParamType::String
            ]
        );
        assert_eq!(
            render_signature("memory_recall", &params),
            "memory_recall[0|<query>|1|<limit>|2|<namespace>]"
        );

        // The minimal call is `[0|value]` — required-first is what makes the one
        // value the model has to send slot 0, rather than an arbitrary number it
        // has to look up.
        let mut reg = PFormatRegistry::new();
        reg.insert("memory_recall".to_string(), params);
        let (name, args) = parse_call("memory_recall[0|Colorado]", &reg).unwrap();
        assert_eq!(name, "memory_recall");
        assert_eq!(args, json!({"query": "Colorado"}));

        // Several required parameters keep the schema's declared order, not
        // alphabetical, so the layout matches how the tool documents itself.
        let multi = PFormatToolParams::from_schema(&json!({
            "type": "object",
            "properties": {
                "alpha": { "type": "string" },
                "rule": { "type": "string" },
                "tool_name": { "type": "string" }
            },
            "required": ["tool_name", "rule"]
        }));
        assert_eq!(multi.names, vec!["tool_name", "rule", "alpha"]);
    }

    /// Slots are rendered as `<name>` placeholders, never bare names. A bare
    /// signature reads as a call to copy: live on flo the model replied
    /// `memory_recall[limit|namespace|query]`, sending the parameter names as the
    /// argument values, and every one failed schema validation.
    #[test]
    fn signature_slots_are_marked_as_placeholders() {
        let reg = make_registry();
        let sig = render_signature("get_weather", &reg["get_weather"]);
        for name in &reg["get_weather"].names {
            assert!(
                sig.contains(&format!("<{name}>")),
                "slot {name} must be a placeholder in {sig}"
            );
            assert!(
                !sig.contains(&format!("[{name}|")) && !sig.contains(&format!("|{name}]")),
                "slot {name} must not appear bare in {sig}"
            );
        }
        // A zero-arg tool has no slots to mark.
        assert_eq!(render_signature("ping", &reg["ping"]), "ping[]");
    }

    #[test]
    fn parses_simple_call() {
        let reg = make_registry();
        let (name, args) = parse_call("get_weather[0|London|1|metric]", &reg).unwrap();
        assert_eq!(name, "get_weather");
        assert_eq!(args, json!({"location": "London", "unit": "metric"}));
    }

    #[test]
    fn parses_zero_arg_call() {
        let reg = make_registry();
        let (name, args) = parse_call("ping[]", &reg).unwrap();
        assert_eq!(name, "ping");
        assert_eq!(args, json!({}));
    }

    #[test]
    fn parses_single_arg_with_spaces() {
        let reg = make_registry();
        let (name, args) = parse_call("shell[0|ls -la /tmp]", &reg).unwrap();
        assert_eq!(name, "shell");
        assert_eq!(args, json!({"command": "ls -la /tmp"}));
    }

    #[test]
    fn handles_pipe_escape() {
        let reg = make_registry();
        let (_, args) = parse_call(r"shell[0|cat foo \| grep bar]", &reg).unwrap();
        assert_eq!(args, json!({"command": "cat foo | grep bar"}));
    }

    #[test]
    fn handles_bracket_escape() {
        let reg = make_registry();
        let (_, args) = parse_call(r"shell[0|echo \]done\]]", &reg).unwrap();
        assert_eq!(args, json!({"command": "echo ]done]"}));
    }

    #[test]
    fn handles_backslash_escape() {
        let reg = make_registry();
        let (_, args) = parse_call(r"shell[0|C:\\Users\\bob]", &reg).unwrap();
        assert_eq!(args, json!({"command": r"C:\Users\bob"}));
    }

    #[test]
    fn coerces_typed_arguments() {
        let reg = make_registry();
        // Alphabetical order: verbose, x, y. The signature the model sees in
        // the catalogue is `math[0|<verbose>|1|<x>|2|<y>]`, so this is the call
        // it would write.
        let (_, args) = parse_call("math[0|true|1|42|2|3.14]", &reg).unwrap();
        assert_eq!(args, json!({"verbose": true, "x": 42, "y": 3.14}));
    }

    #[test]
    fn coercion_falls_back_to_string_on_failure() {
        let reg = make_registry();
        let (_, args) = parse_call("math[0|maybe|1|notanumber|2|alsonotanumber]", &reg).unwrap();
        assert_eq!(
            args,
            json!({
                "verbose": "maybe",
                "x": "notanumber",
                "y": "alsonotanumber"
            })
        );
    }

    #[test]
    fn signature_uses_alphabetical_order() {
        let reg = make_registry();
        // `math` has properties (in source) {x, y, verbose} but
        // BTreeMap iteration sorts to {verbose, x, y}.
        assert_eq!(
            render_signature("math", &reg["math"]),
            "math[0|<verbose>|1|<x>|2|<y>]"
        );
    }

    #[test]
    fn rejects_unknown_tool() {
        let reg = make_registry();
        assert!(parse_call("nope[0|arg]", &reg).is_none());
    }

    #[test]
    fn rejects_missing_brackets() {
        let reg = make_registry();
        assert!(parse_call("get_weather London metric", &reg).is_none());
    }

    #[test]
    fn rejects_trailing_garbage() {
        let reg = make_registry();
        // Closing bracket isn't last char → invalid p-format, dispatcher
        // should try the JSON fallback path.
        assert!(parse_call("get_weather[London|metric] // comment", &reg).is_none());
    }

    /// A slot number the schema has no parameter for is refused outright.
    /// Silently dropping it is what the bare-positional form did with an excess
    /// value, and dropping is only safe when the remaining values are still in
    /// the right slots — which is exactly the assumption indices exist to stop
    /// relying on.
    #[test]
    fn an_out_of_range_slot_is_refused() {
        let reg = make_registry();
        assert!(parse_call("get_weather[0|London|1|metric|2|extra]", &reg).is_none());
    }

    /// An odd token count means a delimiter was dropped or added, so the pairs
    /// no longer say what the model meant. Refuse rather than keep the prefix
    /// that happens to line up.
    #[test]
    fn an_odd_token_count_is_refused() {
        let reg = make_registry();
        assert!(parse_call("get_weather[0|London|1]", &reg).is_none());
        assert!(parse_call("get_weather[London]", &reg).is_none());
    }

    /// The old bare-positional form must not parse. This is the guarantee that
    /// replaces the old failure mode: a leftover positional call is rejected and
    /// reported, instead of binding its values to whichever slots they land in.
    #[test]
    fn a_bare_positional_call_is_refused_not_reinterpreted() {
        let reg = make_registry();
        // Two values, no indices — the pre-index form.
        assert!(parse_call("get_weather[London|metric]", &reg).is_none());
        // And the shape that actually misfired live: leading empties standing in
        // for skipped arguments.
        assert!(parse_call("get_weather[||metric]", &reg).is_none());
    }

    /// A named slot with nothing after it is an argument the model chose not to
    /// send, so it does not appear in the object at all. Sending `""` instead is
    /// what made a skipped integer slot fail validation with an error the model
    /// could not act on.
    #[test]
    fn an_empty_value_is_an_omitted_argument() {
        let reg = make_registry();
        let (_, args) = parse_call("get_weather[0||1|]", &reg).unwrap();
        assert_eq!(args, json!({}));

        // Whitespace is nothing written, too.
        let (_, args) = parse_call("get_weather[0|  |1|metric]", &reg).unwrap();
        assert_eq!(args, json!({"unit": "metric"}));
    }

    /// Sending only the arguments you mean to send is the whole point: no
    /// leading empties, so no delimiters to miscount.
    #[test]
    fn a_sparse_call_names_only_the_slots_it_fills() {
        let reg = make_registry();
        let (_, args) = parse_call("get_weather[1|metric]", &reg).unwrap();
        assert_eq!(args, json!({"unit": "metric"}));
    }

    /// The shape that hard-looped a live turn twelve times, and the one that
    /// later ran with the search text as the account id. Written with indices,
    /// both are unambiguous.
    #[test]
    fn the_live_gmail_call_binds_correctly_with_indices() {
        let mut reg = PFormatRegistry::new();
        reg.insert(
            "list_threads".to_string(),
            PFormatToolParams::from_schema(&json!({
                "type": "object",
                "properties": {
                    "connection_id": { "type": "string" },
                    "max_results": { "type": "integer" },
                    "query": { "type": "string" },
                    "user_id": { "type": "string" },
                },
            })),
        );

        let (_, args) = parse_call("list_threads[2|Colorado|3|me]", &reg).unwrap();
        assert_eq!(args, json!({"query": "Colorado", "user_id": "me"}));
        assert!(
            args.get("max_results").is_none(),
            "a slot that was never named must be absent, not an empty string: {args}"
        );

        // One extra leading delimiter used to shift every value one slot late.
        // It cannot now: the slot is named, so an accidental delimiter makes the
        // count odd and the call is refused instead of misbound.
        assert!(parse_call("list_threads[|2|Colorado|3|me]", &reg).is_none());
    }

    /// A repeated slot takes the later value — the model's latest intent — and
    /// does not fail the call.
    #[test]
    fn a_repeated_slot_takes_the_last_value() {
        let reg = make_registry();
        let (_, args) = parse_call("get_weather[0|London|0|Berlin]", &reg).unwrap();
        assert_eq!(args, json!({"location": "Berlin"}));
    }

    #[test]
    fn signature_round_trips_with_parser() {
        let reg = make_registry();
        let sig = render_signature("get_weather", &reg["get_weather"]);
        assert_eq!(sig, "get_weather[0|<location>|1|<unit>]");
        // The numbering the signature shows is the numbering the parser reads.
        let (name, args) = parse_call("get_weather[0|Berlin|1|imperial]", &reg).unwrap();
        assert_eq!(name, "get_weather");
        assert_eq!(args["location"], json!("Berlin"));
        assert_eq!(args["unit"], json!("imperial"));
    }
}
