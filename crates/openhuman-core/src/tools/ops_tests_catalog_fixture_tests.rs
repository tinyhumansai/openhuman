//! Drift guard between the core's registered tool catalog and the frontend's
//! own copy of that name list (issue: tool-call presentation).
//!
//! `app/src/features/conversations/tools/` renders a fallback label/icon for
//! any tool name it recognizes even before the server-computed
//! `display_label`/`display_detail` arrive (e.g. on a cold reconnect that
//! replays a persisted timeline). That fallback table is only ever as
//! accurate as the day someone last updated it by hand, so this test builds
//! the REAL registered catalog on every core test run and fails loudly the
//! moment the shipped product disagrees with the frontend's copy. Contributor
//! builds may omit product-only gates, so they check only for unrecognized
//! registered names rather than treating gated names as removals.
use super::*;
use std::path::PathBuf;

/// Path to the frontend's copy of the tool-name list, relative to this
/// crate's manifest directory (`crates/openhuman-core`).
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../app/src/features/conversations/tools/__fixtures__/coreToolNames.json")
}

/// Track the full shipped feature contract, rather than a hand-picked subset
/// that can accidentally classify a partial build as the product profile.
fn full_product_features_enabled() -> bool {
    let flags = [
        ("channels", cfg!(feature = "channels")),
        ("media", cfg!(feature = "media")),
        ("inference", cfg!(feature = "inference")),
        ("voice", cfg!(feature = "voice")),
        ("web3", cfg!(feature = "web3")),
        ("documents", cfg!(feature = "documents")),
        ("modules", cfg!(feature = "modules")),
        ("flows", cfg!(feature = "flows")),
        ("skills", cfg!(feature = "skills")),
        ("mcp", cfg!(feature = "mcp")),
        ("crash-reporting", cfg!(feature = "crash-reporting")),
        ("http-server", cfg!(feature = "http-server")),
        ("scheduler-gate", cfg!(feature = "scheduler-gate")),
        ("file-logging", cfg!(feature = "file-logging")),
        ("hosting", cfg!(feature = "hosting")),
        ("tinymemes", cfg!(feature = "tinymemes")),
        ("storage-sqlite", cfg!(feature = "storage-sqlite")),
    ];
    let declared: std::collections::BTreeSet<_> = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../scripts/ci/product-features.txt"
    ))
    .lines()
    .map(|line| line.split('#').next().unwrap_or("").trim())
    .filter(|line| !line.is_empty())
    .collect();
    let checked: std::collections::BTreeSet<_> = flags.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        checked, declared,
        "update the product catalog feature gate list"
    );
    flags.iter().all(|(_, enabled)| *enabled)
}

/// The full model-facing tool catalog this build can register, sorted and
/// deduplicated.
///
/// Built from [`all_tools`] (which thinly wraps [`all_tools_with_runtime`]
/// with the native runtime adapter) under a config that widens every toggle
/// this test controls — the browser tool enabled, in addition to whatever
/// `Config::default()` already turns on — so the registered set is as close
/// to maximal as a config alone can make it. What this canNOT widen:
///
/// * **Composio per-connection action tools** (`ComposioActionTool`, dynamic
///   slugs like `GMAIL_SEND_EMAIL`) are never part of this static list.
///   `all_composio_agent_tools` only ever registers its five fixed dispatcher
///   tools (`composio_list_toolkits`, `composio_list_connections`,
///   `composio_authorize`, `composio_connect`, `composio_list_tools`,
///   `composio_execute`) and gates even those on a signed-in session, which
///   this test's config does not have — so this build contributes none of
///   them, static or dynamic, and the fixture should never carry a
///   `COMPOSIO_*`/upper-snake action slug.
/// * **Search** registers one tool per capability role (`web_search_tool`,
///   `web_answer_tool`, `web_contents_tool`). A role tool exists only when a
///   provider can serve it, so this catalog configures BYOK keys for Brave,
///   Gemini and Tavily to register all three without a signed-in session.
///   Provider-specific tools (`exa_search`, `brave_news_search`, ...) appear
///   only in the advanced `all_tools` presentation and are not listed.
/// * **Cargo feature gates**: the fixture represents the shipped product
///   feature set (`scripts/ci/product-features.txt`). A default contributor
///   build may register fewer tools, but every name it registers must be in
///   the product catalog. The full product build checks exact equality.
///
/// On top of the domain registry this adds the harness-intrinsic bridge tool
/// name, `tool_search` (`tinyagents_harness::tool::discover::TOOL_SEARCH_NAME`):
/// it is never a registered [`tinytools::Tool`] — the agent loop answers it
/// itself once a turn has deferred tools (see that module's doc comment) —
/// but it is a model-visible tool name the frontend's tool-call presentation
/// must recognize exactly like any other.
fn full_tool_catalog_names() -> Vec<String> {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mut cfg = test_config(&tmp);
    cfg.browser.enabled = true;
    for (provider, key) in [("brave", "b"), ("gemini", "g"), ("tavily", "t")] {
        cfg.search.providers.insert(
            provider.into(),
            crate::config::SearchProviderSettings::direct(),
        );
        cfg.search.credentials_mut(provider).unwrap().api_key = Some(key.into());
    }
    // Memory registers its `memory` tool only while an engine is usable (a
    // signed-in user, or a CortexDB key). Bind the reference engine so the
    // catalog is the one a signed-in user sees.
    crate::memory::test_fixtures::bind_reference(&cfg);
    let browser = cfg.browser.clone();
    let http = cfg.http_request.clone();

    let tools = all_tools(
        Arc::new(cfg.clone()),
        &security,
        AuditLogger::disabled(),
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );

    let mut names: Vec<String> = tools.iter().map(|t| t.name().to_string()).collect();
    names.push(tinyagents_harness::tool::discover::TOOL_SEARCH_NAME.to_string());
    names.sort();
    names.dedup();
    // Defensive: a Composio per-connection action tool would be an
    // upper-snake slug (e.g. `GMAIL_SEND_EMAIL`) and must never reach this
    // static fixture — see the doc comment above for why none should be
    // registered here in the first place.
    for name in &names {
        assert!(
            !(name.chars().any(|c| c.is_ascii_uppercase()) && name.contains('_')),
            "catalog contains what looks like a dynamic Composio action slug \
             ({name}); those must be excluded from the static fixture"
        );
    }
    names
}

const REGENERATE_COMMAND: &str = "UPDATE_TOOL_CATALOG=1 cargo test -p openhuman --lib \
     --features \"$(bash scripts/ci/product-features.sh)\" \
     tools::ops::tests::catalog_fixture_tests::tool_catalog_matches_frontend_fixture";

/// Regenerates the fixture when `UPDATE_TOOL_CATALOG=1`, otherwise fails with
/// exactly what was added/removed relative to it.
#[test]
fn tool_catalog_matches_frontend_fixture() {
    let names = full_tool_catalog_names();
    let path = fixture_path();

    if std::env::var("UPDATE_TOOL_CATALOG").as_deref() == Ok("1") {
        assert!(full_product_features_enabled(), "{REGENERATE_COMMAND}");
        let json = serde_json::to_string_pretty(&names).expect("serialize tool catalog");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create fixture directory");
        }
        std::fs::write(&path, format!("{json}\n")).expect("write tool catalog fixture");
        eprintln!(
            "[tool-catalog] rewrote {} with {} names",
            path.display(),
            names.len()
        );
        return;
    }

    let existing = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "missing tool-catalog fixture at {}: {e}\nGenerate it with:\n  {REGENERATE_COMMAND}",
            path.display()
        )
    });
    let mut expected: Vec<String> = serde_json::from_str(&existing).unwrap_or_else(|e| {
        panic!(
            "fixture at {} is not a JSON array of strings: {e}",
            path.display()
        )
    });
    expected.sort();
    expected.dedup();

    // Contributor and partial-feature builds may lack product tools, but
    // every registered name must still belong to the shipped catalog.
    let full_product = full_product_features_enabled();
    let added: Vec<&String> = names.iter().filter(|n| !expected.contains(n)).collect();
    let removed: Vec<&String> = if full_product {
        expected.iter().filter(|n| !names.contains(n)).collect()
    } else {
        Vec::new()
    };
    if !added.is_empty() || !removed.is_empty() {
        panic!(
            "core tool catalog drifted from the frontend fixture at {} (product features compiled: {full_product}).\n\
             added:   {added:?}\n\
             removed: {removed:?}\n\n\
             Regenerate with:\n  {REGENERATE_COMMAND}",
            path.display()
        );
    }
}
