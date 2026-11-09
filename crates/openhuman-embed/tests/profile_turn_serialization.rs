//! One profile runs one turn at a time: a due cron job waits for the profile's
//! in-flight chat turn instead of running beside it, while another profile's
//! turn is not held up. One runtime and one test: SaaS operating mode is
//! process-wide.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{chat_completion, last_user_message, runtime_without_master_key, PointedInference};
use openhuman_core::cron::{self, AgentJobSpec, Schedule};
use openhuman_embed::profiles::ProfileCredentialKind;
use openhuman_embed::{ProfileRuntime, SaasConfig};
use wiremock::{Mock, Request, Respond, ResponseTemplate};

/// How long the slow chat turn's model request takes to answer.
const SLOW: Duration = Duration::from_millis(2500);

/// Records when each marked prompt's model request arrived. Prompts containing
/// `slow` are answered after [`SLOW`]; the rest at once.
#[derive(Clone, Default)]
struct Arrivals(Arc<Mutex<Vec<(String, Instant)>>>);

impl Arrivals {
    fn at(&self, marker: &str) -> Option<Instant> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .find(|(prompt, _)| prompt.contains(marker))
            .map(|(_, at)| *at)
    }

    async fn wait_for(&self, marker: &str) -> Instant {
        for _ in 0..600 {
            if let Some(at) = self.at(marker) {
                return at;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("no model request for {marker:?}");
    }
}

impl Respond for Arrivals {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let prompt = last_user_message(&body);
        self.0
            .lock()
            .unwrap()
            .push((prompt.clone(), Instant::now()));
        let reply = ResponseTemplate::new(200).set_body_json(chat_completion("ok"));
        if prompt.contains("slow") {
            reply.set_delay(SLOW)
        } else {
            reply
        }
    }
}

#[test]
fn a_profiles_cron_run_waits_for_its_chat_turn_but_other_profiles_do_not() {
    runtime_without_master_key().block_on(async {
        tokio::spawn(scenario()).await.unwrap();
    });
}

async fn scenario() {
    let arrivals = Arrivals::default();
    let inference = wiremock::MockServer::start().await;
    Mock::given(wiremock::matchers::path_regex(r"chat/completions$"))
        .respond_with(arrivals.clone())
        .with_priority(1)
        .mount(&inference)
        .await;
    Mock::given(wiremock::matchers::any())
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({"success":true,"data":[]})),
        )
        .with_priority(10)
        .mount(&inference)
        .await;
    PointedInference::install(&inference.uri());
    openhuman_core::storage::install(Arc::new(tinystoragedrivers_testkit::MemoryBackend::new()));

    let root = tempfile::tempdir().unwrap();
    let runtime = ProfileRuntime::builder(SaasConfig::new(root.path()))
        .configure(|config| config.lease_ttl_secs = 3600)
        .build()
        .await
        .unwrap();

    let mut handles = Vec::new();
    for user in ["alice", "bob"] {
        let provisioned = runtime.provision(user).await.unwrap();
        runtime
            .set_credential(
                &provisioned.profile_id,
                ProfileCredentialKind::ApiKey,
                "fixture-key",
            )
            .await
            .expect("profile credential is stored");
        handles.push(runtime.open(user).await.unwrap());
    }
    let (alice, bob) = (handles.remove(0), handles.remove(0));

    // Alice has a cron job that will be due.
    let soon = chrono::Utc::now() + chrono::Duration::seconds(5);
    let config = alice.context().embedder_config().unwrap().clone();
    alice
        .scope(async {
            let spec = AgentJobSpec::new(Schedule::At { at: soon }, "alice cron".to_string());
            cron::add_agent_job_from_spec(&config, spec).unwrap();
        })
        .await;

    // Alice's chat turn is mid-model-call when the sweep fires.
    let chat = tokio::spawn({
        let alice = alice.clone();
        async move { alice.chat("t1", "alice slow chat").await }
    });
    let chat_started = arrivals.wait_for("alice slow chat").await;

    // Another profile is not held up by Alice's turn.
    tokio::time::timeout(Duration::from_secs(60), bob.chat("t1", "bob chat"))
        .await
        .expect("bob's turn finishes")
        .expect("bob's turn");
    assert!(
        chat_started.elapsed() < SLOW,
        "bob's turn must finish while alice's is still in flight"
    );
    assert!(!chat.is_finished(), "alice's turn is still running");

    // The sweep runs Alice's job, but only once her chat turn is over.
    common::wait_until(soon).await;
    let report = runtime.sweep_cron_at(chrono::Utc::now()).await;
    assert_eq!(
        report.ran, 1,
        "the occurrence is deferred, not skipped: {report:?}"
    );
    let cron_started = arrivals.wait_for("alice cron").await;
    assert!(
        cron_started >= chat_started + SLOW,
        "the cron run started {:?} after the chat turn began; it must wait {SLOW:?}",
        cron_started.duration_since(chat_started)
    );
    chat.await.unwrap().expect("alice's chat turn");
}
