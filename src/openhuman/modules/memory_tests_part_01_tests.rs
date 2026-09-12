//! Split off memory_tests.rs to stay under the repo's line-count gate.
//! Same module, same imports — see memory_tests.rs for what this covers.
use super::*;

/// The runtime-tree and flavour doors, driven against a **real** module.
///
/// The test above proves the `module_call!` arms exist by discriminating
/// `Other` from `Unsupported` against a disabled host; it cannot prove the wire
/// names are right, because a mistyped one fails the same way a disabled host
/// does. This one can: it loads an actual artifact and asserts the answers.
///
/// # What it deliberately does not drive
///
/// `runtime_summarize` and `runtime_rebuild` resolve a chat model on the
/// driver's side and then spend on it. A test that called them would either
/// reach the network or assert against a provider-resolution failure, and
/// neither says anything about the door. The five below are store-shaped and
/// answer from a fresh workspace with no ambiguity: a buffered write reports
/// where it landed, an empty tree has no root and no children, its status is
/// all zeroes, and nothing has been distilled for a persona scope.
///
/// Run it against a locally built module, one test per process:
///
/// ```text
/// TINYMEMORY_TEST_MODULE=/path/to/libtinymemory_module.dylib \
///   cargo test --lib -- --ignored --exact --test-threads=1 \
///   openhuman::modules::memory::tests::part_01_tests::the_runtime_tree_doors_round_trip_through_a_real_module
/// ```
#[tokio::test]
#[ignore = "needs a built tinymemory module (TINYMEMORY_TEST_MODULE) and its own process: \
the bus belongs to whichever runtime creates it, so a second module-loading test in the same \
process finds a broker whose tasks are already gone and hangs rather than failing"]
async fn the_runtime_tree_doors_round_trip_through_a_real_module() {
    let module = std::env::var_os("TINYMEMORY_TEST_MODULE")
        .expect("set TINYMEMORY_TEST_MODULE to a built libtinymemory_module cdylib");
    let workspace = tempfile::TempDir::new().expect("tempdir");

    let mut config = Config::default();
    config.workspace_dir = workspace.path().to_path_buf();
    config.modules.enabled = true;
    config.modules.install_dir = Some(
        workspace
            .path()
            .join("modules")
            .to_string_lossy()
            .into_owned(),
    );
    config
        .modules
        .overrides
        .push(crate::openhuman::config::schema::ModuleOverride {
            id: MODULE_ID.to_string(),
            path: module.to_string_lossy().into_owned(),
        });

    let provider = ModuleMemoryProvider::new(Arc::new(config));
    let tree = provider.as_tree().expect("the Tree family");
    let at = chrono::Utc::now();

    let path = tree
        .runtime_buffer_write("team", "standup notes", at, None)
        .await
        .expect("RuntimeBufferWrite must reach the module");
    assert!(
        !path.trim().is_empty(),
        "the buffered write reports where it landed"
    );

    assert!(
        tree.runtime_read_node("team", "root")
            .await
            .expect("RuntimeReadNode must reach the module")
            .is_none(),
        "a buffered write creates no nodes; absence is data, not an error"
    );
    assert!(
        tree.runtime_read_children("team", "root")
            .await
            .expect("RuntimeReadChildren must reach the module")
            .is_empty(),
        "a parent that does not exist has no children"
    );

    let status = tree
        .runtime_tree_status("team")
        .await
        .expect("RuntimeTreeStatus must reach the module");
    assert_eq!(status.namespace, "team");
    assert_eq!(status.total_nodes, 0);
    assert_eq!(status.depth, 0);

    assert!(
        tree.flavour_profile("persona/communication")
            .await
            .expect("FlavourProfile must reach the module")
            .is_none(),
        "nothing has been distilled for this scope yet"
    );

    // The two refusals the doors make before touching the store, so a wrong
    // wire name cannot pass this test by answering plausibly to everything.
    let rejected = tree
        .runtime_buffer_write("../escape", "x", at, None)
        .await
        .expect_err("a traversal namespace is refused");
    assert!(
        matches!(rejected, MemoryError::Invalid(_)),
        "a rejected namespace is a caller mistake, not a backend failure: {rejected:?}"
    );
    let blank = tree
        .runtime_buffer_write("team", "   ", at, None)
        .await
        .expect_err("blank content is refused");
    assert!(
        matches!(blank, MemoryError::Invalid(_)),
        "blank content is a caller mistake: {blank:?}"
    );
}

#[test]
fn scoring_is_advertised_and_has_a_host_accessor() {
    // tinymemory v1.13.2 (tinymemory#110) added the family; advertising it and
    // forwarding it must land together, or the driver claims a family whose
    // accessor answers `None` — the #5598 over-claim in miniature.
    let mut config = Config::default();
    config.modules.enabled = false;
    let provider = ModuleMemoryProvider::new(Arc::new(config));
    assert!(super::super::capabilities_for(false).contains(Capability::Scoring));
    assert!(
        provider.as_scoring().is_some(),
        "Scoring is advertised, so the accessor must be wired"
    );
}

/// Every operation label in the client is classified, and no mutation reached
/// the read list.
///
/// The classification has been wrong in both directions, so this checks both.
///
/// It first named the *writes* and let everything else be a read, which
/// silently bounded two dozen mutations (#6006 review) — a cold launch would
/// answer `Unavailable` to an ingest nobody retries. Inverting it fixed the
/// lost writes but left the list merely *safe*, not complete: it named 37 of
/// 141 labels, so `entities`, `relations`, `summary_forest`,
/// `retrieve_children` and 37 other genuine reads still waited out the entire
/// module download — the tree, graph and sources panels this change exists to
/// unblock, still blank on the launch that motivated it.
///
/// So the assertion is a partition, not a filter: every label the sources
/// dispatch must appear in `BOUNDED_READ_OPERATIONS` or in
/// `UNBOUNDED_WRITE_OPERATIONS` below, and no label may be in both. A new
/// member fails here rather than in the field, where a misclassified write is
/// a lost write with nothing in the log and a misclassified read is a panel
/// that hangs for the length of a download.
#[test]
fn every_operation_label_is_classified_and_no_mutation_is_a_read() {
    // Written the way the call sites are: `self.proxy("x")` directly, or the
    // operation literal handed to one of the two dispatch macros.
    // `\s*` between the macro name and the literal is load-bearing: most call
    // sites wrap, and a pattern anchored to one line saw 70 of the 141 labels
    // and reported a pass on the half it could see.
    let call_site = regex::Regex::new(
        r#"(?:proxy\(\s*|module_call!\(\s*self,\s*|module_call_slow!\(\s*self,\s*)"([a-z_]+)""#,
    )
    .expect("a valid pattern");

    let mut labels: Vec<String> = Vec::new();
    // Discovered, not enumerated. A hard-coded `memory_part_01..04` would keep
    // passing after the file is split differently — scanning fewer sources,
    // finding fewer labels, and quietly checking less than it claims to.
    let modules_dir = format!("{}/src/openhuman/modules", env!("CARGO_MANIFEST_DIR"));
    let mut parts: Vec<std::path::PathBuf> = std::fs::read_dir(&modules_dir)
        .unwrap_or_else(|error| panic!("read {modules_dir}: {error}"))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("memory_part_")
                        && name.ends_with(".rs")
                        && !name.contains("_tests")
                })
        })
        .collect();
    parts.sort();
    assert!(
        parts.len() >= 4,
        "expected the memory client to be split across at least four parts, found {:?} — if the \
         split changed, this scan is looking in the wrong place",
        parts
    );

    for path in &parts {
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for captured in call_site.captures_iter(&source) {
            labels.push(captured[1].to_string());
        }
    }
    labels.sort();
    labels.dedup();
    assert!(
        labels.len() >= 130,
        "the scan found only {} labels; the call-site pattern has drifted from the code",
        labels.len()
    );

    // The other half of the partition. `BOUNDED_READ_OPERATIONS` names the
    // reads; this names every label that must keep waiting. Together they have
    // to cover the scan exactly, which is what turns "no write is a read" into
    // "every member is classified" — the gap that let 41 genuine reads sit
    // unlisted and wait out the whole download on a cold launch.
    const UNBOUNDED_WRITE_OPERATIONS: &[&str] = &[
        "accept_source_items",
        "add_handle_alias",
        "append",
        "append_turn",
        // openhuman#6012: re-files stored connector documents into the memory
        // tree, so it writes and must wait rather than answer "still loading".
        "backfill_connector_trees",
        "bootstrap_connection",
        "capture_snapshot",
        "cascade",
        "clear_namespace",
        "close_segment",
        "compact",
        "consolidate",
        "create_segment",
        "delete_document",
        "delete_facet",
        "delete_facet_by_id",
        "delete_tool_rule",
        "drop_facets_below",
        "flush_pending",
        "flush_source_tree",
        "forget",
        "forget_matching",
        "forget_source",
        "import_records",
        "ingest_chat",
        "ingest_coding_sessions",
        "ingest_document",
        "ingest_email",
        "insert_event",
        "insert_turn",
        "kv_delete",
        "kv_put",
        "open_segment",
        "override_scheduler_gate",
        "purge_all",
        "put_document",
        "put_relation",
        "put_tool_rule",
        "rebuild_from_raw_archive",
        "record_interaction",
        "reembed",
        "reset_derived_index",
        "retry_failed",
        "run_connection_sync",
        "run_source_sync",
        "runtime_buffer_write",
        "runtime_rebuild",
        "runtime_summarize",
        "seal",
        "seed_from_address_book",
        "set_facet_user_state",
        "set_goals",
        "set_segment_summary",
        "shutdown",
        "store",
        "summarise",
        "touch_entities",
        "typed_ingest_conversation",
        "typed_ingest_document",
        "typed_ingest_event",
        "typed_ingest_learning",
        "upsert_facet",
        "upsert_provider_facet",
        "upsert_segment_embedding",
    ];

    // A label carrying one of these is a mutation by name. The read list must
    // not contain one, whatever a future edit believes.
    const MUTATING: &[&str] = &[
        "store",
        "forget",
        "purge",
        "delete",
        "put_",
        "set_",
        "insert_",
        "ingest",
        "append",
        "upsert",
        "import",
        "reembed",
        "compact",
        "consolidate",
        "cascade",
        "seal",
        "flush",
        "retry_",
        "run_",
        "bootstrap_",
        "override_",
        "shutdown",
        "summaris",
        "open_segment",
    ];
    // Reads whose names carry a mutation marker anyway: `store_stats` reads
    // where `store` writes, and `source_ingest_status` reports on ingestion
    // rather than performing it. Named here so the marker list above can stay
    // blunt — a marker that has to dodge every compound name stops catching
    // the mutations it is for.
    const READS_DESPITE_A_MUTATING_MARKER: &[&str] = &["source_ingest_status", "store_stats"];

    let provider = provider();
    for label in &labels {
        // Match the whole word for the labels that are a prefix of a
        // legitimate read.
        let mutates = MUTATING.iter().any(|marker| {
            if marker.ends_with('_') {
                label.starts_with(marker)
            } else {
                label == marker
                    || label.starts_with(&format!("{marker}_"))
                    || label.contains(marker)
            }
        }) && !READS_DESPITE_A_MUTATING_MARKER.contains(&label.as_str());
        if mutates {
            assert_eq!(
                provider.loading_grace(label),
                None,
                "{label} names a mutation but is classified as a bounded read"
            );
        }
    }

    // Every discovered label lands in exactly one half.
    let unclassified: Vec<&str> = labels
        .iter()
        .map(String::as_str)
        .filter(|label| {
            provider.loading_grace(label).is_none() && !UNBOUNDED_WRITE_OPERATIONS.contains(label)
        })
        .collect();
    assert!(
        unclassified.is_empty(),
        "these dispatch labels are classified as neither a bounded read nor a \
         write that must wait: {unclassified:?} — add each to \
         BOUNDED_READ_OPERATIONS if it cannot mutate, or to \
         UNBOUNDED_WRITE_OPERATIONS if it can"
    );

    for label in UNBOUNDED_WRITE_OPERATIONS {
        assert_eq!(
            provider.loading_grace(label),
            None,
            "{label} is listed as a write that must wait but is classified as a bounded read"
        );
        assert!(
            labels.iter().any(|found| found == label),
            "{label} is listed as a write but no call site dispatches it; the list has gone stale"
        );
    }
}
