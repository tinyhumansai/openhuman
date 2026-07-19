//! Unit tests for the custom-server validation helpers.

use serde_json::json;

use super::*;

fn stdio_input(command: &str) -> CustomServerInput {
    CustomServerInput {
        display_name: "Local Server".to_string(),
        transport: "stdio".to_string(),
        command: Some(command.to_string()),
        args: vec!["-y".to_string(), "pkg".to_string()],
        ..Default::default()
    }
}

fn http_input(url: &str) -> CustomServerInput {
    CustomServerInput {
        display_name: "Remote Server".to_string(),
        transport: "http_remote".to_string(),
        url: Some(url.to_string()),
        ..Default::default()
    }
}

#[test]
fn stdio_input_resolves_command_and_args() {
    let (transport, kind, command, args) =
        build_custom_transport(&stdio_input("npx")).expect("stdio resolves");
    assert_eq!(transport, Transport::Stdio);
    assert_eq!(kind, CommandKind::Node);
    assert_eq!(command, "npx");
    assert_eq!(args, vec!["-y".to_string(), "pkg".to_string()]);
}

/// Blank args are form noise (an empty row in the args editor), not
/// arguments — passing them through would hand the subprocess an empty
/// argv entry.
#[test]
fn stdio_input_drops_blank_args() {
    let mut input = stdio_input("npx");
    input.args = vec!["-y".to_string(), "   ".to_string(), String::new()];
    let (_, _, _, args) = build_custom_transport(&input).expect("stdio resolves");
    assert_eq!(args, vec!["-y".to_string()]);
}

#[test]
fn stdio_input_requires_command() {
    let err = build_custom_transport(&stdio_input("   ")).expect_err("blank command rejected");
    assert!(err.contains("command must not be empty"), "got: {err}");
}

#[test]
fn http_input_resolves_url() {
    let (transport, _, command, args) =
        build_custom_transport(&http_input("https://x.io/mcp")).expect("http resolves");
    assert_eq!(
        transport,
        Transport::HttpRemote {
            url: "https://x.io/mcp".to_string()
        }
    );
    assert!(command.is_empty(), "http_remote stores no command");
    assert!(args.is_empty(), "http_remote stores no args");
}

#[test]
fn http_input_requires_url() {
    let err = build_custom_transport(&http_input("  ")).expect_err("blank url rejected");
    assert!(err.contains("url must not be empty"), "got: {err}");
}

#[test]
fn http_input_rejects_relative_url() {
    let err = build_custom_transport(&http_input("/mcp")).expect_err("relative url rejected");
    assert!(err.contains("not a valid absolute URL"), "got: {err}");
}

/// A non-http scheme must never reach the HTTP client.
#[test]
fn http_input_rejects_non_http_scheme() {
    let err = build_custom_transport(&http_input("file:///etc/passwd"))
        .expect_err("file scheme rejected");
    assert!(err.contains("scheme must be http or https"), "got: {err}");
}

#[test]
fn unknown_transport_is_rejected() {
    let mut input = stdio_input("npx");
    input.transport = "carrier-pigeon".to_string();
    let err = build_custom_transport(&input).expect_err("unknown transport rejected");
    assert!(
        err.contains("must be `stdio` or `http_remote`"),
        "got: {err}"
    );
}

/// A URL is stored verbatim and echoed back in `InstalledServer`, so a
/// credential in the userinfo would be persisted in cleartext and returned.
#[test]
fn url_with_embedded_credentials_is_rejected() {
    for url in [
        "https://user:pass@host.example/mcp",
        "https://user@host.example/mcp",
        "http://alice:secret@127.0.0.1:8080/mcp",
    ] {
        let err = build_custom_transport(&http_input(url))
            .expect_err(&format!("embedded-credential URL `{url}` must be rejected"));
        assert!(err.contains("must not embed credentials"), "got: {err}");
    }
}

/// A credential-free URL still passes.
#[test]
fn url_without_credentials_is_accepted() {
    assert!(build_custom_transport(&http_input("https://host.example/mcp")).is_ok());
}

#[test]
fn command_kind_is_inferred_from_launcher() {
    assert_eq!(infer_command_kind("npx"), CommandKind::Node);
    assert_eq!(infer_command_kind("uvx"), CommandKind::Python);
    assert_eq!(
        infer_command_kind("/usr/local/bin/python3"),
        CommandKind::Python
    );
    assert_eq!(infer_command_kind(r"C:\tools\npx.exe"), CommandKind::Node);
    assert_eq!(
        infer_command_kind("/opt/my-mcp-server"),
        CommandKind::Binary
    );
}

/// `__oauth__` holds the OAuth refresh bundle; a user-supplied `__` key
/// would be dropped from outgoing headers anyway and could collide with it.
#[test]
fn reserved_env_keys_are_rejected() {
    let env = HashMap::from([("__oauth__".to_string(), "{}".to_string())]);
    let err = validate_env(&env, false).expect_err("reserved key rejected");
    assert!(err.contains("reserved"), "got: {err}");
}

/// Header names are case-insensitive (RFC 9110), so the core rejects a
/// case-variant duplicate over /rpc even though the form also blocks it.
#[test]
fn case_variant_headers_are_rejected_on_http_remote() {
    let env = HashMap::from([
        ("Authorization".to_string(), "a".to_string()),
        ("authorization".to_string(), "b".to_string()),
    ]);
    let err = validate_env(&env, true).expect_err("case-variant header rejected");
    assert!(err.contains("more than once"), "got: {err}");
}

/// Env var names are case-sensitive on Unix, so stdio keeps both.
#[cfg(not(windows))]
#[test]
fn case_variant_env_vars_are_allowed_on_stdio_unix() {
    let env = HashMap::from([
        ("Path".to_string(), "a".to_string()),
        ("PATH".to_string(), "b".to_string()),
    ]);
    assert!(validate_env(&env, false).is_ok());
}

/// Env var names are case-insensitive on Windows (`Path` == `PATH`), so a
/// case-only stdio duplicate would collapse in the spawned process.
#[cfg(windows)]
#[test]
fn case_variant_env_vars_are_rejected_on_stdio_windows() {
    let env = HashMap::from([
        ("Path".to_string(), "a".to_string()),
        ("PATH".to_string(), "b".to_string()),
    ]);
    let err = validate_env(&env, false).expect_err("case-variant env var rejected on Windows");
    assert!(err.contains("more than once"), "got: {err}");
}

/// The rows mean subprocess env on stdio and request headers on
/// http_remote, so nothing stored survives a switch — a blank submitted key
/// resolves to nothing rather than to the stored secret.
#[test]
fn transport_change_drops_stored_env() {
    let stored = HashMap::from([
        ("GITHUB_TOKEN".to_string(), "ghp_live".to_string()),
        (
            "__oauth__".to_string(),
            "{\"refresh_token\":\"r\"}".to_string(),
        ),
    ]);
    let submitted = HashMap::from([("GITHUB_TOKEN".to_string(), String::new())]);

    let resolved = resolve_env_for_transport(
        &submitted,
        &stored,
        &Transport::Stdio,
        &Transport::HttpRemote {
            url: "https://x.io/mcp".to_string(),
        },
    );

    assert!(
        resolved.is_empty(),
        "a stdio secret must not become a header on the new endpoint: {resolved:?}"
    );
}

/// `__oauth__` is a refresh bundle for one endpoint. `McpStdioClient` hands
/// the whole env to the child process with no `__` filter, so carrying it
/// into stdio would give a user-typed command the refresh token.
#[test]
fn transport_change_drops_the_oauth_bundle() {
    let stored = HashMap::from([(
        "__oauth__".to_string(),
        "{\"refresh_token\":\"r\",\"client_secret\":\"s\"}".to_string(),
    )]);

    let resolved = resolve_env_for_transport(
        &HashMap::new(),
        &stored,
        &Transport::HttpRemote {
            url: "https://x.io/mcp".to_string(),
        },
        &Transport::Stdio,
    );

    assert!(
        !resolved.contains_key("__oauth__"),
        "the OAuth bundle must not reach a subprocess: {resolved:?}"
    );
}

/// Same transport: the blank-means-keep contract holds, so an unrelated
/// rename doesn't wipe the credentials.
#[test]
fn same_transport_keeps_stored_env() {
    let stored = HashMap::from([
        ("API_KEY".to_string(), "secret".to_string()),
        ("__oauth__".to_string(), "{}".to_string()),
    ]);
    let submitted = HashMap::from([("API_KEY".to_string(), String::new())]);

    let resolved =
        resolve_env_for_transport(&submitted, &stored, &Transport::Stdio, &Transport::Stdio);

    assert_eq!(resolved.get("API_KEY"), Some(&"secret".to_string()));
    assert_eq!(resolved.get("__oauth__"), Some(&"{}".to_string()));
}

/// A URL edit that keeps the same origin (path/query only) keeps the env —
/// the bearer token and OAuth bundle are still valid for that origin.
#[test]
fn same_origin_url_change_keeps_stored_env() {
    let stored = HashMap::from([("Authorization".to_string(), "Bearer t".to_string())]);
    let submitted = HashMap::from([("Authorization".to_string(), String::new())]);

    let resolved = resolve_env_for_transport(
        &submitted,
        &stored,
        &Transport::HttpRemote {
            url: "https://svc.io/mcp".to_string(),
        },
        &Transport::HttpRemote {
            url: "https://svc.io/mcp/v2".to_string(),
        },
    );

    assert_eq!(resolved.get("Authorization"), Some(&"Bearer t".to_string()));
}

/// Pointing the server at a *different origin* re-scopes the credentials: a
/// token minted for one service must not be sent to another. `__oauth__`
/// would otherwise re-mint against the old endpoint and ship the result to
/// the new host.
#[test]
fn cross_origin_url_change_drops_stored_env() {
    let stored = HashMap::from([
        ("Authorization".to_string(), "Bearer for-a".to_string()),
        (
            "__oauth__".to_string(),
            "{\"refresh_token\":\"r\"}".to_string(),
        ),
    ]);
    let submitted = HashMap::from([("Authorization".to_string(), String::new())]);

    let resolved = resolve_env_for_transport(
        &submitted,
        &stored,
        &Transport::HttpRemote {
            url: "https://a.com/mcp".to_string(),
        },
        &Transport::HttpRemote {
            url: "https://b.com/mcp".to_string(),
        },
    );

    assert!(
        resolved.is_empty(),
        "a token for a.com must not carry to b.com: {resolved:?}"
    );
}

/// A different port is a different origin.
#[test]
fn port_change_drops_stored_env() {
    let stored = HashMap::from([("Authorization".to_string(), "Bearer t".to_string())]);
    let resolved = resolve_env_for_transport(
        &HashMap::new(),
        &stored,
        &Transport::HttpRemote {
            url: "https://svc.io:8443/mcp".to_string(),
        },
        &Transport::HttpRemote {
            url: "https://svc.io:9443/mcp".to_string(),
        },
    );
    assert!(
        resolved.is_empty(),
        "different port re-scopes: {resolved:?}"
    );
}

#[test]
fn ordinary_env_keys_are_accepted() {
    let env = HashMap::from([("Authorization".to_string(), "Bearer t".to_string())]);
    assert!(validate_env(&env, true).is_ok());
}

/// `resolve_env` trims before storing, so validating the raw key would let a
/// padded `"  __oauth__"` through and land it as `__oauth__` — a caller
/// could then plant a refresh bundle pointing at a token endpoint of their
/// choosing. Validate what actually gets stored.
#[test]
fn reserved_env_keys_are_rejected_despite_padding() {
    for padded in ["  __oauth__", "__oauth__  ", "\t__oauth__"] {
        let env = HashMap::from([(padded.to_string(), "{}".to_string())]);
        let err = validate_env(&env, false)
            .expect_err(&format!("padded reserved key `{padded}` must be rejected"));
        assert!(err.contains("reserved"), "got: {err}");
    }
}

#[test]
fn empty_env_key_is_rejected() {
    let env = HashMap::from([("  ".to_string(), "v".to_string())]);
    assert!(validate_env(&env, false).is_err());
}

fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// A retyped value wins over the stored one.
#[test]
fn resolve_env_takes_supplied_values() {
    let resolved = resolve_env(&env(&[("KEY", "new")]), &env(&[("KEY", "old")]), false);
    assert_eq!(resolved.get("KEY").map(String::as_str), Some("new"));
}

/// The edit form cannot render stored secrets (they are never returned), so
/// an untouched row arrives blank. Blank must mean "keep" — dropping it
/// would erase the credential on an unrelated rename.
#[test]
fn resolve_env_blank_value_keeps_stored_secret() {
    let resolved = resolve_env(&env(&[("KEY", "")]), &env(&[("KEY", "stored")]), false);
    assert_eq!(resolved.get("KEY").map(String::as_str), Some("stored"));
}

/// Removing a row must actually delete the key — this is why the edit path
/// resolves against the submitted key set rather than merging like
/// `mcp_clients_update_env` does.
#[test]
fn resolve_env_omitted_key_is_removed() {
    let resolved = resolve_env(
        &env(&[("KEEP", "v")]),
        &env(&[("KEEP", "v"), ("GONE", "x")]),
        false,
    );
    assert!(resolved.contains_key("KEEP"));
    assert!(
        !resolved.contains_key("GONE"),
        "omitted key must be dropped"
    );
}

/// The form never submits `__oauth__`, so without an explicit carry-over an
/// edit would drop the refresh bundle and sign the user out of an
/// OAuth-authenticated server.
#[test]
fn resolve_env_preserves_reserved_internal_state() {
    let resolved = resolve_env(
        &env(&[("Authorization", "Bearer new")]),
        &env(&[("__oauth__", "{\"refresh_token\":\"r\"}")]),
        true,
    );
    assert_eq!(
        resolved.get("__oauth__").map(String::as_str),
        Some("{\"refresh_token\":\"r\"}")
    );
    assert_eq!(
        resolved.get("Authorization").map(String::as_str),
        Some("Bearer new")
    );
}

/// http_remote header names are case-insensitive, so re-casing a header and
/// leaving the value blank still means "keep" — a case-sensitive lookup would
/// miss the stored value and silently erase the credential.
#[test]
fn resolve_env_blank_keeps_stored_header_across_case_change() {
    let resolved = resolve_env(
        &env(&[("authorization", "")]),
        &env(&[("Authorization", "Bearer keep")]),
        true,
    );
    assert_eq!(
        resolved.get("authorization").map(String::as_str),
        Some("Bearer keep"),
        "a re-cased header with a blank value must keep the stored secret"
    );
}

/// stdio env var names are case-sensitive, so the same blank re-cased key is
/// a *different* key with nothing stored — not a keep.
#[test]
fn resolve_env_blank_recased_key_is_dropped_on_stdio() {
    let resolved = resolve_env(&env(&[("path", "")]), &env(&[("PATH", "/usr/bin")]), false);
    assert!(
        resolved.is_empty(),
        "a case-different env var is a new key, not a keep: {resolved:?}"
    );
}

/// On add there is nothing stored, so a blank row is simply not a value.
#[test]
fn resolve_env_drops_blank_value_with_nothing_stored() {
    let resolved = resolve_env(&env(&[("KEY", "  ")]), &HashMap::new(), false);
    assert!(resolved.is_empty());
}

#[test]
fn env_key_list_is_sorted() {
    assert_eq!(
        env_key_list(&env(&[("b", "1"), ("a", "2")])),
        vec!["a".to_string(), "b".to_string()]
    );
}

#[test]
fn slugify_normalises_punctuation_and_case() {
    assert_eq!(slugify("My Cool Server").as_deref(), Some("my-cool-server"));
    assert_eq!(slugify("  @scope/thing!  ").as_deref(), Some("scope-thing"));
    assert_eq!(slugify("a---b").as_deref(), Some("a-b"));
}

/// A name with no ASCII alphanumerics yields no slug; the caller substitutes
/// a per-server fragment so distinct non-Latin names don't collide.
#[test]
fn slugify_is_none_for_non_ascii_names() {
    assert_eq!(slugify("한글 서버"), None);
    assert_eq!(slugify("日本語"), None);
    assert_eq!(slugify(""), None);
}

/// The whole point of the `Option` return: two differently-named all-CJK
/// servers get *different* base slugs from their unique server_ids, instead
/// of both collapsing onto one constant and racing the collision suffix. The
/// suffix loop itself needs the DB and is covered in `json_rpc_e2e`.
#[test]
fn base_slug_is_distinct_for_distinct_non_ascii_names() {
    let a = base_slug("한글 서버", "11111111-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
    let b = base_slug("日本語サーバー", "22222222-bbbb-4bbb-8bbb-bbbbbbbbbbbb");
    assert_ne!(a, b, "distinct non-Latin names collided on `{a}`");
    assert_eq!(a, "server-11111111");
    assert_eq!(b, "server-22222222");
}

/// An ASCII name ignores the server_id and uses its own slug.
#[test]
fn base_slug_prefers_the_display_name_slug() {
    assert_eq!(base_slug("My Server", "unused-id"), "my-server");
}
