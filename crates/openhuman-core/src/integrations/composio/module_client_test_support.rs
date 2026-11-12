//! Shared configuration for connector tests using synthetic credentials.

use crate::config::Config;

pub(crate) fn config_in(directory: &std::path::Path) -> Config {
    let mut config = crate::credentials::test_support::plaintext_fixture_config(directory);
    // TinySecurity is loaded once per process and keeps the secret scope of the
    // config dir that admitted it, refusing every other one. Each test brings
    // its own dir, so in one test process every test after the first was
    // refused the redactor ("withheld"). Hand the scope over to this config,
    // as an embedder does between runtimes (TS-E2E-026).
    crate::security::release_secret_scope();
    if let Some(config_dir) = config.config_path.parent() {
        crate::security::declare_transient_secret_scope(config_dir);
    }
    // A locally staged source build exercises the contract compiled by this
    // checkout. The ordinary loader still verifies its operator trust manifest.
    #[cfg(feature = "modules")]
    if let Some(artifact) = crate::modules::local_artifact("tinyconnectors") {
        config
            .modules
            .overrides
            .push(crate::config::schema::ModuleOverride::with_path(
                "tinyconnectors",
                artifact.to_string_lossy(),
            ));
    }
    config
}

/// Run `future` with `config` as the ambient dispatch config. Host checks that
/// read the ambient config instead of a passed one (the egress privacy check)
/// then reach TinySecurity with the scope `config_in` handed over.
pub(crate) async fn scoped<F: std::future::Future>(config: &Config, future: F) -> F::Output {
    let context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config.clone(),
    );
    crate::core::runtime::CoreContext::scope(context, future).await
}
