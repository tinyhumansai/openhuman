//! Markdown rendering helpers for the tiny.place agent flow tools.
//!
//! The agent flow surface deliberately hands the LLM **markdown**, never raw
//! JSON: markdown is materially cheaper in tokens and far easier for the model
//! to act on. Every flow tool builds its result through these helpers so the
//! formatting stays consistent across the whole surface.

use serde_json::Value;

/// Accumulates markdown sections into one document.
///
/// Thin wrapper over a `String` that keeps blank-line separation between
/// sections uniform so individual flows don't each re-implement spacing.
#[derive(Default)]
pub struct Markdown {
    buf: String,
}

impl Markdown {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a level-2 heading (`## title`).
    pub fn heading(&mut self, title: impl AsRef<str>) -> &mut Self {
        self.section(format!("## {}", title.as_ref().trim()))
    }

    /// Append a level-3 heading (`### title`).
    pub fn subheading(&mut self, title: impl AsRef<str>) -> &mut Self {
        self.section(format!("### {}", title.as_ref().trim()))
    }

    /// Append a free-form paragraph.
    pub fn paragraph(&mut self, text: impl AsRef<str>) -> &mut Self {
        self.section(text.as_ref().trim().to_string())
    }

    /// Append a bullet list. Empty lists are skipped.
    pub fn bullets<I, S>(&mut self, items: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let rendered: Vec<String> = items
            .into_iter()
            .map(|item| format!("- {}", item.as_ref().trim()))
            .collect();
        if rendered.is_empty() {
            return self;
        }
        self.section(rendered.join("\n"))
    }

    /// Append a key/value definition block rendered as `**key:** value` lines.
    pub fn kv<I, K, V>(&mut self, pairs: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let rendered: Vec<String> = pairs
            .into_iter()
            .map(|(k, v)| format!("- **{}:** {}", k.as_ref().trim(), v.as_ref().trim()))
            .collect();
        if rendered.is_empty() {
            return self;
        }
        self.section(rendered.join("\n"))
    }

    /// Append a markdown table. `headers` are the column titles; each row must
    /// have the same arity as `headers`. Empty `rows` renders nothing.
    pub fn table<H, R>(&mut self, headers: &[H], rows: R) -> &mut Self
    where
        H: AsRef<str>,
        R: IntoIterator<Item = Vec<String>>,
    {
        let rows: Vec<Vec<String>> = rows.into_iter().collect();
        if rows.is_empty() || headers.is_empty() {
            return self;
        }
        let header_line = format!(
            "| {} |",
            headers
                .iter()
                .map(|h| h.as_ref().trim().to_string())
                .collect::<Vec<_>>()
                .join(" | ")
        );
        let divider = format!(
            "| {} |",
            headers
                .iter()
                .map(|_| "---")
                .collect::<Vec<_>>()
                .join(" | ")
        );
        let mut lines = vec![header_line, divider];
        for row in rows {
            lines.push(format!(
                "| {} |",
                row.iter()
                    .map(|cell| escape_cell(cell))
                    .collect::<Vec<_>>()
                    .join(" | ")
            ));
        }
        self.section(lines.join("\n"))
    }

    /// Append a raw, already-formatted section verbatim (used by the
    /// suggestions block and other composed fragments).
    pub fn raw_section(&mut self, section: impl Into<String>) -> &mut Self {
        let section = section.into();
        if section.trim().is_empty() {
            return self;
        }
        self.section(section)
    }

    fn section(&mut self, section: String) -> &mut Self {
        if !self.buf.is_empty() {
            self.buf.push_str("\n\n");
        }
        self.buf.push_str(section.trim_end());
        self
    }

    pub fn build(self) -> String {
        self.buf
    }
}

/// Escape a table cell so pipes and newlines don't break the row.
fn escape_cell(cell: &str) -> String {
    cell.replace('\n', " ")
        .replace('|', "\\|")
        .trim()
        .to_string()
}

/// Render a JSON scalar to a short human string for tables/kv blocks.
///
/// Objects and arrays collapse to a compact one-line form so a table cell
/// stays readable; callers that want full nesting should pull the field out
/// explicitly instead.
pub fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "—".to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.trim().to_string(),
        Value::Array(items) => format!("{} item(s)", items.len()),
        Value::Object(map) => format!("{{{} field(s)}}", map.len()),
    }
}

/// Look up a string field on a JSON object, returning `"—"` when missing so
/// table cells are never blank.
pub fn field<'a>(obj: &'a Value, key: &str) -> String {
    obj.get(key).map(scalar).unwrap_or_else(|| "—".to_string())
}

/// Truncate a body of text to `max` chars with an ellipsis, collapsing
/// internal newlines — for feed/message previews inside a table or list.
pub fn preview(text: &str, max: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max {
        return collapsed;
    }
    let truncated: String = collapsed.chars().take(max.saturating_sub(1)).collect();
    format!("{truncated}…")
}

/// Whether a JSON value is a scalar (renders inline in a kv/table cell).
fn is_scalar(value: &Value) -> bool {
    !matches!(value, Value::Array(_) | Value::Object(_))
}

/// Humanise a camelCase / snake_case key into a Title Case label.
pub fn humanize_key(key: &str) -> String {
    let spaced = key
        .chars()
        .flat_map(|c| {
            if c.is_ascii_uppercase() {
                vec![' ', c.to_ascii_lowercase()]
            } else if c == '_' {
                vec![' ']
            } else {
                vec![c]
            }
        })
        .collect::<String>();
    let mut out = String::with_capacity(spaced.len());
    let mut start_word = true;
    for c in spaced.trim().chars() {
        if start_word && c.is_ascii_alphabetic() {
            out.extend(c.to_uppercase());
            start_word = false;
        } else {
            out.push(c);
            if c == ' ' {
                start_word = true;
            }
        }
    }
    out
}

/// Generic JSON → markdown renderer used by the graphql gateway and raw escape
/// hatch (and as a flow fallback). Goes at most two levels deep so the output
/// stays scannable:
///
/// * an **object** renders its scalar fields as a kv block, then each nested
///   array/object as its own subsection;
/// * an **array of objects** renders as a table over the union of the first
///   row's scalar keys;
/// * an **array of scalars** renders as a bullet list.
pub fn render_json(value: &Value) -> String {
    let mut md = Markdown::new();
    render_into(&mut md, value, 0);
    let out = md.build();
    if out.is_empty() {
        "_(empty result)_".to_string()
    } else {
        out
    }
}

const MAX_TABLE_ROWS: usize = 40;

fn render_into(md: &mut Markdown, value: &Value, depth: usize) {
    match value {
        Value::Object(map) => {
            let scalars: Vec<(String, String)> = map
                .iter()
                .filter(|(_, v)| is_scalar(v))
                .map(|(k, v)| (humanize_key(k), scalar(v)))
                .collect();
            md.kv(scalars);
            if depth >= 2 {
                return;
            }
            for (key, child) in map.iter().filter(|(_, v)| !is_scalar(v)) {
                let label = humanize_key(key);
                match child {
                    Value::Array(items) if items.is_empty() => {
                        md.kv([(label, "none".to_string())]);
                    }
                    _ => {
                        md.subheading(&label);
                        render_into(md, child, depth + 1);
                    }
                }
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                md.paragraph("_(none)_");
                return;
            }
            let objects: Vec<&serde_json::Map<String, Value>> =
                items.iter().filter_map(Value::as_object).collect();
            if objects.len() == items.len() {
                render_table_of_objects(md, &objects);
            } else {
                md.bullets(items.iter().map(scalar));
            }
        }
        other => {
            md.paragraph(scalar(other));
        }
    }
}

fn render_table_of_objects(md: &mut Markdown, objects: &[&serde_json::Map<String, Value>]) {
    // Column set: the scalar keys of the first row (stable, predictable order).
    let headers: Vec<String> = objects[0]
        .iter()
        .filter(|(_, v)| is_scalar(v))
        .map(|(k, _)| k.clone())
        .collect();
    if headers.is_empty() {
        // Rows have no scalar fields — fall back to a compact count list.
        md.bullets(objects.iter().map(|o| format!("{{{} field(s)}}", o.len())));
        return;
    }
    let header_labels: Vec<String> = headers.iter().map(|k| humanize_key(k)).collect();
    let rows = objects.iter().take(MAX_TABLE_ROWS).map(|obj| {
        headers
            .iter()
            .map(|h| obj.get(h).map(scalar).unwrap_or_else(|| "—".to_string()))
            .collect::<Vec<_>>()
    });
    md.table(&header_labels, rows);
    if objects.len() > MAX_TABLE_ROWS {
        md.paragraph(format!(
            "_…and {} more (refine with limit/offset)_",
            objects.len() - MAX_TABLE_ROWS
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sections_are_blank_line_separated() {
        let mut md = Markdown::new();
        md.heading("Title").paragraph("Body text.");
        assert_eq!(md.build(), "## Title\n\nBody text.");
    }

    #[test]
    fn empty_collections_render_nothing() {
        let mut md = Markdown::new();
        md.bullets(Vec::<String>::new())
            .table::<&str, Vec<Vec<String>>>(&["A"], vec![]);
        assert_eq!(md.build(), "");
    }

    #[test]
    fn table_escapes_pipes_and_newlines() {
        let mut md = Markdown::new();
        md.table(&["Col"], vec![vec!["a|b\nc".to_string()]]);
        let out = md.build();
        assert!(out.contains("a\\|b c"), "got: {out}");
        assert!(out.contains("| --- |"));
    }

    #[test]
    fn scalar_collapses_containers() {
        assert_eq!(scalar(&json!("hi ")), "hi");
        assert_eq!(scalar(&json!(null)), "—");
        assert_eq!(scalar(&json!([1, 2, 3])), "3 item(s)");
        assert_eq!(scalar(&json!({"a":1})), "{1 field(s)}");
    }

    #[test]
    fn preview_truncates_on_char_boundary() {
        assert_eq!(preview("one two three", 100), "one two three");
        assert_eq!(preview("abcdef", 4), "abc…");
    }
}
