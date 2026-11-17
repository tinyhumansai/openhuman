use super::*;

#[test]
fn reads_completed_versions_from_a_read_only_database_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("storage.db");
    let connection = rusqlite::Connection::open(&db).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE docs (
                scope TEXT NOT NULL,
                coll TEXT NOT NULL,
                id TEXT NOT NULL,
                updated_ms INTEGER NOT NULL,
                deleted INTEGER NOT NULL DEFAULT 0,
                body TEXT NOT NULL,
                PRIMARY KEY (scope, coll, id)
            ) WITHOUT ROWID;",
        )
        .unwrap();
    for (id, owner, to, done, updated_ms) in [
        ("a", "threads", 1, true, 100),
        ("b", "threads", 3, true, 300),
        ("c", "profiles", 2, true, 200),
        ("d", "incomplete", 9, false, 900),
        ("e", "zero", 0, true, 1000),
    ] {
        let body = serde_json::json!({
            "owner": owner,
            "step": "step",
            "from": 0,
            "to": to,
            "cursor": null,
            "written": 1,
            "done": done,
        });
        connection
            .execute(
                "INSERT INTO docs (scope, coll, id, updated_ms, body) \
                 VALUES ('local', 'tsd.migrations', ?1, ?2, ?3)",
                rusqlite::params![id, updated_ms, body.to_string()],
            )
            .unwrap();
    }
    let compressed = serde_json::json!({
        "owner": "compressed",
        "step": "large-step",
        "from": 0,
        "to": 4,
        "cursor": null,
        "written": 1,
        "done": true,
        "padding": "x".repeat(6000),
    })
    .to_string();
    let mut blob = vec![1];
    blob.extend(zstd::encode_all(compressed.as_bytes(), 3).unwrap());
    connection
        .execute(
            "INSERT INTO docs (scope, coll, id, updated_ms, body) \
             VALUES ('local', 'tsd.migrations', 'compressed', 400, ?1)",
            [rusqlite::types::Value::Blob(blob)],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO docs (scope, coll, id, updated_ms, body) \
             VALUES ('cluster', 'tsd.migrations', 'other-scope', 5000, ?1)",
            [r#"{"owner":"ignored","to":7,"done":true}"#],
        )
        .unwrap();
    drop(connection);
    let original = std::fs::read(&db).unwrap();

    let (versions, last) = read(&db).unwrap();

    assert_eq!(
        versions,
        vec![
            ("compressed".into(), 4),
            ("profiles".into(), 2),
            ("threads".into(), 3)
        ]
    );
    assert_eq!(last, Some(400));
    assert_eq!(std::fs::read(&db).unwrap(), original);
}
