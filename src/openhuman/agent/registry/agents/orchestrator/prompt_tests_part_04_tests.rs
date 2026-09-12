use super::*;

#[test]
fn the_archetype_never_names_a_withheld_tool() {
    let named = withheld_names_presented_as_callable(ARCHETYPE);
    assert!(
        named.is_empty(),
        "orchestrator/prompt.md names withheld tools as if directly callable: {named:?}. \
         Route them through `use_skill` instead, or unpack them."
    );
}

#[test]
fn the_rendered_prompt_never_names_a_withheld_tool() {
    let body = build(&ctx_with(&[])).unwrap();
    assert!(
        !body.contains("## Capabilities not in your tool list"),
        "an empty visible set means no filter, so nothing can be withheld"
    );
    let named = withheld_names_presented_as_callable(&body);
    assert!(
        named.is_empty(),
        "the rendered orchestrator prompt names withheld tools as if directly \
         callable: {named:?}"
    );
}

fn withheld_names_presented_as_callable(text: &str) -> Vec<&'static str> {
    let packed = crate::openhuman::tools::toolpacks::all_packed_tool_names();
    const HEADING: &str = "## Capabilities not in your tool list";
    let mut prose = match text.find(HEADING) {
        Some(start) => {
            // Search for the next heading strictly after this one's own text
            // (`start + HEADING.len()`, not `start + 1`) — both indices land on
            // an ASCII byte, so this can never split a multi-byte UTF-8
            // character or run past `text.len()`.
            let search_from = start + HEADING.len();
            let end = text[search_from..]
                .find("\n## ")
                .map(|i| search_from + i)
                .unwrap_or(text.len());
            format!("{}{}", &text[..start], &text[end..])
        }
        None => text.to_string(),
    };
    for pack in crate::openhuman::tools::toolpacks::PACKS {
        for name in pack.tools {
            prose = prose.replace(&format!("skill `{}`, tool `{name}`", pack.id), "");
        }
    }
    for name in &packed {
        prose = prose.replace(&format!("skill `{name}`"), "");
    }
    packed
        .into_iter()
        .filter(|name| prose.contains(&format!("`{name}`")))
        .collect()
}
