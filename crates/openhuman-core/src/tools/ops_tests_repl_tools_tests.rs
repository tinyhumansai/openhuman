//! REPL tools (`juice_find`, `juice_extract`, `juice_summarize`) are registered
//! only while a handle can name them.

use super::*;

/// The full registry for `cfg`, with a disabled browser.
fn registry_for(tmp: &TempDir, cfg: &Config) -> Vec<Box<dyn Tool>> {
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig {
        enabled: false,
        allowed_domains: vec![],
        session_name: None,
        ..BrowserConfig::default()
    };
    all_tools(
        Arc::new(cfg.clone()),
        &security,
        &browser,
        &crate::config::HttpRequestConfig::default(),
        tmp.path(),
        &HashMap::new(),
        cfg,
    )
}

#[test]
fn repl_tools_are_registered_by_default_with_the_recovery_tool() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    assert!(crate::inference::tokenjuice::repl_handle_active(&cfg));
    let names = tool_names(&registry_for(&tmp, &cfg));
    assert_contains_all(&names, tinyjuice_bus::tools::REPL_TOOLS);
    assert_contains_all(&names, &[tinyjuice_bus::tools::RETRIEVE]);
}

#[test]
fn repl_tools_are_absent_whenever_a_handle_cannot_be_produced() {
    let tmp = TempDir::new().unwrap();
    type Flip = fn(&mut Config);
    let off: [(&str, Flip); 4] = [
        ("compaction off", |c| c.context.compaction_enabled = false),
        ("router off", |c| c.tokenjuice.router_enabled = false),
        ("ccr off", |c| c.tokenjuice.ccr_enabled = false),
        ("handle mode off", |c| {
            c.tokenjuice.repl_handle_enabled = false
        }),
    ];
    for (label, flip) in off {
        let mut cfg = test_config(&tmp);
        flip(&mut cfg);
        assert!(
            !crate::inference::tokenjuice::repl_handle_active(&cfg),
            "{label}"
        );
        let names = tool_names(&registry_for(&tmp, &cfg));
        for absent in tinyjuice_bus::tools::REPL_TOOLS {
            assert!(
                !names.iter().any(|n| n == absent),
                "{label}: `{absent}` must not be registered"
            );
        }
        // The whole-original recovery tool is independent of handle mode.
        assert_contains_all(&names, &[tinyjuice_bus::tools::RETRIEVE]);
    }
}

#[test]
fn repl_tools_belong_to_the_inference_family_and_are_read_only() {
    use crate::core::all::DomainGroup;
    for name in tinyjuice_bus::tools::REPL_TOOLS {
        assert_eq!(tool_group(name), DomainGroup::Inference, "{name}");
    }
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    let tools = registry_for(&tmp, &cfg);
    for name in tinyjuice_bus::tools::REPL_TOOLS {
        let tool = find_tool(&tools, name);
        assert_eq!(
            tool.permission_level(),
            tinytools::PermissionLevel::ReadOnly
        );
        assert!(tool.is_concurrency_safe(&serde_json::json!({})));
    }
}

#[test]
fn juice_off_registers_no_juice_or_recovery_tools() {
    let tmp = TempDir::new().unwrap();
    let mut cfg = test_config(&tmp);
    cfg.components.juice = false;
    let names = tool_names(&registry_for(&tmp, &cfg));
    for absent in tinyjuice_bus::tools::REPL_TOOLS.iter().chain([
        &tinyjuice_bus::tools::RETRIEVE,
        &tinyjuice_bus::tools::LEGACY_RETRIEVE,
    ]) {
        assert!(!names.iter().any(|n| n == absent), "`{absent}` registered");
    }
    // Unchanged elsewhere: the rest of the registry is still there.
    assert!(names.iter().any(|n| n == "current_time"));
}
