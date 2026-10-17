use super::*;

#[test]
fn search_switched_off_in_settings_is_unavailable() {
    let mut config = Config::default();
    config.search.enabled = Some(false);
    assert!(OpenHumanSearch::available(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config
    )
    .is_none());
}

#[test]
fn availability_follows_the_configured_providers() {
    // Whatever the default providers resolve to, a backend is returned only
    // when search is on; it never panics on a default config.
    let mut config = Config::default();
    config.search.enabled = Some(true);
    if let Some(search) =
        OpenHumanSearch::available(&crate::core::runtime::DispatchCtx::host_default(), &config)
    {
        assert!(search.config.search.is_enabled());
    }
}
