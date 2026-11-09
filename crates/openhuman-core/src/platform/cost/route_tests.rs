use super::*;

/// Both decorations, in BOTH orders, must normalize to the same slug.
/// A fixed-order strip classified `openhuman/hint:…` as BYOK, so managed
/// spend silently stopped counting toward the cap depending only on which
/// prefix a recording site applied first (review, #5016).
#[test]
fn decoration_prefixes_are_order_independent() {
    for id in [
        "openrouter/test/chat-v1",
        "hint:openrouter/test/chat-v1",
        "openhuman/openrouter/test/chat-v1",
        "hint:openhuman/openrouter/test/chat-v1",
        "openhuman/hint:openrouter/test/chat-v1",
        "  HINT:OpenHuman/OpenRouter/Test/Chat-V1  ",
    ] {
        assert_eq!(
            route_for_model(id),
            CostRoute::Managed,
            "{id} must classify as managed"
        );
    }
}

#[test]
fn decorations_do_not_promote_a_byok_model_to_managed() {
    for id in [
        "openhuman/hint:llama3:8b",
        "hint:openhuman/anthropic/claude-sonnet-4-20250514",
        "ollama:chat-v1-not-a-slug",
    ] {
        assert_eq!(route_for_model(id), CostRoute::Byok, "{id} must stay BYOK");
    }
}

/// The managed default is an `openrouter/...` passthrough id and bills as
/// managed like every other catalog model the backend serves.
#[test]
fn managed_default_model_is_managed() {
    assert_eq!(
        route_for_model("openrouter/deepseek/deepseek-v4-flash"),
        CostRoute::Managed
    );
}

#[test]
fn retired_provider_aliases_need_a_registered_provider_policy() {
    for tier in [
        "chat-v1",
        "reasoning-v1",
        "agentic-v1",
        "burst-v1",
        "coding-v1",
        "vision-v1",
        "summarization-v1",
    ] {
        assert_eq!(route_for_model(tier), CostRoute::Byok);
    }
}

#[test]
fn byok_and_local_models_are_not_managed() {
    // The exact models from #5016 / #5127 (OpenRouter + a self-hosted
    // OpenAI-compatible gateway) and other common BYOK / local shapes.
    for model in [
        "minimax/minimax-m3",
        "anthropic/claude-sonnet-4-20250514",
        "openai/gpt-4o",
        "llama3:8b",
        "ollama:gemma3:1b-it-qat",
        "lmstudio:qwen2.5-coder",
        "",
    ] {
        assert_eq!(
            route_for_model(model),
            CostRoute::Byok,
            "{model} must not bill as managed"
        );
    }
}

#[test]
fn normalizes_case_whitespace_and_prefixes() {
    crate::agent::session_host::test_support::install_model_fixture();
    assert_eq!(route_for_model("  FIXTURE-REASONING "), CostRoute::Managed);
    assert_eq!(route_for_model("fixture-reasoning"), CostRoute::Managed);
    assert_eq!(
        route_for_model("hint:fixture-reasoning"),
        CostRoute::Managed
    );
    assert_eq!(
        route_for_model("openhuman/fixture-reasoning"),
        CostRoute::Managed
    );
}

#[test]
fn a_byok_model_merely_containing_a_tier_name_is_not_managed() {
    // Substring matching would misclassify these and silently re-introduce
    // the phantom limit for the user.
    for model in ["vendor/chat-v1-turbo", "my-chat-v1", "chat-v1x"] {
        assert_eq!(route_for_model(model), CostRoute::Byok, "{model}");
    }
}

#[test]
fn only_managed_counts_toward_budget() {
    assert!(CostRoute::Managed.counts_toward_budget());
    assert!(!CostRoute::Byok.counts_toward_budget());
}

/// The managed backend's OpenRouter passthrough bills managed credits, so its
/// `openrouter/<author>/<slug>` ids must count toward the local managed cap.
/// They are not tier slugs, so before this they fell to the `Byok` default and
/// managed spend silently stopped being counted.
#[test]
fn managed_openrouter_passthrough_ids_are_managed() {
    for model in [
        "openrouter/deepseek/deepseek-v4-flash",
        "openrouter/anthropic/claude-sonnet-4.5",
        // decorations must normalize away first, in either order
        "hint:openrouter/deepseek/deepseek-v4-flash",
        "openhuman/openrouter/deepseek/deepseek-v4-flash",
        "  OpenRouter/DeepSeek/DeepSeek-V4-Flash  ",
    ] {
        assert_eq!(route_for_model(model), CostRoute::Managed, "{model}");
    }
}

/// A BYOK OpenRouter provider addresses models by their bare upstream slug, so
/// those must stay BYOK. Only the `openrouter/` qualifier means managed
/// passthrough, and only with exactly `<author>/<slug>` after it.
#[test]
fn byok_openrouter_slugs_and_malformed_passthrough_ids_stay_byok() {
    for model in [
        // bare upstream slug — what a BYOK openrouter provider sends
        "deepseek/deepseek-v4-flash",
        // wrong segment count after the qualifier
        "openrouter/deepseek",
        "openrouter/",
        "openrouter/a/b/c",
        // empty segments are not the passthrough shape either
        "openrouter/a//b",
        "openrouter/a/b/",
        "openrouter//b",
        "openrouter///",
    ] {
        assert_eq!(route_for_model(model), CostRoute::Byok, "{model}");
    }
}
