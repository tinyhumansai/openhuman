//! Shared configuration for connector tests using synthetic credentials.

use crate::config::Config;

pub(crate) fn config_in(directory: &std::path::Path) -> Config {
    let mut config = crate::credentials::test_support::plaintext_fixture_config(directory);
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
