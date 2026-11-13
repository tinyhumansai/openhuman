// Matching a `workflow_builder` hand-off to the proposal it is about: id and
// name lookups that ignore nested-workflow references inside a graph the
// hand-off carries.
//
// Textually included into `spawn_subagent_surface.rs`, so it shares that
// module's imports. Split out of `spawn_subagent_workflow.rs` to keep it under
// the layout check's line limit.

/// Byte ranges of `text` that describe nested workflows rather than the
/// proposal being handed off. A hand-off may include the requested graph:
/// every JSON object whose `"type"` is `"sub_workflow"` (the node, its name
/// and config) and every `"workflow_id": "…"` value identifies a child
/// workflow. One string-aware pass over the text; malformed JSON simply
/// yields fewer ranges.
fn nested_workflow_ranges(text: &str) -> Vec<std::ops::Range<usize>> {
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    // Open objects: (start offset, whether it is a sub_workflow node).
    let mut objects: Vec<(usize, bool)> = Vec::new();
    // The last two string literals, to recognise `"key": "value"` pairs.
    let mut last_key: Option<(usize, usize)> = None;
    let mut awaiting_value_of: Option<(usize, usize)> = None;
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'"' => {
                let start = at;
                let mut end = at + 1;
                let mut escaped = false;
                while end < bytes.len() {
                    let byte = bytes[end];
                    if escaped {
                        escaped = false;
                    } else if byte == b'\\' {
                        escaped = true;
                    } else if byte == b'"' {
                        break;
                    }
                    end += 1;
                }
                let literal = start..(end + 1).min(bytes.len());
                if let Some((key_start, key_end)) = awaiting_value_of.take() {
                    let key = &text[key_start..key_end];
                    let value = &text[literal.clone()];
                    if key == "\"workflow_id\"" {
                        ranges.push(literal.clone());
                    } else if key == "\"type\"" && value == "\"sub_workflow\"" {
                        if let Some(open) = objects.last_mut() {
                            open.1 = true;
                        }
                    }
                } else {
                    last_key = Some((literal.start, literal.end));
                }
                at = literal.end;
                continue;
            }
            b':' => awaiting_value_of = last_key.take(),
            b'{' => {
                objects.push((at, false));
                last_key = None;
            }
            b'}' => {
                if let Some((start, true)) = objects.pop() {
                    ranges.push(start..at + 1);
                }
                last_key = None;
                awaiting_value_of = None;
            }
            b',' | b'[' | b']' => {
                last_key = None;
                awaiting_value_of = None;
            }
            _ => {}
        }
        at += 1;
    }
    ranges
}

/// Whether `at` falls inside one of the nested-workflow `ranges`.
fn in_nested_workflow(ranges: &[std::ops::Range<usize>], at: usize) -> bool {
    ranges.iter().any(|range| range.contains(&at))
}

/// Whether `token` occurs in `text` as a delimited id token outside a
/// nested-workflow reference (see [`nested_workflow_ranges`]). The characters
/// on either side must not be id characters (`[A-Za-z0-9_-]`), so a short id
/// (`draft_id` / `flow_id`) never matches a longer one that merely contains it.
fn contains_target_id(text: &str, token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let nested = nested_workflow_ranges(text);
    text.match_indices(token).any(|(at, _)| {
        let is_id_char = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
        let before_ok = at == 0 || !text[..at].chars().last().is_some_and(is_id_char);
        let after_at = at + token.len();
        let after_ok =
            after_at == text.len() || !text[after_at..].chars().next().is_some_and(is_id_char);
        before_ok && after_ok && !in_nested_workflow(&nested, at)
    })
}

/// Whether `name` appears in `text` outside a nested-workflow reference, so a
/// graph node naming a child workflow cannot select that child's proposal.
fn contains_target_name(text: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let nested = nested_workflow_ranges(text);
    text.match_indices(name)
        .any(|(at, _)| !in_nested_workflow(&nested, at))
}
