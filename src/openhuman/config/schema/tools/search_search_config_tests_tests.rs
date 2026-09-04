use super::*;
use crate::openhuman::config::schema::tools::http::HttpRequestConfig;

#[test]
fn defaults_to_managed() {
    let cfg = SearchConfig::default();
    assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
}

#[test]
fn disabled_stays_disabled() {
    let cfg = SearchConfig {
        engine: SEARCH_ENGINE_DISABLED.into(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_engine(), SearchEngine::Disabled);
}

#[test]
fn parallel_requires_key() {
    let mut cfg = SearchConfig {
        engine: SEARCH_ENGINE_PARALLEL.into(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
    cfg.parallel.api_key = Some("  ".into());
    assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
    cfg.parallel.api_key = Some("real".into());
    assert_eq!(cfg.effective_engine(), SearchEngine::Parallel);
}

#[test]
fn brave_requires_key() {
    let mut cfg = SearchConfig {
        engine: SEARCH_ENGINE_BRAVE.into(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
    cfg.brave.api_key = Some("real".into());
    assert_eq!(cfg.effective_engine(), SearchEngine::Brave);
}

#[test]
fn querit_requires_key() {
    let mut cfg = SearchConfig {
        engine: SEARCH_ENGINE_QUERIT.into(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
    cfg.querit.api_key = Some("real".into());
    assert_eq!(cfg.effective_engine(), SearchEngine::Querit);
}

#[test]
fn exa_requires_key() {
    let mut cfg = SearchConfig {
        engine: SEARCH_ENGINE_EXA.into(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
    cfg.exa.api_key = Some("  ".into());
    assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
    cfg.exa.api_key = Some("real".into());
    assert_eq!(cfg.effective_engine(), SearchEngine::Exa);
}

#[test]
fn exa_key_does_not_disturb_the_managed_default() {
    // BYOK Exa must be opt-in: a stored key alone never flips the engine.
    let mut cfg = SearchConfig::default();
    cfg.exa.api_key = Some("real".into());
    assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
}

#[test]
fn tavily_requires_key() {
    let mut cfg = SearchConfig {
        engine: SEARCH_ENGINE_TAVILY.into(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
    cfg.tavily.api_key = Some("  ".into());
    assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
    cfg.tavily.api_key = Some("real".into());
    assert_eq!(cfg.effective_engine(), SearchEngine::Tavily);
}

#[test]
fn tavily_key_does_not_disturb_the_managed_default() {
    // BYOK Tavily must be opt-in: a stored key alone never flips the engine.
    let mut cfg = SearchConfig::default();
    cfg.tavily.api_key = Some("real".into());
    assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
}

#[test]
fn http_request_defaults_to_allow_all() {
    // Web research works out of the box: the default allowlist is the
    // wildcard. The SSRF guard (url_guard) still blocks local/private
    // hosts regardless, so this only opens public sites.
    let cfg = HttpRequestConfig::default();
    assert_eq!(cfg.allowed_domains, vec!["*".to_string()]);
    assert_eq!(cfg.max_response_size, 1_000_000);
    assert_eq!(cfg.timeout_secs, 30);
}

#[test]
fn unknown_engine_falls_back_to_managed() {
    let cfg = SearchConfig {
        engine: "duckduckgo".into(),
        ..Default::default()
    };
    assert_eq!(cfg.effective_engine(), SearchEngine::Managed);
}
