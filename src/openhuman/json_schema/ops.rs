//! Business logic for [`super`] — see that module's docs for why this domain
//! is neutral (owned by neither the adapter seam nor the integration
//! catalog) and for the vendor-name ban that applies to this file too.

use serde_json::Value;

/// Dotted path to the shallowest array-typed property declared by `schema`.
///
/// Breadth-first, so the shallowest match wins: every property at one level is
/// checked for `type: "array"` before descending, which matters when a schema
/// declares both a top-level list and a deeper one and a binding should prefer
/// the outer.
///
/// `None` when `schema` is absent, declares no `properties`, or names no array
/// anywhere.
pub(crate) fn compute_primary_array_path(schema: Option<&Value>) -> Option<String> {
    let root = schema?;
    let mut queue: std::collections::VecDeque<(String, &Value)> = std::collections::VecDeque::new();
    queue.push_back((String::new(), root));

    while let Some((path, node)) = queue.pop_front() {
        let Some(props) = node.get("properties").and_then(Value::as_object) else {
            continue;
        };
        // Check every property at THIS level for an array before descending
        // to the next level — guarantees the shallowest match wins.
        for (key, prop_schema) in props {
            if prop_schema.get("type").and_then(Value::as_str) == Some("array") {
                let prop_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                return Some(prop_path);
            }
        }
        for (key, prop_schema) in props {
            let prop_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            queue.push_back((prop_path, prop_schema));
        }
    }
    None
}

/// [`compute_primary_array_path`]'s counterpart for a REAL runtime value rather
/// than a schema — walks the value breadth-first for the first array-typed
/// property.
///
/// `skip_root_keys` names properties to ignore **at the root only**. A caller
/// whose values arrive wrapped in a response envelope passes the envelope's own
/// metadata keys here, so an envelope field can never masquerade as (or shadow,
/// via the shallowest-wins tie) the tool's real array. Pass `&[]` when the value
/// has no envelope.
///
/// Because the scan starts at whatever root it is given, a hit beneath a wrapper
/// property comes back already prefixed (e.g. `"data.issues"`) with no separate
/// stitching step needed.
///
/// `None` when no array is found anywhere in the value — note a genuinely empty
/// list still serializes as `[]`, which *is* an array, so `None` means the shape
/// truly has no list at any depth.
pub(crate) fn compute_primary_array_path_from_value(
    value: &Value,
    skip_root_keys: &[&str],
) -> Option<String> {
    let mut queue: std::collections::VecDeque<(String, &Value)> = std::collections::VecDeque::new();
    queue.push_back((String::new(), value));

    while let Some((path, node)) = queue.pop_front() {
        let Some(obj) = node.as_object() else {
            continue;
        };
        for (key, v) in obj {
            if path.is_empty() && skip_root_keys.contains(&key.as_str()) {
                continue;
            }
            if v.is_array() {
                let prop_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                return Some(prop_path);
            }
        }
        for (key, v) in obj {
            if path.is_empty() && skip_root_keys.contains(&key.as_str()) {
                continue;
            }
            let prop_path = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            queue.push_back((prop_path, v));
        }
    }
    None
}

/// Extracts top-level field names from an output-schema value.
///
/// Reads `.properties`' keys for a standard object schema
/// (`{"type": "object", "properties": {…}}`), and falls back to the schema's own
/// top-level keys minus common JSON-Schema keywords for a looser or legacy
/// shape. Empty when the schema is absent or unrecognized — never fails.
pub(crate) fn response_fields_from_schema(schema: Option<&Value>) -> Vec<String> {
    const SCHEMA_KEYWORDS: &[&str] = &[
        "type",
        "required",
        "additionalProperties",
        "$schema",
        "description",
        "title",
        "examples",
    ];

    let Some(obj) = schema.and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut fields: Vec<String> =
        if let Some(props) = obj.get("properties").and_then(Value::as_object) {
            props.keys().cloned().collect()
        } else {
            obj.keys()
                .filter(|k| !SCHEMA_KEYWORDS.contains(&k.as_str()))
                .cloned()
                .collect()
        };
    fields.sort();
    fields
}

/// Returns the names in `required` that are absent or `null` in `args`.
pub(crate) fn missing_required_args(required: &[String], args: &Value) -> Vec<String> {
    required
        .iter()
        .filter(|name| match args.get(name.as_str()) {
            None => true,
            Some(v) => v.is_null(),
        })
        .cloned()
        .collect()
}

/// [B13] Returns argument names in `args` that are NOT declared `properties`
/// of `schema` — the NAME-VALIDITY counterpart to [`missing_required_args`]'s
/// PRESENCE check. Catches the class of bug [`missing_required_args`] alone
/// cannot: a builder wires a real, well-typed value under an arg name the
/// action's schema doesn't recognize at all (e.g. a chat action's `text` when
/// the live action actually wants `markdown_text`) — the required arg still
/// LOOKS satisfied from [`missing_required_args`]' perspective (a value is
/// present under *some* key), so the mistake sails through authoring/save and
/// only 400s from the real provider at runtime.
///
/// `None` means "cannot validate this schema — skip, never reject", so a
/// caller must never turn a `None` into a rejection:
/// - `schema` is `None` (the input schema is unknown for this slug), or
/// - `schema` is not a JSON object, or names no object `properties` map (an
///   unrecognized/legacy shape — nothing to check names against), or
/// - `schema` declares `additionalProperties: true` (the provider explicitly
///   telling us to accept arbitrary keys beyond the declared ones).
///
/// `Some(vec![])` means the schema WAS usable and every arg name in `args`
/// is a real declared property. `args` must be a JSON object to check
/// against; any other shape (including `Value::Null` — no args wired at
/// all) yields `Some(vec![])`, mirroring [`missing_required_args`]' treatment
/// of an absent/non-object `args`.
pub(crate) fn unsupported_arg_names(schema: Option<&Value>, args: &Value) -> Option<Vec<String>> {
    let schema_obj = schema?.as_object()?;
    if schema_obj
        .get("additionalProperties")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return None;
    }
    let properties = schema_obj.get("properties")?.as_object()?;
    let Some(args_obj) = args.as_object() else {
        return Some(Vec::new());
    };
    let mut unsupported: Vec<String> = args_obj
        .keys()
        .filter(|k| !properties.contains_key(k.as_str()))
        .cloned()
        .collect();
    unsupported.sort();
    Some(unsupported)
}
