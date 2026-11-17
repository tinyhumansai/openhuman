use super::*;

#[cfg(feature = "security-module")]
#[tokio::test]
async fn a_file_in_an_earlier_files_folder_can_still_be_revised() {
    let _floor = crate::modules::security::gap::install_path_floor_port(
        crate::modules::security::gap::ScriptedPathFloorPort::new([], vec![]),
    );
    let (workspace, files) = temp_dirs();
    let earlier = files.parent().unwrap().join("OldFiles");
    std::fs::create_dir_all(&earlier).unwrap();
    let old = earlier.join("deck.pptx");
    std::fs::write(&old, b"v1").unwrap();
    // The files folder moved; the earlier one stays trusted.
    let roots = FileRoots::new(&files).with_trusted([earlier.clone()]);
    let ctx = crate::core::runtime::DispatchCtx::process_default();
    let artifacts = artifacts_service(&workspace, roots, None, None, &ctx);
    let id = artifacts
        .create(
            ArtifactKind::File,
            "Deck",
            Content::File(old.display().to_string()),
            &Author::agent("agent"),
            None,
        )
        .await
        .unwrap()
        .record
        .id;
    std::fs::write(&old, b"v2").unwrap();
    let saved = artifacts
        .revise(
            &id,
            Content::File(old.display().to_string()),
            &Author::agent("agent"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(saved.version, 2);
}

// The refusal is TinySecurity's protected-path floor: a build without the
// security module has security disabled and no floor (`security::switch`).
#[cfg(feature = "security-module")]
#[tokio::test]
async fn a_protected_file_inside_the_working_folder_is_refused() {
    let (workspace, files) = temp_dirs();
    let action = workspace.parent().unwrap().join("home");
    let ssh = action.join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::write(ssh.join("id_ed25519"), b"PRIVATE").unwrap();
    let ctx = crate::core::runtime::DispatchCtx::process_default();
    let artifacts = artifacts_service(
        &workspace,
        FileRoots::new(&files),
        Some(&action),
        None,
        &ctx,
    );

    let err = artifacts
        .create(
            ArtifactKind::File,
            "Key",
            Content::File(".ssh/id_ed25519".into()),
            &Author::agent("agent"),
            None,
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("protected location"), "{err}");
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn the_guarded_roots_refuse_to_read_a_protected_file() {
    let (workspace, _files) = temp_dirs();
    let home = workspace.parent().unwrap().join("home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/id_ed25519"), b"PRIVATE").unwrap();
    std::fs::write(home.join("notes.txt"), b"hello").unwrap();
    let key = home.join(".ssh/id_ed25519").canonicalize().unwrap();
    let _floor = crate::modules::security::gap::install_path_floor_port(
        crate::modules::security::gap::ScriptedPathFloorPort::new([key], vec![]),
    );
    let roots = guarded_roots(vec![home]);

    let err = roots.read(".ssh/id_ed25519").await.unwrap_err();
    assert!(err.to_string().contains("protected location"), "{err}");
    let (_, bytes) = roots.read("notes.txt").await.unwrap();
    assert_eq!(bytes, b"hello");
}

#[test]
fn a_working_folder_that_contains_the_workspace_is_not_trusted() {
    let (workspace, files) = temp_dirs();
    let home = workspace.parent().unwrap().to_path_buf();
    let roots = file_roots(&workspace, &FileRoots::new(&files), Some(&home));
    assert!(!roots.contains(&home), "{roots:?}");
    assert!(roots.contains(&files));
    // A working folder beside the workspace stays the first root.
    let beside = home.join("projects");
    let roots = file_roots(&workspace, &FileRoots::new(&files), Some(&beside));
    assert_eq!(roots.first(), Some(&beside));
}

#[test]
fn a_stored_file_key_resolves_under_the_version_files_folder_and_an_absolute_one_stays() {
    let workspace = Path::new("/ws");
    let kept = tiny_artifacts::FileRef {
        key: "artifacts/files/a1/v2.pptx".into(),
        size_bytes: 1,
        media_type: None,
    };
    assert_eq!(
        stored_file_path(workspace, &kept),
        version_files_dir(workspace).join("artifacts/files/a1/v2.pptx")
    );
    let legacy = tiny_artifacts::FileRef {
        key: "/elsewhere/old.pptx".into(),
        ..kept
    };
    assert_eq!(
        stored_file_path(workspace, &legacy),
        PathBuf::from("/elsewhere/old.pptx")
    );
}
