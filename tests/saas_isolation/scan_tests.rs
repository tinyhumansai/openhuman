use super::*;

#[test]
fn shared_storage_bytes_are_checked_through_tenant_surfaces() {
    let root = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let profile = root.path().join("users/alice/workspace");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(profile.join("profile.json"), "CANARY-alice-profile").unwrap();

    let database = home.path().join("saas-storage.db");
    let all_tenants = "CANARY-alice CANARY-bob CANARY-carol";
    std::fs::write(&database, all_tenants).unwrap();
    std::fs::write(format!("{}-wal", database.display()), all_tenants).unwrap();

    let (violations, own) = scan_files(root.path(), home.path(), &database);

    assert!(violations.is_empty(), "{violations:#?}");
    assert_eq!(own[0], 1, "the profile file scan still audits owned data");
}
