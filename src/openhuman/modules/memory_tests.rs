//! Tests for the memory module client.
//!
//! Nothing here loads a module. What is testable without one is what decides a
//! caller's next move: that construction is genuinely I/O-free, that the static
//! capability answer is the one the module actually serves, and that a bus error
//! comes back as the right `MemoryError` variant. The round trips are covered
//! where they can be honest — `tinymemory`'s own loader E2E, against a real
//! broker and a real `dlopen`.

use std::sync::Arc;

use tinymemory_api::capabilities::{Capabilities, Capability};
use tinymemory_api::error::MemoryError;
use tinymemory_api::provider::MemoryProvider;

use super::{from_bus, ModuleMemoryProvider, INGEST_BUS_GRACE, MODULE_ID};
use crate::openhuman::config::Config;
use crate::openhuman::modules::registry;

fn provider() -> ModuleMemoryProvider {
    ModuleMemoryProvider::new(Arc::new(Config::default()))
}

/// A bus failure carrying `name`.
fn failure(name: &str) -> tinybus::Error {
    tinybus::Error::MethodFailed {
        name: name.to_string(),
        message: "something went wrong".to_string(),
    }
}

#[test]
fn construction_touches_no_io_and_needs_no_runtime() {
    // The load-bearing property of this type. `CoreContext::memory_binding` is
    // synchronous and roughly 4000 pre-boot tests call it with no tokio runtime,
    // so a constructor that loaded the module — or merely dialled the bus — would
    // panic across the whole suite rather than in one place.
    //
    // This test runs outside `#[tokio::test]` on purpose: that is what makes it a
    // test of the absence of a runtime requirement.
    let provider = provider();
    assert_eq!(provider.driver_id(), MODULE_ID);
}

#[test]
fn the_advertised_capabilities_match_the_pinned_artifact() {
    // Renamed from `..._cover_the_complete_memory_api`, which asserted
    // `capabilities == Capabilities::all()`. That encoded #5598 as the expected
    // behaviour: the host advertised all eighteen families the contract crate
    // declares while the then-pinned v1.0.1 artifact served thirteen, so the other
    // five answered UnknownMethod instead of reporting themselves absent.
    //
    // The part that was always true is still pinned below: the host assembles
    // the memory RPC surface and its tool families from this set before the
    // async bus starts, so a missing mandatory family is a boot-time defect.
    let capabilities = provider().capabilities();

    for mandatory in Capability::MANDATORY {
        assert!(capabilities.contains(mandatory), "{mandatory:?} is missing");
    }
    assert!(capabilities.contains(Capability::Tree));

    // A strict subset of the contract: the artifact is a released binary and the
    // contract is the crate this host compiles against, so the contract may be
    // ahead but can never be behind.
    assert!(
        Capabilities::all().contains_all(capabilities),
        "the artifact advertises a family the contract does not declare",
    );
    // The pinned list now equals the whole contract, and that is the honest
    // statement rather than the #5598 over-claim: the over-claim was
    // advertising a family the HOST could not reach (no accessor), and the
    // Episodic accessor landing closed the last such gap. What still guards
    // drift is `the_capability_list_matches_the_pinned_release` — a re-pin
    // cannot move the version without this list being re-read at the new tag.
    assert_eq!(
        super::capabilities_for(false),
        Capabilities::all(),
        "every family has a host accessor and a bus member in the pinned artifact; \
         an absence here is an under-claim hiding a reachable family",
    );
}

#[test]
fn the_full_capability_override_restores_the_whole_contract() {
    // The escape hatch for a locally-built module, which serves the whole
    // contract. Asserted through `capabilities_for` rather than by setting
    // `OPENHUMAN_MEMORY_MODULE_ASSUME_FULL_CAPABILITIES` — mutating a
    // process-global env var would race every other test in this binary.
    //
    // With the pinned list now covering the full contract, the override is a
    // no-op by construction — asserted as equality rather than difference, so
    // this starts failing (and the override earns its keep again) the moment a
    // future contract family lands that no release serves yet.
    assert_eq!(super::capabilities_for(true), Capabilities::all());
    assert_eq!(
        super::capabilities_for(true),
        super::capabilities_for(false),
        "the pinned artifact serves every contract family, so the override has \
         nothing to widen",
    );
}

#[test]
fn the_registry_record_matches_the_interface_the_module_serves() {
    // A record whose bus name or object path disagrees with the module produces a
    // proxy that resolves to nothing, and the failure surfaces as an unhelpful
    // transport error rather than a mismatch.
    let record = registry::find(MODULE_ID).expect("the memory module is registered");
    assert_eq!(record.bus_name, "ai.tinyhumans.tinymemory.Memory");
    assert_eq!(record.object_path, "/ai/tinyhumans/tinymemory/Memory");
}

#[test]
fn the_memory_record_publishes_one_asset_per_supported_host() {
    // The release exists now, so the question this test used to ask ("are the
    // assets deliberately absent?") is settled. What is worth pinning instead is
    // that the set is complete: a record missing a host silently reports
    // `Unsupported` there rather than failing loudly, so a platform can lose the
    // driver without anything saying so.
    //
    // The digests themselves are checked structurally by `registry`'s own tests
    // (lowercase, 64 hex chars) and semantically by tinybus, which refetches the
    // release manifest and refuses on disagreement. Nothing here can verify they
    // came from the release rather than a local build — that is a review rule,
    // and it is written on the record itself.
    let record = registry::find(MODULE_ID).expect("registered");
    assert_eq!(
        record.assets.len(),
        11,
        "expected one asset per released host, got {:?}",
        record.assets.iter().map(|a| a.host_key).collect::<Vec<_>>()
    );
    for asset in record.assets {
        assert!(
            asset.archive.contains(record.version),
            "{} names version-less or mismatched archive {}",
            asset.host_key,
            asset.archive
        );
    }
}

#[test]
fn a_not_found_survives_the_round_trip_as_not_found() {
    // `get`'s contract makes a missing entry `Ok(None)` and an `Invalid` a real
    // failure, so collapsing the two would be observable to a caller.
    let error = from_bus(&failure(tinymemory_api::wire::NOT_FOUND));
    assert!(matches!(error, MemoryError::NotFound(_)), "{error:?}");
}

#[test]
fn an_invalid_input_is_reported_as_something_the_caller_can_fix() {
    let error = from_bus(&failure(tinymemory_api::wire::INVALID));
    assert!(matches!(error, MemoryError::Invalid(_)), "{error:?}");
}

#[test]
fn a_path_escape_does_not_arrive_as_a_caller_mistake() {
    // The mapping's most security-relevant case: a sandbox escape must not be
    // reclassified as a malformed argument.
    let error = from_bus(&failure(tinymemory_api::wire::PATH_ESCAPE));
    assert!(matches!(error, MemoryError::PathEscape(_)), "{error:?}");
}

#[test]
fn an_unsupported_capability_keeps_its_family_name() {
    let error = from_bus(&failure(tinymemory_api::wire::UNSUPPORTED));
    assert!(
        matches!(error, MemoryError::Unsupported { .. }),
        "{error:?}"
    );
}

#[test]
fn an_unrecognised_wire_name_is_a_backend_failure_not_an_input_error() {
    // A module newer than this build may name an error the table lacks. Telling a
    // caller its input was wrong when it was not sends it into a rewrite loop over
    // something already correct.
    let error = from_bus(&failure("ai.tinyhumans.tinymemory.Error.SomethingNewer"));
    assert!(matches!(error, MemoryError::Other(_)), "{error:?}");
}

#[test]
fn a_missing_module_is_a_backend_failure_the_caller_cannot_fix() {
    let error = from_bus(&failure("ai.tinyhumans.tinybus.Error.ModuleUnavailable"));
    assert!(matches!(error, MemoryError::Other(_)), "{error:?}");
}

#[test]
fn the_debug_form_never_renders_the_config() {
    // `Config` carries credentials and `Debug` output reaches logs.
    let rendered = format!("{:?}", provider());
    assert!(rendered.contains("ModuleMemoryProvider"), "{rendered}");
    assert!(!rendered.contains("Config"), "{rendered}");
}

#[tokio::test]
async fn a_disabled_host_reports_down_rather_than_erroring() {
    // `health` is the one method whose job is to answer "is this reachable", so an
    // unreachable module is a `Down` health rather than a failure. Status output
    // depends on that distinction.
    let mut config = Config::default();
    config.modules.enabled = false;

    let provider = ModuleMemoryProvider::new(Arc::new(config));
    let health = provider.health().await;
    assert!(
        matches!(health, tinymemory_api::health::MemoryHealth::Down { .. }),
        "a disabled module host must report Down, got {health:?}"
    );
}

#[tokio::test]
async fn a_call_against_a_disabled_host_fails_instead_of_hanging() {
    let mut config = Config::default();
    config.modules.enabled = false;

    let provider = ModuleMemoryProvider::new(Arc::new(config));
    let outcome =
        tinymemory_api::provider::mandatory::MemoryCore::get(&provider, "ns", "key").await;
    assert!(outcome.is_err(), "expected an error, got {outcome:?}");
}

#[tokio::test]
async fn shutdown_on_an_unused_driver_is_a_no_op() {
    // A shutdown must not be the thing that downloads and loads a module. Nothing
    // has been used here, so there is nothing to release.
    let mut config = Config::default();
    config.modules.enabled = false;

    let provider = ModuleMemoryProvider::new(Arc::new(config));
    assert!(provider.shutdown().await.is_ok());
}

#[tokio::test]
async fn a_read_against_a_loading_module_reports_unavailable_within_its_grace() {
    use crate::openhuman::modules::resolution::{self, Resolution};
    use tinymemory_api::provider::mandatory::MemoryCore;

    // Plant an in-flight slot for the memory module. The table is process-wide
    // and shared with the rest of the suite, so the slot is settled and
    // removed before this test returns; a parallel caller sees, at worst, a
    // brief "loading" followed by the terminal failure it would have reached
    // anyway with downloads off.
    let table = resolution::table();
    let sender = table.mark_in_flight_for_test(MODULE_ID);
    let provider = ModuleMemoryProvider::new(Arc::new(Config::default()))
        .with_loading_grace(std::time::Duration::from_millis(30));

    // Observe first, assert last. An assertion that fired here would leave
    // MODULE_ID parked in `Loading` for every test that runs after this one in
    // the same process, turning one failure into a cascade of unrelated ones.
    let health = provider.health().await;
    let started = std::time::Instant::now();
    let outcome = MemoryCore::get(&provider, "ns", "key").await;
    let elapsed = started.elapsed();

    table.complete(
        MODULE_ID,
        Resolution::Failed("planted by a test".to_string()),
        sender,
    );
    table.reset_for_test(MODULE_ID);

    // Loading is degraded, never down: `Down` is what rebinds the fallback.
    assert!(
        matches!(
            health,
            tinymemory_api::health::MemoryHealth::Degraded { .. }
        ),
        "a loading module must report Degraded, got {health:?}"
    );
    // A read gives up after its grace with the retryable class, and does so
    // promptly rather than at some caller's deadline.
    assert!(
        matches!(outcome, Err(MemoryError::Unavailable(_))),
        "expected Unavailable while loading, got {outcome:?}"
    );
    assert!(elapsed < std::time::Duration::from_secs(5));
}

#[test]
fn writes_wait_for_the_module_and_reads_do_not() {
    let provider = provider();
    // Every family that mutates, one representative each. A bounded wait here
    // does not delay the call, it discards it: none of these has a retry above
    // it, so the archivist's turn, the autosaved message or the operator's
    // sync would be lost rather than late.
    for write in [
        "store",
        "forget",
        "import_records",
        "ingest_document",
        "ingest_chat",
        "ingest_email",
        "ingest_coding_sessions",
        "typed_ingest_event",
        "insert_turn",
        "insert_event",
        "open_segment",
        "set_goals",
        "append",
        "put_document",
        "put_relation",
        "put_tool_rule",
        "upsert_facet",
        "delete_facet",
        "forget_source",
        "purge_all",
        "compact",
        "consolidate",
        "reembed",
        "retry_failed",
        "flush_pending",
        "cascade",
        "seal",
        "summarise",
        "run_source_sync",
        "run_connection_sync",
        "bootstrap_connection",
        "override_scheduler_gate",
        "shutdown",
    ] {
        assert_eq!(
            provider.loading_grace(write),
            None,
            "{write} mutates, so it must wait the module out"
        );
    }
    for read in [
        "get",
        "recall",
        "list",
        "health",
        "namespaces",
        "export_page",
    ] {
        assert!(
            provider.loading_grace(read).is_some(),
            "{read} must be bounded"
        );
    }
    // The default for anything nobody classified is to wait, so a member added
    // upstream tomorrow cannot start silently dropping writes.
    assert_eq!(
        provider.loading_grace("a_member_from_a_future_contract"),
        None,
        "an unrecognised operation must wait rather than be treated as a read"
    );
}
#[test]
fn the_capability_list_matches_the_pinned_release() {
    // ARTIFACT_CAPABILITIES describes what ONE specific release of the module
    // serves. Re-pinning the registry to a newer release without re-reading that
    // list would silently re-introduce #5598 in the other direction — the host
    // would under-claim and hide families the new artifact does have.
    //
    // Tying the two together here means the pin bump is a red test, not a
    // silent drift.
    let record = crate::openhuman::modules::registry::find(super::MODULE_ID)
        .expect("the tinymemory module must be in the registry");
    assert_eq!(
        record.version,
        super::ARTIFACT_CAPABILITIES_PIN,
        "the registry pin moved to {} but ARTIFACT_CAPABILITIES is still the list read from {}. \
         Re-read Capability::ALL at the new tag, update both, and re-run.",
        record.version,
        super::ARTIFACT_CAPABILITIES_PIN,
    );
}

#[test]
fn the_advertised_set_does_not_over_claim_the_artifact() {
    // The regression guard for #5598 proper: the driver must not advertise a
    // family the pinned artifact cannot serve. Capabilities::all() is what the
    // CONTRACT declares; the artifact is older and smaller.
    use tinymemory_api::capabilities::{Capabilities, Capability};

    // `capabilities_for(false)` rather than `artifact_capabilities()`: the
    // invariant is a property of the pinned list, and reading the environment
    // here would make this test fail for anyone running with the documented
    // `OPENHUMAN_MEMORY_MODULE_ASSUME_FULL_CAPABILITIES=1` override.
    let advertised = super::capabilities_for(false);

    // Four of the five families v1.0.1 lacked arrived in the v1.2.0 artifact,
    // so the under-claim they used to represent is over — assert they ARE
    // advertised, or a future re-pin that silently narrows the list goes
    // unnoticed.
    for capability in [
        Capability::People,
        Capability::Chunks,
        Capability::Retrieval,
        Capability::Profile,
    ] {
        assert!(
            advertised.contains(capability),
            "{capability:?} has a bus member in the pinned {} artifact but is not advertised — \
             the host is under-claiming and hiding a family it can reach",
            super::ARTIFACT_CAPABILITIES_PIN,
        );
    }

    // `Episodic` joins the loop's spirit in the same change that implemented
    // `as_episodic`, exactly as the previous version of this comment required:
    // the artifact has served the members since v1.2.0, and the archivist now
    // writes through them, so hiding the family would be the under-claim.
    assert!(
        advertised.contains(Capability::Episodic),
        "Episodic has a host accessor and bus members in the pinned {} artifact — \
         hiding it strands the archivist's writes",
        super::ARTIFACT_CAPABILITIES_PIN,
    );

    // With every family reachable, full advertisement IS the honest set. The
    // anti-over-claim tripwire this used to be lives on in the accessor rule
    // itself: `capabilities_for` can only name families `ModuleMemoryProvider`
    // implements, and the pin-drift test re-opens the question on every
    // registry bump.
    assert_eq!(advertised, Capabilities::all());
}

/// The CI workflows download the TinyMemory module and verify it against a
/// digest written inline in the YAML. That digest is a second copy of the one
/// in [`super::super::registry`], and the two drifted: a version bump moved the
/// archive name and the release tag but left the checksum two releases behind,
/// so every lane that installs the module died on
/// `sha256sum: WARNING: 1 computed checksum did NOT match`.
///
/// The failure was loud, which is the system working — a mismatched digest is
/// exactly what should stop a build rather than silently running the wrong
/// artifact. What was missing is anything that catches the drift *before* CI
/// downloads a file, and a comment asking the next person to keep three places
/// in step is not that. This is.
///
/// Scoped to the one row the workflows actually install (`ubuntu-22.04-x86_64`,
/// the CI runner's triple) rather than all eleven, because that is the only
/// pair that can disagree.
#[test]
fn the_ci_workflows_pin_the_same_module_digest_as_the_registry() {
    const HOST_KEY: &str = "ubuntu-22.04-x86_64";

    let record = registry::find(MODULE_ID).expect("the memory module is registered");
    let asset = record
        .assets
        .iter()
        .find(|asset| asset.host_key == HOST_KEY)
        .unwrap_or_else(|| panic!("the registry has no {HOST_KEY} asset to compare against"));

    let workflows = [
        "../.github/workflows/ci-full.yml",
        "../.github/workflows/ci-lite.yml",
        "../.github/workflows/e2e-reusable.yml",
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    let mut checked = 0usize;
    for relative in workflows {
        let path = root.join(relative.trim_start_matches("../"));
        let Ok(yaml) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in yaml.lines() {
            let Some(rest) = line.trim().strip_prefix("memory_sha256=") else {
                continue;
            };
            let pinned = rest.trim().trim_matches('"');
            assert_eq!(
                pinned,
                asset.sha256,
                "{} pins a digest the registry does not: the workflow will download \
                 {} and refuse it. Copy both `memory_version` and `memory_sha256` \
                 from the {HOST_KEY} row of registry.rs.",
                path.display(),
                asset.archive,
            );
            checked += 1;
        }
    }

    assert!(
        checked >= 4,
        "expected at least four workflow digest sites, found {checked} — either a \
         lane stopped installing the module, or the assignment was renamed and this \
         guard silently stopped checking anything"
    );
}

/// The bus deadline on `IngestCodingSessions` must never be the one that fires.
///
/// This is the defect from #5802 in test form. Before the fix the module call
/// took tinybus' flat 30 s `DEFAULT_TIMEOUT` while the RPC around it allowed
/// 120 s + 90 s per session, so a perfectly healthy 35 s import was abandoned
/// by the caller, reported as a failure, and finished successfully five
/// seconds later with nobody listening.
///
/// Asserted across the whole input range rather than at one point, because the
/// budget is piecewise: it scales linearly and then clamps at `HARD_CAP_SECS`.
/// A future edit that raises the cap, the base, or the per-session allowance
/// without touching the grace fails here instead of in the field.
#[test]
fn the_ingest_bus_deadline_always_outlasts_the_rpc_budget() {
    use crate::openhuman::memory::sources::rpc::ingest_budget;

    // Below the cap, at the cap, and far past it (`max_sessions` is untrusted
    // input from an advertised RPC, so `usize::MAX` is a reachable argument).
    for max_sessions in [0, 1, 5, 6, 7, 100, 10_000, usize::MAX] {
        let budget = ingest_budget(max_sessions);
        let bus = budget + INGEST_BUS_GRACE;
        assert!(
            bus > budget,
            "max_sessions={max_sessions}: the bus deadline ({bus:?}) must outlast the \
             RPC budget ({budget:?}), or the wire member's error wins and the caller \
             is released while the module is still working"
        );
    }
}

/// The grace has to be big enough to order two timers, not merely non-zero.
///
/// A one-millisecond grace would satisfy the assertion above and still lose the
/// race under ordinary scheduling jitter, which would put the failure back
/// where it started: a bus-member error surfacing instead of the RPC's
/// structured one.
#[test]
fn the_ingest_bus_grace_is_wide_enough_to_order_the_two_timers() {
    assert!(
        INGEST_BUS_GRACE >= std::time::Duration::from_secs(5),
        "INGEST_BUS_GRACE is {INGEST_BUS_GRACE:?}; a grace this small does not \
         reliably order the RPC's timeout ahead of the bus deadline"
    );
}

/// Every `MemorySourceSync` and `MemoryMaintenance` member the trait defaults
/// must be bridged to the module rather than left to inherit the default.
///
/// Three of these members carry a default body that returns
/// `Unsupported(SourceSync)`, and `diagnose` one that returns
/// `Unsupported(Maintenance)`. A defaulted member cannot break an implementor at
/// compile time, so when they were added to the contract `ModuleMemoryProvider`
/// kept compiling and silently began refusing — which is #5801: the manual
/// "Sync now" button answered `unsupported capability: source_sync` while the
/// module's own scheduler, which never crosses this bridge, kept syncing fine.
///
/// The discriminator needs no module. With the host disabled, `proxy()` fails
/// with `MemoryError::Other`, so a member that really dispatches through
/// `module_call!` reports `Other` while one that fell through to the default
/// reports `Unsupported`. Asserting "not Unsupported" therefore asserts the
/// dispatch itself, which is the part that was missing.
#[tokio::test]
async fn the_defaulted_members_dispatch_to_the_module_instead_of_refusing() {
    use tinymemory_api::provider::{MemoryMaintenance, MemorySourceSync};

    let mut config = Config::default();
    config.modules.enabled = false;
    let provider = ModuleMemoryProvider::new(Arc::new(config));

    let refused = |label: &str, error: MemoryError| {
        assert!(
            !matches!(error, MemoryError::Unsupported { .. }),
            "{label} answered from the trait default instead of dispatching to the \
             module — the `module_call!` arm is missing, so every caller gets \
             `unsupported capability` however capable the artifact is. Got {error:?}"
        );
    };

    refused(
        "run_source_sync",
        provider
            .run_source_sync("src_whatever")
            .await
            .expect_err("a disabled host cannot succeed"),
    );
    refused(
        "bootstrap_connection",
        provider
            .bootstrap_connection("gmail", "ca_whatever")
            .await
            .expect_err("a disabled host cannot succeed"),
    );
    refused(
        "is_toolkit_syncable",
        provider
            .is_toolkit_syncable("gmail")
            .await
            .expect_err("a disabled host cannot succeed"),
    );
    refused(
        "diagnose",
        MemoryMaintenance::diagnose(&provider)
            .await
            .expect_err("a disabled host cannot succeed"),
    );

    // The five doors tinymemory added for the openhuman engine shed. Each is
    // defaulted upstream, so a missing `module_call!` arm here is invisible at
    // compile time and turns a working feature into a runtime
    // `unsupported capability`.
    use tinymemory_api::provider::{MemoryChunks, MemoryTree, SummaryContext};
    refused(
        "summarise",
        MemoryTree::summarise(
            &provider,
            &[],
            &SummaryContext {
                tree_id: "t".into(),
                tree_kind: "source".into(),
                target_level: 0,
                token_budget: 1,
                input_token_budget: 1,
                overhead_reserve_tokens: 0,
                ask: None,
            },
        )
        .await
        .expect_err("a disabled host cannot succeed"),
    );
    refused(
        "root_summaries_with_caps",
        MemoryTree::root_summaries_with_caps(&provider, 1, 1)
            .await
            .expect_err("a disabled host cannot succeed"),
    );
    refused(
        "chunk_score",
        MemoryChunks::chunk_score(&provider, "chunk_whatever")
            .await
            .expect_err("a disabled host cannot succeed"),
    );
    refused(
        "source_ingest_status",
        MemoryChunks::source_ingest_status(&provider, &[])
            .await
            .expect_err("a disabled host cannot succeed"),
    );
    refused(
        "degraded_state",
        MemoryMaintenance::degraded_state(&provider)
            .await
            .expect_err("a disabled host cannot succeed"),
    );

    // The seven the runtime-tree round added (contract 4.0). Six carry the
    // `tree_summarizer_*` RPC surface and the `tree-summarizer` CLI; the
    // seventh is what `memory_flavour` reads. All seven are defaulted upstream,
    // so a missing `module_call!` arm is invisible to the compiler and turns
    // "Build Summary Trees" into `unsupported capability` against an artifact
    // that serves it.
    let at = chrono::Utc::now();
    refused(
        "runtime_buffer_write",
        MemoryTree::runtime_buffer_write(&provider, "ns", "content", at, None)
            .await
            .expect_err("a disabled host cannot succeed"),
    );
    refused(
        "runtime_read_node",
        MemoryTree::runtime_read_node(&provider, "ns", "root")
            .await
            .expect_err("a disabled host cannot succeed"),
    );
    refused(
        "runtime_read_children",
        MemoryTree::runtime_read_children(&provider, "ns", "root")
            .await
            .expect_err("a disabled host cannot succeed"),
    );
    refused(
        "runtime_tree_status",
        MemoryTree::runtime_tree_status(&provider, "ns")
            .await
            .expect_err("a disabled host cannot succeed"),
    );
    refused(
        "runtime_summarize",
        MemoryTree::runtime_summarize(&provider, "ns", at)
            .await
            .expect_err("a disabled host cannot succeed"),
    );
    refused(
        "runtime_rebuild",
        MemoryTree::runtime_rebuild(&provider, "ns")
            .await
            .expect_err("a disabled host cannot succeed"),
    );
    refused(
        "flavour_profile",
        MemoryTree::flavour_profile(&provider, "persona/communication")
            .await
            .expect_err("a disabled host cannot succeed"),
    );
}

#[path = "memory_tests_part_01_tests.rs"]
mod part_01_tests;
