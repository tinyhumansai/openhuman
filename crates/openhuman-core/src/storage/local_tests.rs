use super::*;
use crate::storage::documents::Repo;
use serde_json::json;
use tinystoragedrivers::{CollectionSpec, Precondition};

fn things() -> Vec<CollectionSpec> {
    vec![CollectionSpec::new("things")]
}

fn put_thing(opened: &Opened, id: &'static str, n: i64) {
    let repo = Repo::on(&opened.backend, &opened.scope, "things", things).unwrap();
    repo.run(move |docs| async move {
        docs.put("things", id, json!({ "n": n }), Precondition::None)
            .await
            .map(|_| ())
    })
    .unwrap();
}

fn get_thing(opened: &Opened, id: &'static str) -> Option<i64> {
    let repo = Repo::on(&opened.backend, &opened.scope, "things", things).unwrap();
    repo.run(move |docs| async move { docs.get("things", id).await })
        .unwrap()
        .map(|stored| stored.doc["n"].as_i64().unwrap())
}

#[test]
fn an_uninstalled_call_opens_the_default_for_its_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let opened = open("things", dir.path()).unwrap();
    assert_eq!(opened.backend.driver(), "sqlite");
    put_thing(&opened, "a", 1);
    assert!(
        dir.path().join("storage").is_dir(),
        "the default lives in <workspace>/storage"
    );
    // The same workspace reuses the one backend.
    let again = open("things", dir.path()).unwrap();
    assert!(Arc::ptr_eq(&opened.backend, &again.backend));
    assert_eq!(get_thing(&again, "a"), Some(1));
}

#[test]
fn workspaces_do_not_share_a_default() {
    let one = tempfile::tempdir().unwrap();
    let two = tempfile::tempdir().unwrap();
    let a = open("things", one.path()).unwrap();
    let b = open("things", two.path()).unwrap();
    put_thing(&a, "only-in-one", 7);
    assert_eq!(get_thing(&b, "only-in-one"), None);
}

#[test]
fn forgetting_a_workspace_reopens_the_same_data() {
    let dir = tempfile::tempdir().unwrap();
    let first = open("things", dir.path()).unwrap();
    put_thing(&first, "kept", 3);
    forget(dir.path());
    let second = open("things", dir.path()).unwrap();
    assert!(!Arc::ptr_eq(&first.backend, &second.backend));
    assert_eq!(
        get_thing(&second, "kept"),
        Some(3),
        "a restart keeps the data"
    );
}
