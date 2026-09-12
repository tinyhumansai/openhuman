use super::*;

#[test]
fn workspace_policy_id_round_trips() {
    let encoded = workspace_policy_id("alice");
    assert_eq!(encoded, "openhuman.profile:alice");
    assert_eq!(profile_id_from_policy_id(&encoded), Some("alice"));
}

#[test]
fn profile_id_from_policy_id_rejects_non_profile_ids() {
    // Worktree-isolation / test ids and empty strings are not profiles.
    assert_eq!(profile_id_from_policy_id("test-worktree"), None);
    assert_eq!(profile_id_from_policy_id(""), None);
    assert_eq!(profile_id_from_policy_id("openhuman.profile:"), None);
    assert_eq!(
        profile_id_from_policy_id("openhuman.profile:bob"),
        Some("bob")
    );
}

// ── Cross-profile classifier (1b) ─────────────────────────────────────

fn profiles_layout() -> (tempfile::TempDir, PathBuf) {
    let action = tempfile::tempdir().expect("action tempdir");
    let profiles = action.path().join("profiles");
    for id in ["alice", "bob"] {
        std::fs::create_dir_all(profiles.join(id)).unwrap();
    }
    let action_dir = action.path().to_path_buf();
    (action, action_dir)
}

#[test]
fn same_profile_target_is_allowed() {
    let (_g, action) = profiles_layout();
    let target = action.join("profiles").join("alice").join("notes.txt");
    assert_eq!(
        classify_cross_profile_target(&action, "alice", &target),
        CrossProfileDecision::Allow
    );
}

#[test]
fn other_profile_target_is_blocked() {
    let (_g, action) = profiles_layout();
    let target = action.join("profiles").join("bob").join("secret.txt");
    assert_eq!(
        classify_cross_profile_target(&action, "alice", &target),
        CrossProfileDecision::Block {
            other_id: "bob".into()
        }
    );
}

#[test]
fn target_outside_profiles_root_is_allowed() {
    let (_g, action) = profiles_layout();
    // A plain file under action_dir (the shared workspace) — not under
    // profiles/ at all.
    let target = action.join("scratch.txt");
    assert_eq!(
        classify_cross_profile_target(&action, "alice", &target),
        CrossProfileDecision::Allow
    );
}

#[test]
fn nonexistent_sibling_target_is_blocked_via_ancestor() {
    let (_g, action) = profiles_layout();
    // File does not exist yet, but its parent (profiles/bob) does → the
    // deepest-existing-ancestor resolution still classifies it as bob's.
    let target = action
        .join("profiles")
        .join("bob")
        .join("nested")
        .join("fresh.txt");
    assert_eq!(
        classify_cross_profile_target(&action, "alice", &target),
        CrossProfileDecision::Block {
            other_id: "bob".into()
        }
    );
}

#[test]
fn relative_traversal_into_sibling_is_blocked() {
    let (_g, action) = profiles_layout();
    // A relative `../bob/x` composed from the active profile's own dir.
    let target = action
        .join("profiles")
        .join("alice")
        .join("..")
        .join("bob")
        .join("x.txt");
    assert_eq!(
        classify_cross_profile_target(&action, "alice", &target),
        CrossProfileDecision::Block {
            other_id: "bob".into()
        }
    );
}

#[cfg(unix)]
#[test]
fn symlink_into_sibling_profile_is_blocked() {
    use std::os::unix::fs::symlink;
    let (_g, action) = profiles_layout();
    // Inside alice, a symlink `link -> ../bob`. Writing `link/hijack.txt`
    // must resolve to bob's dir and block.
    let alice = action.join("profiles").join("alice");
    let bob = action.join("profiles").join("bob");
    symlink(&bob, alice.join("link")).unwrap();
    let target = alice.join("link").join("hijack.txt");
    assert_eq!(
        classify_cross_profile_target(&action, "alice", &target),
        CrossProfileDecision::Block {
            other_id: "bob".into()
        }
    );
}

#[test]
fn profiles_root_itself_is_blocked() {
    // Mutating the shared root can affect every sibling at once.
    let (_g, action) = profiles_layout();
    let target = action.join("profiles");
    assert_eq!(
        classify_cross_profile_target(&action, "alice", &target),
        CrossProfileDecision::Block {
            other_id: PROFILES_ROOT_SENTINEL.into()
        }
    );
}

// ── Shell command scan (1b) ───────────────────────────────────────────

#[test]
fn scan_command_allows_same_profile_and_bare_tokens() {
    let (_g, action) = profiles_layout();
    let cwd = action.join("profiles").join("alice");
    // Relative writes under cwd, plain words, and reads under cwd are fine.
    assert_eq!(
        scan_command_for_cross_profile("echo hi > notes.txt", &cwd, &action, "alice"),
        None
    );
    assert_eq!(
        scan_command_for_cross_profile("ls -la sub/dir", &cwd, &action, "alice"),
        None
    );
}

#[test]
fn scan_command_blocks_absolute_sibling_target() {
    let (_g, action) = profiles_layout();
    let cwd = action.join("profiles").join("alice");
    let sibling = action.join("profiles").join("bob").join("loot.txt");
    let command = format!("cat secret > {}", sibling.display());
    assert_eq!(
        scan_command_for_cross_profile(&command, &cwd, &action, "alice"),
        Some("bob".to_string())
    );
}

#[test]
fn scan_command_blocks_relative_traversal_into_sibling() {
    let (_g, action) = profiles_layout();
    let cwd = action.join("profiles").join("alice");
    assert_eq!(
        scan_command_for_cross_profile("cp x ../bob/y", &cwd, &action, "alice"),
        Some("bob".to_string())
    );
}

#[test]
fn scan_command_tracks_cd_before_sibling_write() {
    let (_g, action) = profiles_layout();
    let cwd = action.join("profiles").join("alice");
    assert_eq!(
        scan_command_for_cross_profile("cd .. && printf x > bob/loot.txt", &cwd, &action, "alice"),
        Some(PROFILES_ROOT_SENTINEL.to_string())
    );
}

#[test]
fn scan_command_tracks_cd_before_bare_sibling_operand() {
    let (_g, action) = profiles_layout();
    let cwd = action.join("profiles").join("alice");
    assert_eq!(
        scan_command_for_cross_profile("cd ..; rm -rf bob", &cwd, &action, "alice"),
        Some(PROFILES_ROOT_SENTINEL.to_string())
    );
}

#[test]
fn scan_command_blocks_parent_profiles_root_operand() {
    let (_g, action) = profiles_layout();
    let cwd = action.join("profiles").join("alice");
    assert_eq!(
        scan_command_for_cross_profile("rm -rf ..", &cwd, &action, "alice"),
        Some(PROFILES_ROOT_SENTINEL.to_string())
    );
}

#[test]
fn scan_command_blocks_bare_profiles_root_from_action_dir() {
    let (_g, action) = profiles_layout();
    assert_eq!(
        scan_command_for_cross_profile("rm -rf profiles", &action, &action, "alice"),
        Some(PROFILES_ROOT_SENTINEL.to_string())
    );
}

#[test]
fn scan_command_tracks_chained_bare_cd_into_sibling() {
    let (_g, action) = profiles_layout();
    let cwd = action.join("profiles").join("alice");
    assert_eq!(
        scan_command_for_cross_profile(
            "cd ..; cd bob; printf x > loot.txt",
            &cwd,
            &action,
            "alice"
        ),
        Some(PROFILES_ROOT_SENTINEL.to_string())
    );
}

#[test]
fn scan_command_blocks_path_embedded_in_quoted_interpreter_arg() {
    // The path is buried inside a python -c program string. Splitting on
    // quotes/parens/commas isolates `../bob/loot.txt` so the simple embedded
    // case is still caught.
    let (_g, action) = profiles_layout();
    let cwd = action.join("profiles").join("alice");
    let command = r#"python -c 'open("../bob/loot.txt","w").write("x")'"#;
    assert_eq!(
        scan_command_for_cross_profile(command, &cwd, &action, "alice"),
        Some("bob".to_string())
    );
}

#[test]
fn scan_command_blocks_flag_equals_sibling_path() {
    // A `--flag=../bob/…` form: splitting on `=` isolates the path token.
    let (_g, action) = profiles_layout();
    let cwd = action.join("profiles").join("alice");
    assert_eq!(
        scan_command_for_cross_profile(
            "tar --directory=../bob/x -cf a.tar .",
            &cwd,
            &action,
            "alice"
        ),
        Some("bob".to_string())
    );
}

#[test]
fn scan_command_documents_variable_expansion_gap() {
    // Documented best-effort limitation: a shell variable that expands to a
    // sibling path at runtime is not statically resolvable, so the scan does
    // not catch it. The hard boundary for this is an OS sandbox (follow-up);
    // this test pins the known gap so it's a conscious contract, not a
    // surprise regression.
    let (_g, action) = profiles_layout();
    let cwd = action.join("profiles").join("alice");
    assert_eq!(
        scan_command_for_cross_profile("cp x $TARGET_DIR/y", &cwd, &action, "alice"),
        None
    );
}
