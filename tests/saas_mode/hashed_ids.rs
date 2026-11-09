//! `profile_ids = "hashed"` end to end, through the real binary: every
//! gateway user is served by profile `h-<32 hex>`, the same user always
//! resolves to the same profile (across a restart too), gateway requests
//! work under it, and the user id itself never reaches a path on disk, a
//! response or the core's log.

use std::path::{Path, PathBuf};

use super::*;
use openhuman_core::profiles::{ProfileId, ProfileIdMode};

/// One user id that would pass through unchanged in raw mode, and one that
/// would not.
const USERS: [&str; 2] = ["carol-0042", "Dave.Example+tenant@example.com"];

/// What must never show up anywhere: the user ids and their recognisable
/// parts, compared case-insensitively.
const NEEDLES: [&str; 3] = ["carol-0042", "dave.example", "tenant@example.com"];

/// Start a SaaS core with hashed profile ids and every log level captured
/// to `<tmp>/<name>.log`.
fn start_hashed(d: &Deployment, name: &str) -> (Server, String, PathBuf) {
    std::fs::write(
        &d.config,
        format!(
            "root = {:?}\nprofile_ids = \"hashed\"\nnode_id = \"hashed-profile-test\"\n",
            d.root.display().to_string()
        ),
    )
    .unwrap();
    let port = free_port();
    let log = d.tmp.path().join(format!("{name}.log"));
    let child = core_command(d, &["--port", &port.to_string()])
        .env("RUST_LOG", "debug")
        .env("BACKEND_URL", "http://127.0.0.1:9")
        .stdout(std::fs::File::create(&log).unwrap())
        .stderr(std::fs::File::create(d.tmp.path().join(format!("{name}.err"))).unwrap())
        .spawn()
        .expect("spawn openhuman-core");
    let mut server = Server(child);
    let base = format!("http://127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(120);
    while !client_for_tests()
        .get(format!("{base}/health"))
        .send()
        .is_ok_and(|r| r.status().is_success())
    {
        if let Ok(Some(status)) = server.0.try_wait() {
            panic!("SaaS core exited before serving: {status}");
        }
        assert!(Instant::now() < deadline, "SaaS core never became healthy");
        std::thread::sleep(Duration::from_millis(250));
    }
    (server, base, log)
}

fn client_for_tests() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

fn assert_free_of_user_ids(what: &str, text: &str) {
    let lower = text.to_lowercase();
    for needle in NEEDLES {
        assert!(
            !lower.contains(needle),
            "{what} carries the user id fragment {needle:?}"
        );
    }
}

/// Every path under `dir`, recursively.
fn all_paths(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        out.push(path.clone());
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            all_paths(&path, out);
        }
    }
}

/// Provision `user` through the operator plane; returns the profile id and
/// whether it was new.
fn provision_hashed(
    client: &reqwest::blocking::Client,
    base: &str,
    user: &str,
    seen: &mut String,
) -> (String, bool) {
    let (status, body) = rpc_with(
        client,
        base,
        Some(BEARER),
        "openhuman.profiles_provision",
        json!({ "user_id": user }),
    );
    assert_eq!(status, 200, "{body}");
    seen.push_str(&body.to_string());
    let result = body
        .pointer("/result/result")
        .or_else(|| body.get("result"))
        .cloned()
        .unwrap_or(Value::Null);
    let text = result.to_string();
    let profile = text
        .split("\"profile_id\":\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("no profile_id in {body}"))
        .to_string();
    (profile, text.contains("\"created\":true"))
}

#[test]
fn hashed_profile_ids_keep_user_ids_off_disk_out_of_responses_and_logs() {
    let d = deployment(true);
    let client = client_for_tests();
    let expected: Vec<String> = USERS
        .iter()
        .map(|user| {
            ProfileId::for_user(user, ProfileIdMode::Hashed)
                .unwrap()
                .to_string()
        })
        .collect();
    for id in &expected {
        let hex = id.strip_prefix("h-").expect("hashed ids start with h-");
        assert_eq!(hex.len(), 32, "{id}");
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()), "{id}");
    }
    let mut seen = String::new();
    let mut logs = Vec::new();

    {
        let (server, base, log) = start_hashed(&d, "first");
        logs.push(log);
        for (user, profile) in USERS.iter().zip(&expected) {
            let (id, created) = provision_hashed(&client, &base, user, &mut seen);
            assert_eq!(&id, profile, "provisioning maps the user to users/h-<hash>");
            assert!(created);
            let (again, created) = provision_hashed(&client, &base, user, &mut seen);
            assert_eq!(
                &again, profile,
                "the same user resolves to the same profile"
            );
            assert!(!created);

            let layout = d.root.join("users").join(profile);
            assert!(layout.join("workspace").is_dir(), "{}", layout.display());
            assert!(layout.join("sandbox").is_dir(), "{}", layout.display());

            let (_, body) = rpc_with(
                &client,
                &base,
                Some(BEARER),
                "openhuman.profiles_set_credential",
                json!({ "profile_id": profile, "kind": "session", "token": "hashed-e2e-session" }),
            );
            assert!(body.get("result").is_some(), "{body}");
            seen.push_str(&body.to_string());

            // The gateway serves the user under that profile.
            let call = |method: &str, params: Value| {
                user_rpc_with(&client, &base, BEARER, user, None, method, params)
            };
            let (status, body) = call("core.ping", json!({}));
            assert_eq!(status, 200, "{body}");
            seen.push_str(&body.to_string());
            let (status, body) = call(
                "openhuman.threads_upsert",
                json!({ "id": "hashed-thread", "title": "kept", "created_at": "2026-10-10T00:00:00Z" }),
            );
            assert_eq!(status, 200, "{body}");
            assert!(body.get("result").is_some(), "{body}");
            seen.push_str(&body.to_string());
            let (_, body) = call("openhuman.threads_list", json!({}));
            assert_eq!(
                thread_ids(&body),
                vec!["hashed-thread".to_string()],
                "{body}"
            );
            seen.push_str(&body.to_string());
        }
        let (_, body) = rpc(&client, &base, Some(BEARER), "openhuman.profiles_list");
        for profile in &expected {
            assert!(body.to_string().contains(profile.as_str()), "{body}");
        }
        assert!(
            body.to_string().contains("\"has_credential\":true"),
            "{body}"
        );
        seen.push_str(&body.to_string());
        drop(server);
    }

    // After a restart the same users land on the same profiles, with their
    // threads.
    {
        let (server, base, log) = start_hashed(&d, "second");
        logs.push(log);
        for (user, profile) in USERS.iter().zip(&expected) {
            let (id, created) = provision_hashed(&client, &base, user, &mut seen);
            assert_eq!((&id, created), (profile, false));
            let (status, body) = user_rpc_with(
                &client,
                &base,
                BEARER,
                user,
                None,
                "openhuman.threads_list",
                json!({}),
            );
            assert_eq!(status, 200, "{body}");
            assert_eq!(
                thread_ids(&body),
                vec!["hashed-thread".to_string()],
                "{body}"
            );
            seen.push_str(&body.to_string());
        }
        drop(server);
    }

    // No user id on disk, in a response, or in the log.
    let mut paths = Vec::new();
    all_paths(d.tmp.path(), &mut paths);
    for path in &paths {
        assert_free_of_user_ids("a path on disk", &path.display().to_string());
    }
    assert_free_of_user_ids("a response", &seen);
    for log in &logs {
        // The CLI server's logging layer writes to stderr (`<name>.err`).
        let mut text = std::fs::read_to_string(log).unwrap_or_default();
        text.push_str(&std::fs::read_to_string(log.with_extension("err")).unwrap_or_default());
        assert!(
            text.contains("[profiles]"),
            "the log was captured: {}",
            log.display()
        );
        assert_free_of_user_ids(&format!("log {}", log.display()), &text);
    }
}
