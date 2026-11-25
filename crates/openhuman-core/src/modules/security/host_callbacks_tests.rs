//! Which secret scope a config names on the admitted TinySecurity module.

use super::*;

#[test]
fn a_config_names_the_secret_scope_beside_its_config_file() {
    let config = Config {
        config_path: PathBuf::from("/tmp/openhuman-scratch-first/config.toml"),
        ..Config::default()
    };
    assert_eq!(
        config_key_path(&config),
        Path::new("/tmp/openhuman-scratch-first/.secret_key")
    );
}

/// A desktop profile registered at runtime keeps the registration id the
/// single-scope host configured at init, so its persisted device-pairing
/// state stays bound to it.
#[test]
fn a_scopes_first_runtime_registration_keeps_the_single_scope_ids() {
    let key = Path::new("/tmp/openhuman-scratch-compat/.secret_key");
    let ids = super::super::secret_tenants::ids_for(key, "Keyring");
    assert_eq!(
        ids.registration_id,
        opaque_id("secret-registration", key, "config-user-master")
    );
    assert_eq!(
        ids.consumer_id,
        opaque_id("secret-cipher-consumer", key, "config-user-master")
    );
    assert_eq!(
        ids.target_backend_id,
        opaque_id("secret-backend", key, "Keyring")
    );
    assert_eq!(
        ids.legacy_backend_id,
        opaque_id("secret-legacy", key, "file")
    );
}
