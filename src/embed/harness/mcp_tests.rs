use super::*;

#[test]
fn stdio_and_http_are_mutually_exclusive_transports() {
    // A config entry carrying both would leave which transport is used up to
    // the registry's internal precedence rather than to the caller.
    let stdio = McpServer::stdio("gh", "gh-mcp", ["stdio"]).into_config();
    assert_eq!(stdio.command, "gh-mcp");
    assert_eq!(stdio.args, vec!["stdio".to_string()]);
    assert!(stdio.endpoint.is_empty());

    let http = McpServer::http("remote", "https://mcp.example/v1").into_config();
    assert_eq!(http.endpoint, "https://mcp.example/v1");
    assert!(http.command.is_empty());
    assert!(http.args.is_empty());
}

#[test]
fn a_declared_server_is_enabled_with_the_default_timeout() {
    let config = McpServer::http("remote", "https://mcp.example/v1").into_config();
    assert!(
        config.enabled,
        "declaring a server and having it ignored would be a silent no-op"
    );
    assert_eq!(config.timeout_secs, 30);
}

#[test]
fn env_accumulates_across_calls() {
    let config = McpServer::stdio("gh", "gh-mcp", Vec::<String>::new())
        .env([("A", "1")])
        .env([("B", "2")])
        .into_config();
    assert_eq!(config.env.get("A").map(String::as_str), Some("1"));
    assert_eq!(config.env.get("B").map(String::as_str), Some("2"));
}

#[test]
fn tool_filters_round_trip() {
    let config = McpServer::http("remote", "https://mcp.example/v1")
        .allow_tools(["read", "search"])
        .deny_tools(["delete"])
        .into_config();
    assert_eq!(config.allowed_tools, vec!["read", "search"]);
    assert_eq!(config.disallowed_tools, vec!["delete"]);
}

#[test]
fn auth_and_metadata_are_carried_through() {
    let config = McpServer::http("remote", "https://mcp.example/v1")
        .auth(McpAuthConfig::BearerToken { token: "t".into() })
        .description("a remote")
        .timeout_secs(90)
        .cwd("/srv")
        .into_config();

    assert!(matches!(config.auth, McpAuthConfig::BearerToken { .. }));
    assert_eq!(config.description.as_deref(), Some("a remote"));
    assert_eq!(config.timeout_secs, 90);
    assert_eq!(config.cwd.as_deref(), Some("/srv"));
}

#[test]
fn the_name_is_the_agent_facing_slug() {
    let server = McpServer::stdio("github", "gh-mcp", Vec::<String>::new());
    assert_eq!(server.name(), "github");
    assert_eq!(server.into_config().name, "github");
}

#[test]
fn debug_redacts_credentials() {
    // The derived Debug would print bearer tokens, passwords, header values and
    // stdio env values. The manual implementation must redact all of them while
    // keeping the identifying fields readable.
    let server = McpServer::http("remote", "https://mcp.example/v1")
        .auth(McpAuthConfig::BearerToken {
            token: "super-secret-token".into(),
        })
        .env([("AUTH_KEY", "env-secret"), ("TOKEN", "another-secret")]);
    let debug = format!("{server:?}");

    assert!(debug.contains("remote"), "name stays readable");
    assert!(debug.contains("mcp.example"), "endpoint stays readable");
    assert!(
        !debug.contains("super-secret-token")
            && !debug.contains("env-secret")
            && !debug.contains("another-secret"),
        "credentials leaked into Debug: {debug}"
    );
    assert!(debug.contains("redacted"));
}

#[test]
fn debug_redacts_credentials_embedded_in_the_endpoint() {
    // The endpoint can itself carry userinfo or query credentials; Debug must
    // strip them rather than print them verbatim.
    let server = McpServer::http("remote", "https://user:s3cret@mcp.example/v1?api_key=leaky");
    let debug = format!("{server:?}");

    assert!(
        !debug.contains("user") && !debug.contains("s3cret") && !debug.contains("leaky"),
        "endpoint credentials leaked into Debug: {debug}"
    );
    assert!(debug.contains("mcp.example"), "origin stays readable");
}
