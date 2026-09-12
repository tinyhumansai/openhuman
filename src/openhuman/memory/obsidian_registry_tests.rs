//! Ported alongside [`super`] from the engine's `obsidian_registry_tests.rs`,
//! assertion for assertion, so the host copy is held to the same behaviour the
//! engine copy was — including the two regressions the engine's suite pins
//! (an empty vault path must not match every root, and a sibling directory
//! sharing a name prefix is not a match).

use super::*;
use std::io::Write;

/// Write an `obsidian.json` containing `vault_paths` and return its path.
fn write_config(dir: &Path, vault_paths: &[&str]) -> PathBuf {
    let entries: Vec<String> = vault_paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            format!(
                "\"id{i}\": {{ \"path\": {}, \"ts\": 1700000000000, \"open\": true }}",
                serde_json::to_string(p).unwrap()
            )
        })
        .collect();
    let body = format!("{{ \"vaults\": {{ {} }} }}", entries.join(", "));
    let path = dir.join("obsidian.json");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(body.as_bytes()).unwrap();
    path
}

#[test]
fn exact_match_is_registered() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("memory_tree/content");
    let cfg = write_config(tmp.path(), &[root.to_str().unwrap()]);
    let got = registration_in_files(&root, &[cfg]);
    assert_eq!(
        got,
        VaultRegistration {
            registered: true,
            config_found: true
        }
    );
}

#[test]
fn ancestor_vault_is_registered() {
    // A vault rooted at the parent still "contains" the content root.
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().join("workspace");
    let root = parent.join("memory_tree/content");
    let cfg = write_config(tmp.path(), &[parent.to_str().unwrap()]);
    assert!(registration_in_files(&root, &[cfg]).registered);
}

#[test]
fn trailing_slash_does_not_matter() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("memory_tree/content");
    let with_slash = format!("{}/", root.to_str().unwrap());
    let cfg = write_config(tmp.path(), &[&with_slash]);
    assert!(registration_in_files(&root, &[cfg]).registered);
}

#[test]
fn relative_content_root_matches_absolute_vault() {
    // A relative content root must be absolutized against the CWD before the
    // prefix comparison, or an already-registered vault is always reported
    // unregistered.
    let tmp = tempfile::tempdir().unwrap();
    let cwd = std::env::current_dir().unwrap();
    let abs_root = cwd.join("memory_tree/content");
    let cfg = write_config(tmp.path(), &[abs_root.to_str().unwrap()]);
    let got = registration_in_files(Path::new("memory_tree/content"), &[cfg]);
    assert!(
        got.registered,
        "relative content root must be absolutized against the CWD before matching"
    );
}

#[test]
fn unrelated_vault_is_not_registered_but_config_found() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("memory_tree/content");
    let cfg = write_config(tmp.path(), &["/some/other/vault"]);
    let got = registration_in_files(&root, &[cfg]);
    assert_eq!(
        got,
        VaultRegistration {
            registered: false,
            config_found: true
        }
    );
}

#[test]
fn empty_vault_path_does_not_match_every_root() {
    // Regression: a malformed entry with an empty `path` must not
    // normalize to "" and match every content root as an ancestor.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("memory_tree/content");
    let cfg = write_config(tmp.path(), &[""]);
    let got = registration_in_files(&root, &[cfg]);
    assert_eq!(
        got,
        VaultRegistration {
            registered: false,
            config_found: true
        }
    );
}

#[test]
fn sibling_prefix_is_not_a_false_match() {
    // `/a/b/content` must NOT match a vault at `/a/b/content-archive`.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("content");
    let decoy = format!("{}-archive", root.to_str().unwrap());
    let cfg = write_config(tmp.path(), &[&decoy]);
    assert!(!registration_in_files(&root, &[cfg]).registered);
}

#[test]
fn missing_config_reports_not_found() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("memory_tree/content");
    let missing = tmp.path().join("does-not-exist.json");
    let got = registration_in_files(&root, &[missing]);
    assert_eq!(
        got,
        VaultRegistration {
            registered: false,
            config_found: false
        }
    );
}

#[test]
fn malformed_config_is_skipped_not_fatal() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("memory_tree/content");
    let bad = tmp.path().join("obsidian.json");
    std::fs::write(&bad, b"{ this is not json ").unwrap();
    // config_found is true (we read it) but parse fails → not registered.
    let got = registration_in_files(&root, &[bad]);
    assert_eq!(
        got,
        VaultRegistration {
            registered: false,
            config_found: true
        }
    );
}

#[test]
fn second_candidate_wins_when_first_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("memory_tree/content");
    let missing = tmp.path().join("nope.json");
    let real = write_config(tmp.path(), &[root.to_str().unwrap()]);
    assert!(registration_in_files(&root, &[missing, real]).registered);
}

/// The override arm is the host's own reason for keeping this code: a user
/// with a non-standard Obsidian install points at their config dir and the
/// probe has to look there *first*. Both spellings are accepted because users
/// cannot tell whether the path should end in `obsidian/`.
#[test]
fn extra_config_dir_is_probed_before_the_standard_locations() {
    let tmp = tempfile::tempdir().unwrap();
    let files = candidate_config_files(Some(tmp.path()));
    assert_eq!(files[0], tmp.path().join("obsidian.json"));
    assert_eq!(files[1], tmp.path().join("obsidian").join("obsidian.json"));
}

/// No override means no override-derived candidates — the probe must not
/// invent a relative `./obsidian.json`, which would make the answer depend on
/// the process working directory.
#[test]
fn no_extra_config_dir_adds_no_relative_candidate() {
    let files = candidate_config_files(None);
    assert!(
        files.iter().all(|p| p.is_absolute()),
        "every candidate must be absolute; got {files:?}"
    );
}
