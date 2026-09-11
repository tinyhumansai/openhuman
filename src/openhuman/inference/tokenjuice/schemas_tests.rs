use super::*;

#[test]
fn all_schemas_have_namespace() {
    for s in all_controller_schemas() {
        assert_eq!(s.namespace, "tokenjuice");
    }
}

/* Module-backed behavior is covered by TinyJuice's loader E2E. */
#[tokio::test]
#[ignore = "requires a built TinyJuice module"]
async fn detect_handler_classifies_json() {
    let mut p = Map::new();
    p.insert(
        "content".into(),
        Value::String(r#"[{"a":1,"b":2},{"a":3,"b":4}]"#.into()),
    );
    let out = handle_detect(p).await.unwrap();
    assert_eq!(out["kind"], "json");
}

#[tokio::test]
#[ignore = "requires a built TinyJuice module"]
async fn cache_stats_handler_returns_counts() {
    let out = handle_cache_stats(Map::new()).await.unwrap();
    assert!(out["entries"].is_u64());
}

// ── #6088: `compress` must expose all five ContentHint fields ────────────────

/// Before #6088 the handler built `ContentHint { source_tool, ..Default::default() }`,
/// so four of the five fields were unreachable and the debug controller could not
/// reproduce what the content router does. These assert the mapping directly —
/// no module, no router — so they run everywhere.
#[test]
fn compress_hint_carries_every_caller_supplied_field() {
    let mut p = Map::new();
    p.insert("tool_name".into(), Value::from("grep"));
    p.insert("mime".into(), Value::from("text/html"));
    p.insert("extension".into(), Value::from("rs"));
    p.insert("query".into(), Value::from("needle"));
    p.insert("explicit".into(), Value::from("search"));

    let hint = compress_hint_from_params(&p).expect("a well-formed hint must parse");
    assert_eq!(
        hint.source_tool.as_deref(),
        Some("grep"),
        "`tool_name` was the only field the old handler read"
    );
    assert_eq!(
        hint.mime.as_deref(),
        Some("text/html"),
        "`mime` must reach the router; it was dropped before #6088"
    );
    assert_eq!(
        hint.extension.as_deref(),
        Some("rs"),
        "`extension` must reach the router; it was dropped before #6088"
    );
    assert_eq!(
        hint.query.as_deref(),
        Some("needle"),
        "`query` must reach the router — the search compressor ranks by it"
    );
    assert_eq!(
        hint.explicit,
        Some(ContentKind::Search),
        "`explicit` is the field that skips detection entirely — without it the \
         controller cannot force a kind, which is the point of the fix"
    );
}

#[test]
fn compress_hint_defaults_every_field_to_none() {
    let mut p = Map::new();
    p.insert("content".into(), Value::from("hello"));
    let hint = compress_hint_from_params(&p).expect("an empty hint must parse");
    assert!(hint.source_tool.is_none());
    assert!(hint.mime.is_none());
    assert!(hint.extension.is_none());
    assert!(hint.query.is_none());
    assert!(hint.explicit.is_none());
}

#[test]
fn compress_hint_refuses_an_unknown_explicit_kind() {
    let mut p = Map::new();
    p.insert("explicit".into(), Value::from("nonsense"));
    let err = compress_hint_from_params(&p)
        .expect_err("an unrecognised kind must be refused, not silently detected");
    assert!(
        err.contains("nonsense") && err.contains("plain_text"),
        "the refusal must quote the bad value and list the accepted set: {err}"
    );
}

/// `plain_text` is the spelling `ContentKind::from_str` accepts; serde renders the
/// same variant as `plainText`. The vendored `from_str` doc calls this out as the
/// exact trap, so pin the spelling the schema advertises.
#[test]
fn compress_hint_accepts_the_documented_plain_text_spelling() {
    let mut p = Map::new();
    p.insert("explicit".into(), Value::from("plain_text"));
    assert_eq!(
        compress_hint_from_params(&p)
            .expect("plain_text must parse")
            .explicit,
        Some(ContentKind::PlainText),
        "`plain_text` is the spelling `ContentKind::from_str` accepts and the one \
         the schema advertises; serde's `plainText` must not be what we parse"
    );
}

#[test]
fn compress_schema_declares_all_five_hint_inputs() {
    let s = schemas("compress");
    for field in [
        "content",
        "tool_name",
        "mime",
        "extension",
        "query",
        "explicit",
    ] {
        assert!(
            s.inputs.iter().any(|f| f.name == field),
            "`compress` must declare `{field}`; a hint field the schema does not \
             name is one a caller cannot discover"
        );
    }
    assert!(
        s.inputs
            .iter()
            .find(|f| f.name == "content")
            .is_some_and(|f| f.required),
        "only `content` is required"
    );
    for optional in ["tool_name", "mime", "extension", "query", "explicit"] {
        assert!(
            !s.inputs
                .iter()
                .find(|f| f.name == optional)
                .unwrap()
                .required,
            "`{optional}` must stay optional"
        );
    }
}

/// `/schema` is what generators and model-facing tool definitions read. Declaring
/// `explicit` as a bare string there invites a caller to propose a value that only
/// fails once the handler runs; as an enum, `check_type` rejects it at the dispatch
/// boundary and the published contract matches the rejection.
#[test]
fn compress_declares_explicit_as_an_enum_of_the_accepted_kinds() {
    let s = schemas("compress");
    let explicit = s
        .inputs
        .iter()
        .find(|f| f.name == "explicit")
        .expect("`compress` must declare `explicit`");

    let TypeSchema::Option(inner) = &explicit.ty else {
        panic!(
            "`explicit` is optional and must be declared as such: {:?}",
            explicit.ty
        );
    };
    let TypeSchema::Enum { variants } = inner.as_ref() else {
        panic!(
            "`explicit` must be an enum so the accepted kinds are discoverable \
             from /schema rather than only from the handler: {inner:?}"
        );
    };
    assert_eq!(
        variants,
        &[
            "json",
            "code",
            "log",
            "search",
            "diff",
            "html",
            "plain_text"
        ],
        "the declared variants must be exactly what `ContentKind::from_str` parses \
         — note `plain_text`, not serde's `plainText`"
    );
}
