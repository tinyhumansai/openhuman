use super::*;

#[test]
fn parses_typical_output() {
    assert_eq!(
        parse_version("2.0.4 (Claude Code)\n").as_deref(),
        Some("2.0.4")
    );
}

#[test]
fn rejects_non_numeric_prefix() {
    assert_eq!(parse_version("claude version 2.0.4"), None);
}

#[test]
fn version_compare() {
    assert!(version_lt("1.9.9", "2.0.0"));
    assert!(version_lt("2.0.0", "2.0.1"));
    assert!(!version_lt("2.0.0", "2.0.0"));
    assert!(!version_lt("2.1.0", "2.0.9"));
}

#[test]
fn version_compare_strips_prerelease() {
    assert!(!version_lt("2.0.0-rc.1", "2.0.0"));
}

/// A macOS app launched from Finder gets launchd's minimal `PATH`, so the
/// native-installer location must be probed directly — it is the default
/// install route and the one that regressed in the field.
#[test]
fn well_known_candidates_cover_the_native_installer_and_homebrew() {
    let home = Path::new("/Users/someone");
    let candidates = super::well_known_candidates(Some(home));

    #[cfg(not(windows))]
    assert_eq!(candidates.first(), Some(&home.join(".local/bin/claude")));
    #[cfg(windows)]
    assert_eq!(candidates.first(), Some(&home.join(".local/bin/claude.exe")));
    assert!(candidates.contains(&PathBuf::from("/opt/homebrew/bin/claude")));
    assert!(candidates.contains(&PathBuf::from("/usr/local/bin/claude")));
}

#[test]
fn well_known_candidates_without_a_home_still_probe_system_prefixes() {
    let candidates = super::well_known_candidates(None);

    assert!(candidates.contains(&PathBuf::from("/opt/homebrew/bin/claude")));
    assert!(candidates.contains(&PathBuf::from("/usr/local/bin/claude")));

    #[cfg(windows)]
    {
        let native = PathBuf::from("/opt/homebrew/bin/claude.cmd");
        let shim = PathBuf::from("/opt/homebrew/bin/claude");
        let native_index = candidates
            .iter()
            .position(|p| p == &native)
            .expect("native .cmd candidate");
        let shim_index = candidates
            .iter()
            .position(|p| p == &shim)
            .expect("unsuffixed shim candidate");
        assert!(native_index < shim_index);
    }
}
