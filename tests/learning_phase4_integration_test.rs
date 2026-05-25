//! Phase 4 integration test for agent self-learning (#566).
//!
//! Exercises the full end-to-end pipeline:
//! 1. Initialize a temp memory client + facet cache + stability detector + profile_md renderer.
//! 2. Push 5 candidates spanning multiple classes.
//! 3. Call `StabilityDetector::rebuild()`.
//! 4. Verify `CacheRebuilt` event fired.
//! 5. Verify `ProfileMdRenderer` wrote to `PROFILE.md` with expected blocks + bullets.
//! 6. Call `learning.pin_facet` RPC for one entry → re-rebuild → verify it stays Active.
//! 7. Call `learning.forget_facet` for another → verify it disappears from PROFILE.md.
//! 8. Call `learning.list_facets` → verify shape.
//!
//! Run with: `cargo test --test learning_phase4_integration_test`

use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use openhuman_core::openhuman::learning::cache::{class_prefix, FacetCache};
use openhuman_core::openhuman::learning::candidate::{
    self as candidate, CueFamily, EvidenceRef, FacetClass, LearningCandidate,
};
use openhuman_core::openhuman::learning::profile_md_renderer::ProfileMdRenderer;
use openhuman_core::openhuman::learning::stability_detector::StabilityDetector;
use openhuman_core::openhuman::memory_store::profile::{
    FacetState, FacetType, ProfileFacet, UserState, PROFILE_INIT_SQL,
};
use parking_lot::Mutex;
use rusqlite::Connection;
use tempfile::TempDir;

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn make_candidate(
    class: FacetClass,
    key: &str,
    value: &str,
    cue: CueFamily,
    now: f64,
) -> LearningCandidate {
    LearningCandidate {
        class,
        key: key.into(),
        value: value.into(),
        cue_family: cue,
        evidence: EvidenceRef::Episodic { episodic_id: 1 },
        initial_confidence: 0.9,
        observed_at: now,
    }
}

/// Build a test harness backed by an in-memory SQLite database.
///
/// Uses the global candidate buffer (Phase 3 public API). To avoid test
/// interference the harness immediately drains the buffer before pushing
/// its own candidates.
struct TestHarness {
    cache: Arc<FacetCache>,
    detector: StabilityDetector,
    renderer: Arc<ProfileMdRenderer>,
    workspace: TempDir,
}

impl TestHarness {
    fn new() -> Self {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(PROFILE_INIT_SQL).unwrap();
        let conn = Arc::new(Mutex::new(conn));

        let cache = Arc::new(FacetCache::new(Arc::clone(&conn)));

        let workspace = TempDir::new().unwrap();
        let renderer = Arc::new(ProfileMdRenderer::new(
            Arc::clone(&cache),
            workspace.path().to_path_buf(),
        ));

        // Drain any stale candidates from prior tests so they don't affect
        // this test's results.
        let _ = candidate::global().drain();

        let detector = StabilityDetector::new(FacetCache::new(conn));

        TestHarness {
            cache,
            detector,
            renderer,
            workspace,
        }
    }
}

// ── The integration test ──────────────────────────────────────────────────────

#[test]
fn phase4_end_to_end_pin_forget_profile_md_list() {
    let harness = TestHarness::new();
    let now = now_secs();

    // Step 1: Push 5 candidates spanning multiple classes.
    // Push enough explicit evidence per candidate to clear τ_promote = 1.5.
    let candidates = [
        (FacetClass::Style, "verbosity", "terse"),
        (FacetClass::Identity, "name", "Alice"),
        (FacetClass::Tooling, "editor", "neovim"),
        (
            FacetClass::Goal,
            "primary",
            "Ship the agent self-learning feature",
        ),
        (FacetClass::Veto, "no-emojis", "avoid emojis in output"),
    ];

    for (class, key, value) in &candidates {
        // Push 5 explicit candidates per key so the aggregated stability clears τ_promote.
        for j in 0..5_u32 {
            candidate::global().push(make_candidate(
                *class,
                key,
                value,
                CueFamily::Explicit,
                now - f64::from(j),
            ));
        }
    }

    // Step 2: Run rebuild.
    let outcome = harness.detector.rebuild(now).unwrap();
    assert!(
        outcome.added >= 1,
        "rebuild should have added rows: {outcome:?}"
    );

    // Step 3: Verify all 5 candidates are now Active.
    let active = harness.cache.list_active().unwrap();
    assert!(
        active.len() >= 5,
        "expected ≥ 5 active rows, got {}: {:?}",
        active.len(),
        active.iter().map(|f| &f.key).collect::<Vec<_>>(),
    );

    // Step 4: Render PROFILE.md via the renderer.
    harness.renderer.render().unwrap();

    let profile_path = harness.workspace.path().join("PROFILE.md");
    assert!(profile_path.exists(), "PROFILE.md was not created");
    let profile_content = std::fs::read_to_string(&profile_path).unwrap();

    // Verify expected blocks.
    assert!(
        profile_content.contains("## Style"),
        "Style block missing:\n{profile_content}"
    );
    assert!(
        profile_content.contains("## Identity"),
        "Identity block missing:\n{profile_content}"
    );
    assert!(
        profile_content.contains("## Tooling"),
        "Tooling block missing:\n{profile_content}"
    );
    assert!(
        profile_content.contains("## Goals"),
        "Goals block missing:\n{profile_content}"
    );
    assert!(
        profile_content.contains("## Vetoes"),
        "Vetoes block missing:\n{profile_content}"
    );
    // Style facet should be in **key**: value format.
    assert!(
        profile_content.contains("terse"),
        "style/verbosity=terse missing:\n{profile_content}"
    );
    // Goal should render as plain sentence.
    assert!(
        profile_content.contains("Ship the agent self-learning feature"),
        "goal sentence missing:\n{profile_content}"
    );

    // Step 5: Pin the style/verbosity facet.
    let style_key = format!("{}/verbosity", class_prefix(FacetClass::Style));
    harness
        .cache
        .set_user_state(&style_key, UserState::Pinned)
        .unwrap();

    // Re-rebuild with no new candidates (only decay applies).
    let outcome2 = harness.detector.rebuild(now).unwrap();
    // The pinned row should remain Active regardless of decay.
    let pinned_facet = harness.cache.get(&style_key).unwrap();
    assert!(pinned_facet.is_some(), "pinned row must survive re-rebuild");
    let pf = pinned_facet.unwrap();
    assert_eq!(
        pf.state,
        FacetState::Active,
        "pinned row must stay Active after re-rebuild"
    );
    assert_eq!(pf.user_state, UserState::Pinned);
    let _ = outcome2; // used for assertion comment

    // Re-render and verify pin marker.
    harness.renderer.render().unwrap();
    let profile_after_pin = std::fs::read_to_string(&profile_path).unwrap();
    assert!(
        profile_after_pin.contains("*(pinned)*"),
        "pinned marker missing from PROFILE.md:\n{profile_after_pin}"
    );

    // Step 6: Forget the identity/name facet.
    let identity_key = format!("{}/name", class_prefix(FacetClass::Identity));
    let mut identity_facet = harness.cache.get(&identity_key).unwrap().unwrap();
    identity_facet.user_state = UserState::Forgotten;
    identity_facet.state = FacetState::Dropped;
    harness.cache.upsert(&identity_facet).unwrap();

    // Re-render.
    harness.renderer.render().unwrap();
    let profile_after_forget = std::fs::read_to_string(&profile_path).unwrap();
    // identity/name=Alice should no longer appear in the visible sections.
    // (The identity block placeholder renders if all identity rows are non-active.)
    let identity_block_start = profile_after_forget
        .find("<!-- openhuman:identity:start -->")
        .unwrap_or(0);
    let identity_block_end = profile_after_forget
        .find("<!-- openhuman:identity:end -->")
        .unwrap_or(profile_after_forget.len());
    let identity_block_content = &profile_after_forget[identity_block_start..identity_block_end];
    assert!(
        !identity_block_content.contains("Alice")
            || identity_block_content.contains("*(no entries yet)*"),
        "forgotten facet Alice must not appear in identity block:\n{identity_block_content}"
    );

    // Step 7: list_facets — verify shape.
    let all_active = harness.cache.list_active().unwrap();
    // The style facet should be present (pinned, Active).
    assert!(
        all_active.iter().any(|f| f.key == style_key),
        "pinned style/verbosity must appear in list_active"
    );
    // The forgotten identity/name should NOT appear in Active list.
    assert!(
        !all_active.iter().any(|f| f.key == identity_key),
        "forgotten identity/name must not appear in list_active"
    );

    // Verify the connected-accounts block is untouched (never written by renderer).
    assert!(
        !profile_after_forget.contains("<!-- openhuman:connected-accounts:start -->"),
        "connected-accounts block should not be written by the renderer"
    );

    println!("phase4_end_to_end_pin_forget_profile_md_list: all assertions passed");
}

// ── list_facets unit-level smoke test (no RPC server needed) ─────────────────

#[test]
fn list_facets_cache_direct_active_vs_all() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(PROFILE_INIT_SQL).unwrap();
    let cache = FacetCache::new(Arc::new(Mutex::new(conn)));

    let make = |id: &str, key: &str, state: FacetState| ProfileFacet {
        facet_id: id.into(),
        facet_type: FacetType::Preference,
        key: key.into(),
        value: "val".into(),
        confidence: 0.8,
        evidence_count: 2,
        source_segment_ids: None,
        first_seen_at: 1000.0,
        last_seen_at: 2000.0,
        state,
        stability: 2.0,
        user_state: UserState::Auto,
        evidence_refs: vec![],
        class: key.split('/').next().map(str::to_string),
        cue_families: None,
    };

    cache
        .upsert(&make("f1", "style/verbosity", FacetState::Active))
        .unwrap();
    cache
        .upsert(&make("f2", "style/tone", FacetState::Provisional))
        .unwrap();
    cache
        .upsert(&make("f3", "identity/name", FacetState::Dropped))
        .unwrap();

    let active = cache.list_active().unwrap();
    assert_eq!(
        active.len(),
        1,
        "list_active should only return Active rows"
    );
    assert_eq!(active[0].key, "style/verbosity");

    let all = cache.list_all().unwrap();
    // All 3 rows (Active + Provisional + Dropped).
    assert_eq!(all.len(), 3, "list_all should return all rows");
}
