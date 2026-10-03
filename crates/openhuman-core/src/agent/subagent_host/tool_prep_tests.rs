use super::*;

#[test]
fn custom_delegate_is_treated_as_spawn_tool() {
    assert!(is_subagent_spawn_tool("spawn_subagent"));
    assert!(is_subagent_spawn_tool("delegate_researcher"));
    assert!(!is_subagent_spawn_tool("directory_resolve"));
}

#[test]
fn unprefixed_delegate_name_overrides_are_treated_as_spawn_tools() {
    // Most synthesised delegation tools use an unprefixed
    // `delegate_name` override (`plan`, `manage_tasks`, `create_image`, …).
    // They must be stripped from every sub-agent surface, exactly like
    // the `delegate_*`-prefixed defaults.
    let tmp = tempfile::TempDir::new().unwrap();
    crate::agent::harness::definition::AgentDefinitionRegistry::init_global(tmp.path()).unwrap();
    for delegate in [
        "plan",
        "review_code",
        "manage_tasks",
        "create_image",
        // `make_presentation` is `presentation_agent`'s `delegate_name`; the agent —
        // and therefore this delegate tool — is compiled out with the
        // `documents` feature.
        #[cfg(feature = "documents")]
        "make_presentation",
    ] {
        assert!(
            is_subagent_spawn_tool(delegate),
            "`{delegate}` is a synthesised delegation tool and must be \
             stripped from sub-agent tool surfaces"
        );
    }
    // Ordinary worker tools stay visible.
    for plain in ["shell", "file_read", "web_fetch", "todo"] {
        assert!(
            !is_subagent_spawn_tool(plain),
            "`{plain}` must not be classified as a spawn tool"
        );
    }
}

#[test]
fn child_keeps_the_parents_protocol() {
    use crate::agent::prompts::ToolCallFormat;

    for parent in [
        ToolCallFormat::Native,
        ToolCallFormat::PFormat,
        ToolCallFormat::Json,
    ] {
        let (format, _) = subagent_prompt_protocol(parent, &[]);
        assert_eq!(format, parent);
    }
}
