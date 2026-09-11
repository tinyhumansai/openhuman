use super::*;
use crate::openhuman::agent::learning::candidate::{Buffer, CueFamily, FacetClass};

fn make_summary(facets: Vec<ParsedFacet>) -> StructuredSummary {
    StructuredSummary {
        summary: "A summary.".into(),
        facets,
    }
}

fn make_facet(
    class: &str,
    key: &str,
    value: &str,
    evidence_chunks: Vec<&str>,
    confidence: f64,
    cue_family: &str,
) -> ParsedFacet {
    ParsedFacet {
        class: class.into(),
        key: key.into(),
        value: value.into(),
        evidence_chunks: evidence_chunks.into_iter().map(str::to_string).collect(),
        confidence,
        cue_family: cue_family.into(),
    }
}

#[test]
fn parse_well_formed_structured_summary() {
    let json = r#"{
        "summary": "The user prefers pnpm.",
        "facets": [
            {
                "class": "tooling",
                "key": "package_manager",
                "value": "pnpm",
                "evidence_chunks": ["chunk-abc"],
                "confidence": 0.85,
                "cue_family": "explicit"
            }
        ]
    }"#;
    let parsed: StructuredSummary = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.summary, "The user prefers pnpm.");
    assert_eq!(parsed.facets.len(), 1);
    assert_eq!(parsed.facets[0].key, "package_manager");
    assert_eq!(parsed.facets[0].cue_family, "explicit");
}

#[test]
fn drops_facet_with_unknown_class() {
    let buf = Buffer::new(64);
    let before = buf.len();

    // Route into global but we check the drop via tracing — here we test
    // that the function doesn't panic and the drop is silent.
    let s = make_summary(vec![make_facet(
        "unknown_class",
        "key",
        "val",
        vec!["c1"],
        0.8,
        "behavioral",
    )]);
    route_facets_to_buffer_into(&s, "src-1", &buf);
    // No new candidates pushed.
    assert_eq!(buf.len(), before, "unknown class should drop the facet");
}

#[test]
fn drops_facet_without_evidence_chunks() {
    let buf = Buffer::new(64);
    let s = make_summary(vec![make_facet(
        "style",
        "verbosity",
        "terse",
        vec![], // empty — must be dropped
        0.8,
        "explicit",
    )]);
    let before = buf.len();
    route_facets_to_buffer_into(&s, "src-2", &buf);
    assert_eq!(
        buf.len(),
        before,
        "facet without evidence_chunks must be dropped"
    );
}

#[test]
fn defaults_cue_family_to_behavioral() {
    let json = r#"{
        "summary": "x",
        "facets": [
            {
                "class": "style",
                "key": "verbosity",
                "value": "terse",
                "evidence_chunks": ["c1"],
                "confidence": 0.7
            }
        ]
    }"#;
    let parsed: StructuredSummary = serde_json::from_str(json).unwrap();
    assert_eq!(parsed.facets[0].cue_family, "behavioral");
}

#[test]
fn route_pushes_to_buffer() {
    let buf = Buffer::new(64);
    let s = make_summary(vec![
        make_facet(
            "style",
            "verbosity",
            "terse",
            vec!["chunk-1"],
            0.75,
            "explicit",
        ),
        make_facet(
            "identity",
            "timezone",
            "UTC+5:30",
            vec!["chunk-2"],
            0.9,
            "structural",
        ),
    ]);
    let before = buf.len();
    route_facets_to_buffer_into(&s, "notion:doc-1", &buf);
    let after = buf.len();
    assert_eq!(
        after,
        before + 2,
        "two valid facets should push two candidates"
    );

    let all = buf.peek();
    let tz = all.iter().find(|c| c.key == "timezone");
    let tz = tz.expect("timezone candidate in buffer");
    assert_eq!(tz.value, "UTC+5:30");
    assert_eq!(tz.class, FacetClass::Identity);
    assert_eq!(tz.cue_family, CueFamily::Structural);
    assert!(
        matches!(&tz.evidence, EvidenceRef::DocumentChunk { source_id, chunk_id }
        if source_id == "notion:doc-1" && chunk_id == "chunk-2")
    );
}
