use super::*;
use crate::modules::tinybox::host_runtime::NativeRuntime;
use crate::tools::ops::all_tools_with_runtime;
use tempfile::TempDir;
use tinytools::ToolResult;

struct FakeTool(&'static str);

#[async_trait::async_trait]
impl Tool for FakeTool {
    fn name(&self) -> &str {
        self.0
    }

    fn description(&self) -> &str {
        "fake tool from a registered source"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
}

fn assemble(tmp: &TempDir) -> Vec<String> {
    let cfg = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig {
        enabled: false,
        ..BrowserConfig::default()
    };
    let http = HttpRequestConfig::default();
    all_tools_with_runtime(
        Arc::new(cfg.clone()),
        &security,
        Arc::new(NativeRuntime::new()),
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
        None,
    )
    .iter()
    .map(|t| t.name().to_string())
    .collect()
}

#[test]
fn nothing_registered_leaves_the_catalogue_without_source_tools() {
    let tmp = TempDir::new().unwrap();
    let names = assemble(&tmp);
    assert!(!names.is_empty());
    assert!(!names.iter().any(|n| n.starts_with("source_fake_")));
}

#[test]
fn registered_sources_append_in_registration_order_after_built_ins() {
    let tmp = TempDir::new().unwrap();
    let baseline = assemble(&tmp);

    register_tool_source("test::second", |_| {
        vec![Box::new(FakeTool("source_fake_b"))]
    });
    register_tool_source("test::first", |_| {
        vec![
            Box::new(FakeTool("source_fake_a1")),
            Box::new(FakeTool("source_fake_a2")),
        ]
    });
    let names = assemble(&tmp);
    unregister_tool_source("test::second");
    unregister_tool_source("test::first");

    let position = |name: &str| names.iter().position(|n| n == name).unwrap();
    assert!(position("source_fake_b") < position("source_fake_a1"));
    assert_eq!(position("source_fake_a2"), position("source_fake_a1") + 1);
    // Built-ins precede registered sources.
    assert!(position("file_read") < position("source_fake_b"));
    assert_eq!(names.len(), baseline.len() + 3);
}

#[test]
fn a_source_receives_the_catalogue_context() {
    let tmp = TempDir::new().unwrap();
    let seen = Arc::new(std::sync::Mutex::new(None));
    let sink = Arc::clone(&seen);
    register_tool_source("test::ctx", move |ctx| {
        *sink.lock().unwrap() = Some((ctx.action_dir.to_path_buf(), ctx.agents.len()));
        Vec::new()
    });
    let _ = assemble(&tmp);
    unregister_tool_source("test::ctx");
    let (action_dir, agents) = seen.lock().unwrap().clone().expect("source ran");
    assert_eq!(action_dir, tmp.path());
    assert_eq!(agents, 0);
}

#[test]
fn unregistering_a_source_removes_its_tools() {
    let tmp = TempDir::new().unwrap();
    register_tool_source("test::gone", |_| {
        vec![Box::new(FakeTool("source_fake_gone"))]
    });
    assert!(assemble(&tmp).contains(&"source_fake_gone".to_string()));
    assert!(unregister_tool_source("test::gone"));
    assert!(!assemble(&tmp).contains(&"source_fake_gone".to_string()));
}

#[test]
fn anchored_sources_sit_where_the_catalogue_always_listed_them() {
    // The pre-refactor catalogue built a host's media / file-storage tools
    // right after the search tools (before hosting, web3 and image_info) and
    // its place / market-data tools right after the delegation tool, ahead of
    // the Composio tools; the order is a prompt-cache input.
    let tmp = TempDir::new().unwrap();
    let anchor_tools = |names: &[&str]| -> Vec<Box<dyn Tool>> {
        names
            .iter()
            .map(|n| Box::new(FakeTool(Box::leak(n.to_string().into_boxed_str()))) as Box<dyn Tool>)
            .collect()
    };

    register_tool_source_at(
        "test::after_search_a",
        ToolSourcePlacement::After(ANCHOR_AFTER_SEARCH),
        move |_| anchor_tools(&["anchor_fake_search_a"]),
    );
    register_tool_source_at(
        "test::after_search_b",
        ToolSourcePlacement::After(ANCHOR_AFTER_SEARCH),
        move |_| anchor_tools(&["anchor_fake_search_b"]),
    );
    register_tool_source_at(
        "test::after_delegate",
        ToolSourcePlacement::After(ANCHOR_AFTER_DELEGATE),
        move |_| anchor_tools(&["anchor_fake_delegate"]),
    );
    register_tool_source("test::end", |_| vec![Box::new(FakeTool("anchor_fake_end"))]);
    register_tool_source_at(
        "test::unknown_anchor",
        ToolSourcePlacement::After("nowhere"),
        |_| vec![Box::new(FakeTool("anchor_fake_unknown"))],
    );
    let names = assemble(&tmp);
    for id in [
        "test::after_search_a",
        "test::after_search_b",
        "test::after_delegate",
        "test::end",
        "test::unknown_anchor",
    ] {
        unregister_tool_source(id);
    }

    let pos = |name: &str| {
        names
            .iter()
            .position(|n| n == name)
            .unwrap_or_else(|| panic!("{name} missing"))
    };
    // Registration order within an anchor.
    assert_eq!(pos("anchor_fake_search_b"), pos("anchor_fake_search_a") + 1);
    // After-search sources precede the image-info built-in registered after
    // hosting/web3, and follow the built-ins that precede the search tools.
    assert!(pos("file_read") < pos("anchor_fake_search_a"));
    assert!(pos("anchor_fake_search_b") < pos("image_info"));
    // End sources, and sources with an unknown anchor, trail every built-in
    // and keep registration order.
    assert!(pos("image_info") < pos("anchor_fake_delegate"));
    assert!(pos("anchor_fake_delegate") < pos("anchor_fake_end"));
    assert!(pos("anchor_fake_end") < pos("anchor_fake_unknown"));
    // Sibling tests register sources on the same process-wide slot, so only
    // relative positions are asserted, never the total length.
}

/// A media source that reads the security policy's files roots while it builds
/// must find them already recorded.
#[cfg(feature = "media")]
#[test]
fn media_files_roots_are_recorded_before_any_source_builds() {
    let tmp = TempDir::new().unwrap();
    let seen = Arc::new(std::sync::Mutex::new(None));
    let sink = Arc::clone(&seen);
    register_tool_source_at(
        "test::roots",
        ToolSourcePlacement::After(ANCHOR_AFTER_SEARCH),
        move |ctx| {
            *sink.lock().unwrap() = Some(ctx.security.media_files_roots.lock().unwrap().is_some());
            Vec::new()
        },
    );
    let _ = assemble(&tmp);
    unregister_tool_source("test::roots");
    assert_eq!(*seen.lock().unwrap(), Some(true));
}
