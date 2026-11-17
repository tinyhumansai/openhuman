//! A signed-in profile's existing v1 `storage.db` survives the desktop host
//! boot migration and remains readable through the domain owners.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tinystoragedrivers::secrets::DerivedKeys;
use zeroize::Zeroizing;

const TOKEN: &str = "legacy-profile-boot-e2e-token";
const FIXTURE: &[u8] = include_bytes!(
    "../crates/openhuman-core/src/storage/import/fixtures/legacy-v1-profile/storage.db"
);
const FIXTURE_PATH: &str =
    "../../crates/openhuman-core/src/storage/import/fixtures/legacy-v1-profile/storage.db";

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn start(home: &Path) -> (Server, String) {
    let port = free_port();
    let server = Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .args(["serve", "--port", &port.to_string()])
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("OPENHUMAN_CORE_TOKEN", TOKEN)
        .env("OPENHUMAN_KEYRING_MASTER_KEY", "22".repeat(32))
        .env("OPENHUMAN_MODULE_SOURCE", "linked")
        .env_remove("OPENHUMAN_KEYRING_MASTER_KEY_FILE")
        .env_remove("OPENHUMAN_WORKSPACE")
        .env_remove("OPENHUMAN_STORAGE_URL")
        .env_remove("OPENHUMAN_MODE")
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(home.join("core.stderr.log")).unwrap())
        .spawn()
        .expect("spawn the desktop core host");
    let mut server = Server(server);
    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::blocking::Client::new();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if client
            .get(format!("{base}/health"))
            .send()
            .is_ok_and(|response| response.status().is_success())
        {
            return (server, base);
        }
        if let Ok(Some(status)) = server.0.try_wait() {
            panic!("desktop core host exited before ready: {status}");
        }
        assert!(
            Instant::now() < deadline,
            "desktop core host never became healthy"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn rpc_with_params(base: &str, method: &str, params: Value) -> Value {
    let response: Value = reqwest::blocking::Client::new()
        .post(format!("{base}/rpc"))
        .bearer_auth(TOKEN)
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        .send()
        .unwrap_or_else(|error| panic!("POST {method}: {error}"))
        .json()
        .unwrap_or_else(|error| panic!("decode {method}: {error}"));
    assert!(response.get("error").is_none(), "{method}: {response}");
    response["result"].clone()
}

fn seed_existing_profile(home: &Path) -> (PathBuf, PathBuf) {
    let openhuman = home.join(".openhuman");
    let profile = openhuman.join("users").join("u1");
    let workspace = profile.join("workspace");
    std::fs::create_dir_all(workspace.join("storage")).unwrap();
    std::fs::write(openhuman.join("active_user.toml"), "user_id = 'u1'\n").unwrap();
    std::fs::write(
        profile.join("config.toml"),
        "[storage]\nauto_migrate_layout = true\n",
    )
    .unwrap();
    let source = workspace.join("storage/storage.db");
    std::fs::write(&source, FIXTURE).unwrap();
    (profile, source)
}

fn fixture_sha256() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_PATH);
    Sha256::digest(std::fs::read(path).expect("read checked-in v1 fixture")).to_vec()
}

#[test]
fn desktop_boot_reads_a_preserved_existing_v1_profile_database() {
    let fixture_hash = fixture_sha256();
    let home = tempfile::tempdir().unwrap();
    let (profile, source) = seed_existing_profile(home.path());
    let (server, base) = start(home.path());
    let credential = rpc_with_params(
        &base,
        "openhuman.auth_set_credential",
        json!({
            "token": "fixture-session-token",
            "kind": "session",
            "userId": "u1",
            "user": {"id": "u1", "name": "Fixture User"}
        }),
    );
    assert!(
        credential.to_string().contains("u1"),
        "profile credential was not installed: {credential}"
    );
    assert!(
        profile.join("storage.db").is_file(),
        "boot migrated the active profile; profile entries: {:?}; workspace entries: {:?}; logs: {}",
        std::fs::read_dir(&profile)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>(),
        std::fs::read_dir(profile.join("workspace"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>(),
        std::fs::read_to_string(home.path().join("core.stderr.log")).unwrap_or_default()
    );
    assert!(
        !source.exists(),
        "migration should archive the old workspace storage path"
    );
    let archive_dir = home.path().join(".openhuman/backups");
    let archive = std::fs::read_dir(&archive_dir)
        .unwrap_or_else(|error| {
            panic!(
                "migration archive missing at {}: {error}",
                archive_dir.display()
            )
        })
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|extension| extension == "zst"))
        .expect("layout migration should archive the v1 workspace");
    let restored = tempfile::tempdir().unwrap();
    let manifest = openhuman_core::storage::layout::archive::restore(&archive, restored.path())
        .expect("migration archive should verify and restore");
    assert!(
        manifest
            .files
            .iter()
            .any(|file| { file.path == "workspace/storage/storage.db" && file.size > 0 }),
        "archive manifest must record the historical storage.db: {:?}",
        manifest
            .files
            .iter()
            .map(|file| (&file.path, file.size))
            .collect::<Vec<_>>()
    );
    let archived_db = restored.path().join("workspace/storage/storage.db");
    assert!(
        archived_db.is_file(),
        "archive must contain the migrated historical database"
    );
    let archive_url = format!("sqlite:{}", archived_db.display());
    openhuman_core::storage::block_on(
        async move { openhuman_core::storage::open(&archive_url).await },
    )
    .expect("archived historical database must remain readable");
    // The profile's V2 workspace resolves its default backend to the exact
    // storage.db that desktop boot just opened.
    let workspace = profile.join("files");
    let mut config = openhuman_core::config::Config::default();
    config.config_path = profile.join("config.toml");
    config.workspace_dir = workspace.clone();
    config.storage.url = Some(format!("sqlite:{}", profile.join("storage.db").display()));
    let owner_workspace = workspace.clone();
    std::thread::spawn(move || {
        let storage_url = config.storage.url.as_deref().unwrap().to_owned();
        let owner_runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        owner_runtime.block_on(async move {
            let runtime = openhuman_core::core::runtime::CoreBuilder::new(
                openhuman_core::core::types::HostKind::TauriShell,
            )
            .boot_mode(openhuman_core::core::runtime::CoreBootMode::Inspection)
            .token(openhuman_core::core::runtime::TokenSource::Fixed(
                std::sync::Arc::new("legacy-profile-inspection-token".to_owned()),
            ))
            .services(openhuman_core::core::runtime::ServiceSet::none())
            .config(config.clone())
            .build()
            .await
            .unwrap();
            let open_url = storage_url.clone();
            let owner_backend = openhuman_core::storage::block_on(async move {
                openhuman_core::storage::open(&open_url).await
            })
            .unwrap();
            openhuman_core::storage::install_for_url(&storage_url, owner_backend);
            openhuman_core::core::runtime::CoreContext::scope(
                std::sync::Arc::clone(runtime.context()),
                async {
                    let dispatch = openhuman_core::core::runtime::DispatchCtx::ambient();
                    let thread_store = openhuman_core::threads::store::ConversationStore::new(
                        dispatch,
                        owner_workspace,
                    );
                    let approvals =
                        openhuman_core::security::approval::rpc::approval_list_recent_decisions(
                            Some(10),
                        )
                        .await
                        .expect("the approval history is imported through TinySecurity");
                    assert_eq!(
                        approvals.value.len(),
                        1,
                        "TinySecurity did not import the v1 approval history"
                    );
                    assert_eq!(approvals.value[0].request_id, "fixture-approval");
                    let flows = openhuman_core::flows::ops::flows_list(&config)
                        .await
                        .unwrap();
                    assert!(
                        serde_json::to_string(&flows.value)
                            .unwrap()
                            .contains("fixture-flow"),
                        "flow owner did not read the v1 database record: {flows:?}"
                    );
                    openhuman_core::flows::ops::flows_delete(&config, "fixture-flow")
                        .await
                        .unwrap();
                    let deleted = openhuman_core::flows::ops::flows_list(&config)
                        .await
                        .unwrap();
                    assert!(
                        !serde_json::to_string(&deleted.value)
                            .unwrap()
                            .contains("fixture-flow"),
                        "flow owner did not persist the fixture deletion: {deleted:?}"
                    );
                    let listed = thread_store.list_threads().unwrap();
                    assert_eq!(
                        listed.len(),
                        1,
                        "thread owner did not read the v1 database thread: {listed:?}"
                    );
                    assert_eq!(
                        thread_store
                            .transcript_key("fixture-thread")
                            .unwrap()
                            .as_deref(),
                        Some("fixture-transcript-key"),
                        "thread owner did not read the v1 database transcript key"
                    );
                    let messages = thread_store.get_messages("fixture-thread").unwrap();
                    assert_eq!(
                        messages.len(),
                        1,
                        "thread owner did not read the historical message"
                    );
                    assert_eq!(messages[0].id, "fixture-message");
                    assert_eq!(messages[0].content, "historical profile message");
                },
            )
            .await;
        });
    })
    .join()
    .unwrap();

    // Read the confidential record with TinyStorage's document-secret owner
    // API after the desktop boot has installed and migrated the profile DB.
    let url = format!("sqlite:{}", profile.join("storage.db").display());
    let backend =
        openhuman_core::storage::block_on(async move { openhuman_core::storage::open(&url).await })
            .unwrap();
    let scope = openhuman_core::storage::Scope::local();
    let scoped = backend.for_scope(&scope).unwrap();
    let secrets = openhuman_core::storage::secrets::over(
        &scoped,
        scope,
        std::sync::Arc::new(DerivedKeys::new(Zeroizing::new([0x5a; 32]))),
    );
    let secret = openhuman_core::storage::secrets::get_blocking(&secrets, "fixture-secret")
        .unwrap()
        .unwrap();
    assert_eq!(secret.as_slice(), b"historical-secret-value");
    assert_eq!(
        fixture_sha256(),
        fixture_hash,
        "boot modified the checked-in source fixture"
    );

    // A second desktop boot must not replay the archived v1 database over an
    // owner deletion made after migration.
    drop(server);
    let (rebooted, _) = start(home.path());
    let workspace = profile.join("files");
    let mut config = openhuman_core::config::Config::default();
    config.config_path = profile.join("config.toml");
    config.workspace_dir = workspace;
    config.storage.url = Some(format!("sqlite:{}", profile.join("storage.db").display()));
    std::thread::spawn(move || {
        openhuman_core::storage::clear();
        let storage_url = config.storage.url.as_deref().unwrap().to_owned();
        let owner_runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        owner_runtime.block_on(async move {
            let runtime = openhuman_core::core::runtime::CoreBuilder::new(
                openhuman_core::core::types::HostKind::TauriShell,
            )
            .boot_mode(openhuman_core::core::runtime::CoreBootMode::Inspection)
            .token(openhuman_core::core::runtime::TokenSource::Fixed(
                std::sync::Arc::new("legacy-profile-reboot-token".to_owned()),
            ))
            .services(openhuman_core::core::runtime::ServiceSet::none())
            .config(config.clone())
            .build()
            .await
            .unwrap();
            let open_url = storage_url.clone();
            let owner_backend = openhuman_core::storage::block_on(async move {
                openhuman_core::storage::open(&open_url).await
            })
            .unwrap();
            openhuman_core::storage::install_for_url(&storage_url, owner_backend);
            openhuman_core::core::runtime::CoreContext::scope(
                std::sync::Arc::clone(runtime.context()),
                async {
                    let flows = openhuman_core::flows::ops::flows_list(&config)
                        .await
                        .unwrap();
                    assert!(
                        !serde_json::to_string(&flows.value)
                            .unwrap()
                            .contains("fixture-flow"),
                        "desktop reboot resurrected the deleted flow: {flows:?}"
                    );
                },
            )
            .await;
        });
    })
    .join()
    .unwrap();
    drop(rebooted);
    assert_eq!(
        fixture_sha256(),
        fixture_hash,
        "reboot modified the checked-in source fixture"
    );
}
