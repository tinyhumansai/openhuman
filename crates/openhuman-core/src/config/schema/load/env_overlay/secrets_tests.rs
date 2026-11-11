use super::*;

struct OneVar(&'static str);

impl EnvLookup for OneVar {
    fn get(&self, key: &str) -> Option<String> {
        (key == UNATTENDED_UNLOCK_ENV).then(|| self.0.to_owned())
    }
}

#[test]
fn secrets_unattended_unlock_env_overrides_the_file() {
    let mut cfg = Config::default();
    assert!(!cfg.secrets.unattended_unlock, "off by default");
    cfg.apply_secrets_env(&OneVar("1"));
    assert!(cfg.secrets.unattended_unlock);

    cfg.apply_secrets_env(&OneVar("off"));
    assert!(
        !cfg.secrets.unattended_unlock,
        "0/off turns a file setting off"
    );

    for ignored in ["", "  ", "maybe"] {
        cfg.secrets.unattended_unlock = true;
        cfg.apply_secrets_env(&OneVar(ignored));
        assert!(
            cfg.secrets.unattended_unlock,
            "{ignored:?} leaves the file setting alone"
        );
    }
}

#[test]
fn the_overlay_applies_the_secrets_override() {
    let mut cfg = Config::default();
    cfg.apply_env_overlay_with(&OneVar("true"));
    assert!(cfg.secrets.unattended_unlock);
}

#[test]
fn a_raw_config_read_still_honours_the_env_override() {
    use crate::config::schema::SecretsConfig;
    assert!(SecretsConfig::unattended_unlock_with(false, Some("1")));
    assert!(!SecretsConfig::unattended_unlock_with(true, Some("off")));
    assert!(SecretsConfig::unattended_unlock_with(true, Some("")));
    assert!(SecretsConfig::unattended_unlock_with(true, Some("maybe")));
    assert!(!SecretsConfig::unattended_unlock_with(false, None));
    assert!(SecretsConfig::unattended_unlock_with(true, None));
}
