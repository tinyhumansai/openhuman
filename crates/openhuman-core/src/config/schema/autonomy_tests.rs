use super::*;

#[test]
fn default_forbidden_paths_are_absolute_or_home_relative() {
    let config = AutonomyConfig::default();

    assert!(config
        .forbidden_paths
        .iter()
        .all(|entry| { entry.starts_with('~') || std::path::Path::new(entry).is_absolute() }));
}
