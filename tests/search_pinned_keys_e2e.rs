//! Two agents in one process, each holding its own search key only in its
//! in-memory config, search through the one process-wide TinySearch module.
//!
//! The regression: the search proxy reloaded `config.toml` before every call,
//! so a key an embedder put on one agent's config never reached the module
//! and the agent's search tools answered "not available". Each agent's calls
//! must authenticate with that agent's key and no other, and the key must stay
//! out of the environment, every file and every log line.
//!
//! Needs the real module: `scripts/test-rust-with-mock.sh` stages the
//! pinned `vendor/tinysearch` (`node scripts/modules/build.mjs tinysearch`). The
//! provider is a loopback stand-in, so nothing leaves the machine.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};

use openhuman_core::config::{Config, SearchHostCredential};
use openhuman_core::search::build_search_tools;
use serde_json::json;
use tinytools::Tool;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEY_A: &str = "agent-a-brave-key-7f3c";
const KEY_B: &str = "agent-b-brave-key-91d2";

/// Every log line the process emits, at every level.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// One agent's config: the embedder's shape. Both agents share the runtime's
/// `config_path`, as every agent of an embedded runtime does.
fn agent_config(root: &Path, provider_url: &str, key: &str) -> Config {
    let mut config = Config {
        config_path: root.join("config.toml"),
        workspace_dir: root.join("workspace"),
        ..Config::default()
    };
    config.secrets.encrypt = false;
    // Brave alone serves search, whatever else the process could reach.
    config
        .search
        .roles
        .insert("search".into(), vec!["brave".into()]);
    config
        .search
        .pin_host_credential(
            "brave",
            SearchHostCredential::new(key).with_base_url(provider_url),
        )
        .expect("brave takes a key");
    config
}

async fn answer_for(server: &MockServer, key: &str, title: &str) {
    Mock::given(method("GET"))
        .and(path("/web/search"))
        .and(header("X-Subscription-Token", key))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "web": { "results": [{
                "title": title,
                "url": format!("https://example.com/{title}"),
                "description": "stand-in result"
            }]}
        })))
        .mount(server)
        .await;
}

fn web_search_tool(config: &Config) -> Box<dyn Tool> {
    build_search_tools(config)
        .into_iter()
        .find(|tool| tool.name() == "web_search_tool")
        .expect("a pinned key offers web_search_tool")
}

async fn search(tool: &dyn Tool) -> String {
    let result = tool
        .execute(json!({ "query": "who answers" }))
        .await
        .expect("the tool runs");
    let text = result.output();
    assert!(!result.is_error, "search failed: {text}");
    text
}

fn files_under(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.extend(files_under(&path));
            } else {
                found.push(path);
            }
        }
    }
    found
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_agent_searches_with_its_own_in_memory_key() {
    if openhuman_core::modules::local_artifact("tinysearch").is_none() {
        eprintln!(
            "SKIPPED (not run, not asserted): no tinysearch library. Run \
             `node scripts/modules/build.mjs tinysearch` and set OPENHUMAN_MODULE_SOURCE_DIR, \
             or use scripts/test-rust-with-mock.sh"
        );
        return;
    }
    let logs = Captured::default();
    let writer = logs.clone();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .try_init()
        .expect("this target installs the only subscriber");

    let env_before: Vec<_> = std::env::vars_os().collect();
    let root = tempfile::tempdir().unwrap();
    let provider = MockServer::start().await;
    answer_for(&provider, KEY_A, "answered-for-agent-a").await;
    answer_for(&provider, KEY_B, "answered-for-agent-b").await;

    let a = web_search_tool(&agent_config(root.path(), &provider.uri(), KEY_A));
    let b = web_search_tool(&agent_config(root.path(), &provider.uri(), KEY_B));

    // In turn: the module is reconfigured for each agent's call.
    for _ in 0..2 {
        let from_a = search(a.as_ref()).await;
        assert!(from_a.contains("answered-for-agent-a"), "{from_a}");
        let from_b = search(b.as_ref()).await;
        assert!(from_b.contains("answered-for-agent-b"), "{from_b}");
    }
    // At once: neither call may run under the other agent's configuration.
    for _ in 0..4 {
        let (from_a, from_b) = tokio::join!(search(a.as_ref()), search(b.as_ref()));
        assert!(from_a.contains("answered-for-agent-a"), "{from_a}");
        assert!(from_b.contains("answered-for-agent-b"), "{from_b}");
    }

    let tokens: Vec<String> = provider
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| {
            request.headers["X-Subscription-Token"]
                .to_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(tokens.len(), 12, "{tokens:?}");
    assert_eq!(tokens.iter().filter(|t| *t == KEY_A).count(), 6);
    assert_eq!(tokens.iter().filter(|t| *t == KEY_B).count(), 6);

    // The process environment is as it was, and holds neither key.
    let env_after: Vec<_> = std::env::vars_os().collect();
    assert_eq!(env_before, env_after);
    for (_, value) in &env_after {
        let value = value.to_string_lossy();
        assert!(!value.contains(KEY_A) && !value.contains(KEY_B));
    }

    // No config.toml was written, and no file anywhere holds a key.
    assert!(!root.path().join("config.toml").exists());
    for file in files_under(root.path()) {
        let bytes = std::fs::read(&file).unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains(KEY_A) && !text.contains(KEY_B),
            "{} holds a key",
            file.display()
        );
    }

    // The calls were logged, and no line carries a key.
    let logged = String::from_utf8_lossy(&logs.0.lock().unwrap()).into_owned();
    assert!(logged.contains("[search][tool]"), "nothing was captured");
    assert!(!logged.contains(KEY_A) && !logged.contains(KEY_B));
}
