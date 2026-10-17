use super::*;

#[test]
fn defaults_match_the_documented_values() {
    let c = MemesConfig::default();
    assert_eq!(c.timeout_ms, 20_000);
    assert_eq!(c.meme_cooldown, 3);
}

#[test]
fn a_partial_section_keeps_the_other_defaults() {
    let c: MemesConfig = toml::from_str("timeout_ms = 5000").unwrap();
    assert_eq!(c.timeout_ms, 5000);
    assert_eq!(c.meme_cooldown, DEFAULT_MEME_COOLDOWN);
}
