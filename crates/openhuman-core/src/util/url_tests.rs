use super::*;

#[test]
fn join_url_replaces_any_base_path_with_the_absolute_route() {
    assert_eq!(
        join_url("https://h.example", "/auth/me"),
        "https://h.example/auth/me"
    );
    assert_eq!(
        join_url(
            "https://h.example/openai/v1/chat/completions",
            "/agent-integrations/x"
        ),
        "https://h.example/agent-integrations/x"
    );
    assert_eq!(join_url(" https://h.example/ ", ""), "https://h.example");
    assert_eq!(join_url("not a url", "/x"), "not a url/x");
    assert_eq!(join_url("not a url/", "x"), "not a url/x");
}

#[test]
fn normalize_backend_api_base_url_keeps_only_the_origin() {
    assert_eq!(
        normalize_backend_api_base_url("https://h.example/openai/v1/chat/completions?x=1#f"),
        "https://h.example"
    );
    assert_eq!(
        normalize_backend_api_base_url("h.example/openai/v1"),
        "https://h.example"
    );
    assert_eq!(normalize_backend_api_base_url("   "), "");
    assert_eq!(
        normalize_api_base_url(" https://h.example// "),
        "https://h.example"
    );
}
