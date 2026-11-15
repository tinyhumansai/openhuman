//! One artefact holding everything a turn ships before the user has spoken.
//!
//! `dump-prompt` wrote the system prompt and `dump-all` wrote the tool schemas
//! into a *sibling* `.tools.json`, pretty-printed. Both halves existed, and
//! neither was what the model receives: the prompt alone is the smaller half of
//! the fixed cost, and pretty-printing inflates the schemas by roughly a third
//! in indentation the model never sees. Reading the two files together and
//! mentally minifying one of them is not a thing anyone does, so in practice
//! the schema half went unlooked-at — which is how sixteen copies of one
//! delegation envelope sat in the orchestrator's budget unnoticed.
//!
//! This renders both halves, in the form they are actually sent, with the
//! byte counts beside them.
//!
//! # What "as sent" means here, precisely
//!
//! Two things are byte-exact: the system prompt text, and each tool schema
//! **minified** with `serde_json::to_string` — the same call
//! `toolpacks::tools::render_pack` uses, for the same reason.
//!
//! What this deliberately does *not* do is fabricate an HTTP body. The
//! surrounding envelope (message array, provider-specific `tools` vs
//! `functions` key, sampling parameters) differs per provider and is built
//! elsewhere; inventing one here would produce a file that *looks* like a
//! captured request and is not. The two payload halves are labelled and exact;
//! the framing around them is presented as framing.
//!
//! # Dialects
//!
//! Under `ToolCallFormat::Native` the schemas ride beside the prompt as
//! structured JSON, which is the shape below. Under `PFormat` / `Json` the same
//! tools are rendered *into* the prompt text as a catalogue, so they are
//! already counted in the prompt half and the array below is what the harness
//! would advertise natively. Either way the total is the total, which is why
//! the header reports it as one number.

use std::fmt::Write as _;

use super::DumpedPrompt;

/// A rough token estimate. Bytes ÷ 4 — the same ratio `prompt-size` uses, kept
/// identical so the two tools never disagree about the same prompt.
fn est_tokens(bytes: usize) -> usize {
    bytes / 4
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The exact bytes of one tool's schema as it goes on the wire.
///
/// Minified, because that is what is sent. A pretty-printed schema is a
/// different — larger — number, and reporting it would overstate every tool.
pub fn tool_schema_bytes(spec: &serde_json::Value) -> usize {
    serde_json::to_string(spec).map(|s| s.len()).unwrap_or(0)
}

/// Total advertised tool-schema bytes for a dump.
pub fn total_tool_bytes(dumped: &DumpedPrompt) -> usize {
    dumped.tool_specs.iter().map(tool_schema_bytes).sum()
}

/// Render the full fixed prefix: header, system prompt, tool schemas.
///
/// The output is plain text and deliberately greppable — `dump-all` writes it
/// to `{stem}.wire.txt` and `dump-prompt --wire` prints it to stdout, so the
/// same bytes are reviewable either way.
pub fn render(dumped: &DumpedPrompt) -> String {
    let prompt_bytes = dumped.text.len();
    let tool_bytes = total_tool_bytes(dumped);
    let total = prompt_bytes + tool_bytes;

    let mut out = String::with_capacity(total + 4096);

    out.push_str("════════════════════════════════════════════════════════════════════\n");
    out.push_str(" WHAT THE MODEL RECEIVES, BEFORE THE USER SAYS ANYTHING\n");
    out.push_str("════════════════════════════════════════════════════════════════════\n");
    let _ = writeln!(out, " agent          {}", dumped.agent_id);
    let _ = writeln!(out, " model          {}", dumped.model);
    let _ = writeln!(out, " workspace      {}", dumped.workspace_dir.display());
    out.push('\n');
    let _ = writeln!(
        out,
        " system prompt  {:>10} B   ~{:>7} tok",
        thousands(prompt_bytes),
        thousands(est_tokens(prompt_bytes))
    );
    let _ = writeln!(
        out,
        " tool schemas   {:>10} B   ~{:>7} tok   ({} tools, minified as sent)",
        thousands(tool_bytes),
        thousands(est_tokens(tool_bytes)),
        dumped.tool_specs.len()
    );
    let _ = writeln!(
        out,
        " ─────────────  {:>10} B   ~{:>7} tok   charged on EVERY turn",
        thousands(total),
        thousands(est_tokens(total))
    );
    out.push('\n');

    out.push_str("────────────────────────────────────────────────────────────────────\n");
    let _ = writeln!(
        out,
        " 1/2  SYSTEM PROMPT  ·  role: system  ·  {} B  ·  verbatim",
        thousands(prompt_bytes)
    );
    out.push_str("────────────────────────────────────────────────────────────────────\n\n");
    out.push_str(&dumped.text);
    if !dumped.text.ends_with('\n') {
        out.push('\n');
    }

    out.push('\n');
    out.push_str("────────────────────────────────────────────────────────────────────\n");
    let _ = writeln!(
        out,
        " 2/2  TOOL SCHEMAS  ·  {} tools  ·  {} B  ·  one per line, minified",
        dumped.tool_specs.len(),
        thousands(tool_bytes)
    );
    out.push_str("────────────────────────────────────────────────────────────────────\n\n");

    if dumped.tool_specs.is_empty() {
        // Not an omission, and worth saying so: an agent can legitimately
        // advertise nothing (`named = []`), and a blank section here would read
        // like the dumper failed rather than like the agent is tool-less.
        out.push_str("(none — this agent advertises no tools)\n");
        return out;
    }

    // Widest-first, so the reader meets the expensive schemas before the cheap
    // ones. Registration order carries no information anyone wants here, and
    // the point of this file is to name what to cut.
    let mut ordered: Vec<(usize, &serde_json::Value)> = dumped
        .tool_specs
        .iter()
        .map(|spec| (tool_schema_bytes(spec), spec))
        .collect();
    ordered.sort_by(|a, b| {
        b.0.cmp(&a.0).then_with(|| {
            let name = |v: &serde_json::Value| {
                v.get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            name(a.1).cmp(&name(b.1))
        })
    });

    for (bytes, spec) in ordered {
        let name = spec
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("<unnamed>");
        let _ = writeln!(out, "# {name}  ({} B)", thousands(bytes));
        let _ = writeln!(
            out,
            "{}",
            serde_json::to_string(spec).unwrap_or_else(|_| "{}".to_string())
        );
        out.push('\n');
    }

    out
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
