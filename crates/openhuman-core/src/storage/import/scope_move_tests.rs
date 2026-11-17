use super::*;
use crate::storage::rusqlite::Connection;
use std::io::Read;

#[test]
fn inventory_reads_all_legacy_families_without_changing_the_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE _tsd_scope_admissions(scope TEXT PRIMARY KEY, generation INTEGER, operation TEXT);
         INSERT INTO _tsd_scope_admissions VALUES('empty-agent',1,'write');
         CREATE TABLE docs(scope TEXT, coll TEXT, id TEXT, version INTEGER, schema INTEGER,
             created_ms INTEGER, updated_ms INTEGER, expires_ms INTEGER, deleted INTEGER, body TEXT,
             PRIMARY KEY(scope, coll, id));
         INSERT INTO docs VALUES('alice~agent','notes','n1',1,4,100,200,900,0,'{\"x\":1}');
         INSERT INTO docs VALUES('alice~agent','notes','gone',1,4,100,200,NULL,1,'{}');
         CREATE TABLE stream_heads(scope TEXT, name TEXT, base INTEGER, len INTEGER,
             sealed INTEGER, sealed_ms INTEGER, last_append_ms INTEGER, PRIMARY KEY(scope,name));
         INSERT INTO stream_heads VALUES('alice~agent','events',3,2,1,700,600);
         CREATE TABLE streams(scope TEXT, name TEXT, seq INTEGER, body TEXT,
             PRIMARY KEY(scope,name,seq));
         INSERT INTO streams VALUES('alice~agent','events',3,'{\"n\":1}');
         INSERT INTO streams VALUES('alice~agent','events',4,'{\"n\":2}');
         CREATE TABLE blobs(scope TEXT, key TEXT, content_type TEXT, size INTEGER, sha256 BLOB,
             expires_ms INTEGER, bytes BLOB, PRIMARY KEY(scope,key));
         INSERT INTO blobs VALUES('alice~agent','b1','application/octet-stream',3,x'010203',NULL,x'010203');",
    )
    .unwrap();
    drop(conn);
    let before = sha256(&path);

    let inventory = LegacyScopeInventory::read(&path).unwrap();

    assert_eq!(sha256(&path), before);
    assert_eq!(inventory.documents.len(), 1);
    assert!(inventory.scopes.contains(&"alice~agent".to_owned()));
    assert!(inventory.scopes.contains(&"empty-agent".to_owned()));
    assert_eq!(inventory.documents[0].meta.schema, 4);
    assert_eq!(inventory.documents[0].meta.created_ms, 100);
    assert_eq!(inventory.documents[0].meta.updated_ms, 200);
    assert_eq!(inventory.documents[0].meta.expires_ms, Some(900));
    assert_eq!(inventory.streams.len(), 1);
    assert_eq!(inventory.streams[0].base, 3);
    assert!(inventory.streams[0].sealed);
    assert_eq!(inventory.streams[0].sealed_ms, Some(700));
    assert_eq!(inventory.streams[0].entries[1].0, 4);
    assert_eq!(inventory.blobs[0].bytes, [1, 2, 3]);
}

#[test]
fn inventory_rejects_stream_gaps_instead_of_silently_resequencing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE streams(scope TEXT, name TEXT, seq INTEGER, body TEXT);
         INSERT INTO streams VALUES('agent','events',0,'{}');
         INSERT INTO streams VALUES('agent','events',2,'{}');",
    )
    .unwrap();
    drop(conn);

    let error = LegacyScopeInventory::read(&path).unwrap_err();
    assert!(error.to_string().contains("not contiguous"));
}

fn sha256(path: &Path) -> Vec<u8> {
    let mut file = std::fs::File::open(path).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    use sha2::Digest;
    sha2::Sha256::digest(bytes).to_vec()
}
