use super::*;
use crate::config::schema::user_openhuman_dir;
use std::path::PathBuf;

#[test]
fn every_path_sits_under_the_profile_directory() {
    let layout = ProfileLayout::new(Path::new("/srv/oh"), "alice");
    assert_eq!(layout.dir, Path::new("/srv/oh/users/alice"));
    assert_eq!(
        layout.meta_path,
        Path::new("/srv/oh/users/alice/profile.toml")
    );
    assert_eq!(
        layout.config_path,
        Path::new("/srv/oh/users/alice/config.toml")
    );
    assert_eq!(
        layout.workspace_dir,
        Path::new("/srv/oh/users/alice/workspace")
    );
    assert_eq!(layout.sandbox_dir, Path::new("/srv/oh/users/alice/sandbox"));
}

#[test]
fn users_dir_is_the_parent_of_every_profile() {
    let root = Path::new("/srv/oh");
    assert_eq!(users_dir(root), Path::new("/srv/oh/users"));
    assert_eq!(
        ProfileLayout::new(root, "bob").dir.parent(),
        Some(users_dir(root).as_path())
    );
}

#[test]
fn the_layout_matches_the_desktop_user_directory() {
    let root = Path::new("/home/u/.openhuman");
    assert_eq!(
        ProfileLayout::new(root, "65f1c0ffee0123456789abcd").dir,
        super::super::user_openhuman_dir(root, "65f1c0ffee0123456789abcd")
    );
}

fn tree(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
        entries.sort_by_key(|e| e.path());
        for entry in entries {
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap().display().to_string();
            if path.is_dir() {
                out.push(format!("{rel}/"));
                walk(&path, root, out);
            } else {
                out.push(format!("{rel}={}", std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn v1_and_v2_resolve_each_accessor_under_the_same_profile_directory() {
    let dir = PathBuf::from("/srv/oh/users/alice");
    let v1 = ProfileLayout::with_version(dir.clone(), LayoutVersion::V1);
    let v2 = ProfileLayout::with_version(dir.clone(), LayoutVersion::V2);

    // Shared by both versions.
    assert_eq!(v1.dir, v2.dir);
    assert_eq!(v1.config_path, v2.config_path);
    assert_eq!(v1.meta_path, v2.meta_path);
    assert_eq!(v1.sandbox_dir, v2.sandbox_dir);
    assert_eq!(v1.storage_db, dir.join("storage.db"));
    assert_eq!(v2.storage_db, dir.join("storage.db"));
    assert_eq!(v1.files_dir, dir.join("files"));
    assert_eq!(v2.files_dir, dir.join("files"));

    // The workspace is the one thing that moves.
    assert_eq!(v1.workspace_dir, dir.join("workspace"));
    assert_eq!(v2.workspace_dir, dir.join("files"));
    assert_eq!(v1.version, LayoutVersion::V1);
    assert_eq!(v2.version, LayoutVersion::V2);
}

#[test]
fn the_pure_constructors_stay_v1() {
    assert_eq!(
        ProfileLayout::new(Path::new("/srv/oh"), "alice").version,
        LayoutVersion::V1
    );
    assert_eq!(
        ProfileLayout::at(PathBuf::from("/srv/oh/users/alice")).workspace_dir,
        Path::new("/srv/oh/users/alice/workspace")
    );
}

#[test]
fn a_missing_or_empty_profile_directory_is_v2_and_content_without_a_marker_is_v1() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(
        LayoutVersion::detect(&tmp.path().join("absent")),
        LayoutVersion::V2
    );
    assert_eq!(LayoutVersion::detect(tmp.path()), LayoutVersion::V2);

    std::fs::write(tmp.path().join("config.toml"), "").unwrap();
    assert_eq!(LayoutVersion::detect(tmp.path()), LayoutVersion::V1);

    LayoutVersion::mark_v2(tmp.path()).unwrap();
    assert_eq!(
        LayoutVersion::detect(tmp.path()),
        LayoutVersion::V2,
        "the marker wins over loose files"
    );
}

#[test]
fn resolving_a_fresh_profile_writes_the_marker_so_it_stays_v2() {
    let tmp = tempfile::tempdir().unwrap();
    let layout = ProfileLayout::resolve(tmp.path(), "alice");
    assert_eq!(layout.version, LayoutVersion::V2);
    assert_eq!(layout.workspace_dir, layout.dir.join("files"));
    assert_eq!(
        std::fs::read_to_string(layout.dir.join(".layout"))
            .unwrap()
            .trim(),
        "2"
    );

    // A config file appearing beside the marker does not flip it to V1.
    std::fs::write(&layout.config_path, "").unwrap();
    assert_eq!(
        ProfileLayout::resolve(tmp.path(), "alice").version,
        LayoutVersion::V2
    );
}

#[test]
fn resolving_a_v1_profile_changes_nothing_on_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = user_openhuman_dir(tmp.path(), "alice");
    std::fs::create_dir_all(dir.join("workspace/storage")).unwrap();
    std::fs::write(dir.join("config.toml"), "api_url = \"x\"\n").unwrap();
    std::fs::write(dir.join("workspace/threads.json"), "[]").unwrap();
    let before = tree(tmp.path());

    let layout = ProfileLayout::resolve(tmp.path(), "alice");

    assert_eq!(layout.version, LayoutVersion::V1);
    assert_eq!(layout.workspace_dir, dir.join("workspace"));
    assert_eq!(tree(tmp.path()), before, "no marker, no new directory");
}

#[test]
fn of_workspace_recognises_only_a_marked_v2_files_directory() {
    let tmp = tempfile::tempdir().unwrap();
    let v2 = ProfileLayout::resolve(tmp.path(), "alice");
    let found = ProfileLayout::of_workspace(&v2.workspace_dir).expect("a v2 files dir");
    assert_eq!(found, v2);

    let v1_dir = user_openhuman_dir(tmp.path(), "bob");
    std::fs::create_dir_all(v1_dir.join("files")).unwrap();
    assert!(ProfileLayout::of_workspace(&v1_dir.join("files")).is_none());
    assert!(ProfileLayout::of_workspace(&v1_dir.join("workspace")).is_none());
}
