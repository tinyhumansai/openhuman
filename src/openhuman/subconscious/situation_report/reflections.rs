//! Recent reflections section — anti-double-emit context for the LLM (#623).
//!
//! Renders the last N persisted reflections so the model can decide to
//! decay a stale observation, strengthen one that's intensifying, or
//! skip emitting a duplicate.
//!
//! The caller does the actual loading from `subconscious_reflections`
//! (see `engine.rs` tick logic) so this module stays a pure formatter
//! and trivial to unit-test.

use std::fmt::Write;

use crate::openhuman::subconscious::reflection::Reflection;

/// Default cap on rendered reflections — `engine.rs` still supplies the
/// vector, but if more are passed we trim here so the prompt section
/// can't blow up.
const RENDER_CAP: usize = 8;

pub fn build_section(reflections: &[Reflection]) -> String {
    if reflections.is_empty() {
        return "## Recent reflections\n\nNone yet — first tick.\n".to_string();
    }

    let mut section = String::from("## Recent reflections\n\n");
    section.push_str(
        "Previous tick observations. Decide whether each still holds, has \
         intensified, or has decayed — emit a fresh reflection only on a \
         materially new signal:\n\n",
    );
    for r in reflections.iter().take(RENDER_CAP) {
        let _ = writeln!(
            section,
            "- [{id}] kind={kind} — {body}",
            id = r.id,
            kind = r.kind.as_str(),
            body = trim_for_prompt(&r.body),
        );
    }
    section
}

fn trim_for_prompt(text: &str) -> String {
    let single_line = text.replace('\n', " ");
    if single_line.chars().count() <= 200 {
        return single_line;
    }
    let mut out: String = single_line.chars().take(200).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::subconscious::reflection::{
        hydrate_draft, ReflectionDraft, ReflectionKind,
    };

    fn r(id: &str, body: &str) -> Reflection {
        hydrate_draft(
            ReflectionDraft {
                kind: ReflectionKind::HotnessSpike,
                body: body.into(),
                proposed_action: None,
                source_refs: vec![],
            },
            id.into(),
            1.0,
            Vec::new(),
        )
    }

    #[test]
    fn empty_renders_first_tick_message() {
        let s = build_section(&[]);
        assert!(s.contains("None yet — first tick"));
    }

    #[test]
    fn renders_each_reflection() {
        let s = build_section(&[r("a", "Phoenix surge"), r("b", "Calendar conflict")]);
        assert!(s.contains("[a]"));
        assert!(s.contains("Phoenix surge"));
        assert!(s.contains("[b]"));
        assert!(s.contains("Calendar conflict"));
    }

    #[test]
    fn caps_at_render_cap() {
        let many: Vec<Reflection> = (0..20).map(|i| r(&format!("r{i}"), "body")).collect();
        let s = build_section(&many);
        assert!(s.contains("[r0]"));
        assert!(s.contains(&format!("[r{}]", RENDER_CAP - 1)));
        // Past the cap should NOT appear.
        assert!(!s.contains(&format!("[r{}]", RENDER_CAP)));
    }
}
