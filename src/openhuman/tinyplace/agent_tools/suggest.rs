//! "Next steps" suggestion chaining for tiny.place agent flows.
//!
//! Mirrors the tiny.place CLI's `suggestions` array (see `flows.ts`): after a
//! flow runs, it returns a short list of ready-to-run follow-up tool calls with
//! IDs already filled in, so the agent can decide what to do next without
//! re-deriving arguments. Unlike the CLI — which suggests `tinyplace …` shell
//! commands — these suggestions reference the **agent tool surface** (the
//! `tinyplace_*` tool names) and carry a concrete JSON argument object.

use serde_json::Value;

use super::render::Markdown;

/// One ready-to-run follow-up the agent can take after a flow.
#[derive(Clone, Debug)]
pub struct Suggestion {
    /// Human-readable rationale ("Watch submissions arrive").
    pub description: String,
    /// The tool to call next (e.g. `tinyplace_submissions`).
    pub tool: String,
    /// Concrete arguments for that call, IDs pre-filled.
    pub args: Value,
}

impl Suggestion {
    pub fn new(description: impl Into<String>, tool: impl Into<String>, args: Value) -> Self {
        Self {
            description: description.into(),
            tool: tool.into(),
            args,
        }
    }

    /// Render a single suggestion as a markdown bullet:
    /// `- **<desc>** — call `tool` with `{json}``
    fn to_bullet(&self) -> String {
        let args = compact_args(&self.args);
        if args == "{}" {
            format!("**{}** — call `{}`", self.description.trim(), self.tool)
        } else {
            format!(
                "**{}** — call `{}` with `{}`",
                self.description.trim(),
                self.tool,
                args
            )
        }
    }
}

/// Compact single-line JSON for an args object (sorted? — serde preserves
/// insertion order for `Map`, which is what callers build, so leave as-is).
fn compact_args(args: &Value) -> String {
    serde_json::to_string(args).unwrap_or_else(|_| "{}".to_string())
}

/// Append a `## Next steps` section to `md` listing the suggestions. A no-op
/// when there are none, so flows can pass an empty list unconditionally.
pub fn append_next_steps(md: &mut Markdown, suggestions: &[Suggestion]) {
    if suggestions.is_empty() {
        return;
    }
    md.heading("Next steps");
    md.bullets(suggestions.iter().map(Suggestion::to_bullet));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bullet_includes_tool_and_args() {
        let s = Suggestion::new(
            "Watch submissions arrive",
            "tinyplace_submissions",
            json!({ "bounty_id": "bnt_123" }),
        );
        let bullet = s.to_bullet();
        assert!(bullet.contains("Watch submissions arrive"));
        assert!(bullet.contains("`tinyplace_submissions`"));
        assert!(bullet.contains(r#"{"bounty_id":"bnt_123"}"#));
    }

    #[test]
    fn empty_args_render_without_with_clause() {
        let s = Suggestion::new("Confirm identity", "tinyplace_whoami", json!({}));
        assert_eq!(
            s.to_bullet(),
            "**Confirm identity** — call `tinyplace_whoami`"
        );
    }

    #[test]
    fn next_steps_section_is_omitted_when_empty() {
        let mut md = Markdown::new();
        append_next_steps(&mut md, &[]);
        assert_eq!(md.build(), "");
    }

    #[test]
    fn next_steps_section_lists_all() {
        let mut md = Markdown::new();
        append_next_steps(
            &mut md,
            &[
                Suggestion::new("A", "tinyplace_a", json!({})),
                Suggestion::new("B", "tinyplace_b", json!({ "x": 1 })),
            ],
        );
        let out = md.build();
        assert!(out.starts_with("## Next steps"));
        assert!(out.contains("`tinyplace_a`"));
        assert!(out.contains("`tinyplace_b`"));
    }
}
