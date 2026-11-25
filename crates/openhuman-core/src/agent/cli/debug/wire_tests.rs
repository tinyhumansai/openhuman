//! Tests for the wire dump.
//!
//! The load-bearing one is `schemas_are_minified_not_pretty_printed`: the whole
//! reason this renderer exists is that the pre-existing `.tools.json` sidecar
//! was pretty-printed, which overstates every schema by roughly a third.

use super::*;
use std::path::PathBuf;

fn dump(text: &str, specs: Vec<serde_json::Value>) -> DumpedPrompt {
    DumpedPrompt {
        agent_id: "test_agent".to_string(),
        mode: "session",
        model: "test-model".to_string(),
        workspace_dir: PathBuf::from("/tmp/ws"),
        text: text.to_string(),
        tool_names: specs
            .iter()
            .filter_map(|s| s.get("name").and_then(|n| n.as_str()).map(str::to_string))
            .collect(),
        skill_tool_count: 0,
        tool_specs: specs,
    }
}

fn spec(name: &str, description: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": description,
        "parameters": { "type": "object", "properties": { "q": { "type": "string" } } }
    })
}

#[test]
fn the_prompt_is_reproduced_verbatim() {
    // Byte-for-byte: the point of the artefact is that it can be diffed and
    // counted against the real thing. Any reformatting here would make the
    // numbers in the header describe a different document to the one below it.
    let text = "# Agent\n\nLine one.\n\n## Section\n\tindented\n";
    let rendered = render(&dump(text, vec![]));
    assert!(
        rendered.contains(text),
        "the exact prompt bytes must appear in the output"
    );
}

#[test]
fn schemas_are_minified_not_pretty_printed() {
    // The reason this renderer exists. `serde_json::to_vec_pretty` — what the
    // `.tools.json` sidecar uses — pads every schema with indentation the model
    // never receives, so a reader auditing that file is reading an inflated
    // number for every single tool.
    let s = spec("search", "Find things.");
    let rendered = render(&dump("prompt", vec![s.clone()]));

    let minified = serde_json::to_string(&s).unwrap();
    let pretty = serde_json::to_string_pretty(&s).unwrap();
    assert!(rendered.contains(&minified), "must carry the minified form");
    assert!(
        !rendered.contains(&pretty),
        "must not carry the pretty-printed form"
    );
    assert!(
        pretty.len() > minified.len(),
        "the two forms must actually differ, or this test proves nothing"
    );
}

#[test]
fn the_header_totals_the_two_halves() {
    let s = spec("search", "Find things.");
    let d = dump("abcdefghij", vec![s.clone()]);
    let expected = 10 + serde_json::to_string(&s).unwrap().len();
    let rendered = render(&d);

    assert_eq!(
        total_tool_bytes(&d),
        serde_json::to_string(&s).unwrap().len()
    );
    assert!(
        rendered.contains(&thousands(expected)),
        "header must report prompt + tools as one figure ({expected})"
    );
}

#[test]
fn tools_are_ordered_widest_first() {
    // The file's job is to name what to cut, so the expensive schema is the one
    // the reader should meet first.
    let small = spec("s", "x");
    let large = spec("l", &"y".repeat(400));
    let rendered = render(&dump("p", vec![small, large]));
    let pos_large = rendered.find("# l  (").expect("large tool listed");
    let pos_small = rendered.find("# s  (").expect("small tool listed");
    assert!(pos_large < pos_small, "widest schema must come first");
}

#[test]
fn a_tool_less_agent_says_so_rather_than_rendering_an_empty_section() {
    // `named = []` is a real declaration two shipped agents make. A blank
    // section would read as a broken dumper rather than a tool-less agent.
    let rendered = render(&dump("prompt", vec![]));
    assert!(rendered.contains("advertises no tools"));
    assert!(rendered.contains("0 tools"));
}

#[test]
fn every_tool_appears_with_its_own_byte_count() {
    let specs = vec![spec("alpha", "a"), spec("beta", "b"), spec("gamma", "c")];
    let d = dump("p", specs.clone());
    let rendered = render(&d);
    for s in &specs {
        let name = s["name"].as_str().unwrap();
        assert!(rendered.contains(&format!("# {name}  (")), "missing {name}");
    }
    // And the per-tool figures must sum to the header's total, or the file
    // would be internally inconsistent — the failure mode that makes a budget
    // report untrustworthy.
    let summed: usize = specs.iter().map(tool_schema_bytes).sum();
    assert_eq!(summed, total_tool_bytes(&d));
}

#[test]
fn thousands_groups_digits_without_mangling_short_numbers() {
    assert_eq!(thousands(0), "0");
    assert_eq!(thousands(7), "7");
    assert_eq!(thousands(999), "999");
    assert_eq!(thousands(1_000), "1,000");
    assert_eq!(thousands(29_821), "29,821");
    assert_eq!(thousands(1_073_644), "1,073,644");
}
