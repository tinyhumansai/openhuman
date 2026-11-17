use super::*;

fn config() -> (tempfile::TempDir, Config) {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    (tmp, config)
}

fn env_with<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |name| {
        pairs
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| (*v).to_owned())
    }
}

fn reason(err: RankError) -> String {
    match err {
        RankError::Backend { reason } => reason,
        other => panic!("expected a backend error, got {other}"),
    }
}

#[test]
fn parse_accepts_the_four_spellings_and_defaults_unknown_to_auto() {
    assert_eq!(JevRoute::parse("auto"), JevRoute::Auto);
    assert_eq!(JevRoute::parse(""), JevRoute::Auto);
    assert_eq!(JevRoute::parse(" OpenRouter "), JevRoute::OpenRouter);
    assert_eq!(JevRoute::parse("typesafe"), JevRoute::TypeSafe);
    assert_eq!(JevRoute::parse("tinyhumans"), JevRoute::TinyHumans);
    assert_eq!(JevRoute::parse("nonsense"), JevRoute::Auto);
}

#[tokio::test]
async fn openrouter_route_uses_the_env_key_and_the_base_override() {
    let (_tmp, mut config) = config();
    config.agent.tool_search.jev_route = "openrouter".into();
    config.agent.tool_search.jev_base_url = Some("http://127.0.0.1:18080/".into());
    let env = env_with(&[(OPENROUTER_API_KEY_ENV, "or-key")]);
    let resolved = resolve(&config, &env).await.expect("resolves");
    assert_eq!(resolved.route, JevRoute::OpenRouter);
    assert_eq!(resolved.secret, "or-key");
    // trailing slash trimmed so the client's path join stays single-slashed
    assert_eq!(resolved.client.base_url, "http://127.0.0.1:18080");
}

#[tokio::test]
async fn openrouter_route_defaults_to_openrouters_origin() {
    let (_tmp, mut config) = config();
    config.agent.tool_search.jev_route = "openrouter".into();
    let env = env_with(&[(OPENROUTER_API_KEY_ENV, "or-key")]);
    let resolved = resolve(&config, &env).await.expect("resolves");
    assert_eq!(resolved.client.base_url, "https://openrouter.ai/api");
}

#[tokio::test]
async fn typesafe_route_uses_the_direct_key_and_origin() {
    let (_tmp, mut config) = config();
    config.agent.tool_search.jev_route = "typesafe".into();
    let env = env_with(&[(TYPESAFE_API_KEY_ENV, "ts-key")]);
    let resolved = resolve(&config, &env).await.expect("resolves");
    assert_eq!(resolved.route, JevRoute::TypeSafe);
    assert_eq!(resolved.secret, "ts-key");
    assert_eq!(resolved.client.base_url, "https://api.typesafe.ai");
}

/// An explicit route is a decision: it must not quietly fall through to some
/// other credential and send decision calls somewhere the operator did not pick.
#[tokio::test]
async fn an_explicit_route_does_not_fall_through_to_another_credential() {
    let (_tmp, mut config) = config();
    config.agent.tool_search.jev_route = "openrouter".into();
    let env = env_with(&[(TYPESAFE_API_KEY_ENV, "ts-key")]);
    let err = reason(resolve(&config, &env).await.err().expect("must fail"));
    assert!(err.contains("jev route `openrouter` unavailable"), "{err}");
    assert!(err.contains("no OpenRouter key"), "{err}");
    assert!(!err.contains("ts-key"), "{err}");
}

#[tokio::test]
async fn auto_prefers_typesafe_over_openrouter_when_there_is_no_tinyhumans_credential() {
    let (_tmp, config) = config();
    let env = env_with(&[
        (TYPESAFE_API_KEY_ENV, "ts-key"),
        (OPENROUTER_API_KEY_ENV, "or-key"),
    ]);
    let resolved = resolve(&config, &env).await.expect("resolves");
    assert_eq!(resolved.route, JevRoute::TypeSafe);
}

#[tokio::test]
async fn auto_falls_back_to_openrouter_alone() {
    let (_tmp, config) = config();
    let env = env_with(&[(OPENROUTER_API_KEY_ENV, "or-key")]);
    let resolved = resolve(&config, &env).await.expect("resolves");
    assert_eq!(resolved.route, JevRoute::OpenRouter);
}

#[tokio::test]
async fn auto_with_nothing_names_every_gap_and_leaks_no_secret() {
    let (_tmp, config) = config();
    let env = env_with(&[]);
    let err = reason(resolve(&config, &env).await.err().expect("must fail"));
    assert!(err.contains("no TinyHumans credential"), "{err}");
    assert!(err.contains("no TypeSafe key"), "{err}");
    assert!(err.contains("no OpenRouter key"), "{err}");
}

#[tokio::test]
async fn the_base_override_does_not_apply_to_the_tinyhumans_route() {
    let (_tmp, mut config) = config();
    config.agent.tool_search.jev_route = "tinyhumans".into();
    config.agent.tool_search.jev_base_url = Some("http://127.0.0.1:1".into());
    // No credential in this config, so the route fails: the override must not
    // rescue it by pointing the TinyHumans route elsewhere.
    let err = reason(
        resolve(&config, &env_with(&[]))
            .await
            .err()
            .expect("must fail"),
    );
    assert!(err.contains("no TinyHumans credential"), "{err}");
}
