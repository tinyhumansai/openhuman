//! Due agent jobs of hosted profiles run once, as their own profile, and
//! their replies reach the host's delivery callback. One runtime and one test:
//! SaaS operating mode is process-wide.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{chat_completion, last_user_message, runtime_without_master_key, PointedInference};
use openhuman_core::cron::{self, AgentJobSpec, DeliveryConfig, Schedule};
use openhuman_embed::profiles::ProfileCredentialKind;
use openhuman_embed::{CronDelivery, CronReply, JobOrigin, ProfileRuntime, SaasConfig};
use wiremock::{Mock, Request, Respond, ResponseTemplate};

/// Answers each job by its prompt: `quiet` (which allows `NO_REPLY`) says
/// nothing, others name their owner.
struct Reply;
impl Respond for Reply {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let prompt = last_user_message(&body);
        let text = if prompt.contains("quiet") {
            "NO_REPLY".to_string()
        } else if prompt.contains("alice") {
            "ran: alice".to_string()
        } else {
            "ran: bob".to_string()
        };
        ResponseTemplate::new(200).set_body_json(chat_completion(&text))
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<CronReply>>);

#[async_trait::async_trait]
impl CronDelivery for Recorder {
    async fn deliver(&self, reply: CronReply) -> Result<(), String> {
        self.0.lock().unwrap().push(reply);
        Ok(())
    }
}

#[test]
fn due_jobs_run_once_as_their_own_profile_even_when_the_profile_is_closed() {
    runtime_without_master_key().block_on(async {
        tokio::spawn(scenario()).await.unwrap();
    });
}

async fn scenario() {
    let inference = wiremock::MockServer::start().await;
    Mock::given(wiremock::matchers::path_regex(r"chat/completions$"))
        .respond_with(Reply)
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
    // The shared backend every sweeper would contend on.
    openhuman_core::storage::install(Arc::new(tinystoragedrivers_testkit::MemoryBackend::new()));

    let root = tempfile::tempdir().unwrap();
    let recorder = Arc::new(Recorder::default());
    let runtime = ProfileRuntime::builder(SaasConfig::new(root.path()))
        .configure(|config| config.lease_ttl_secs = 3600)
        .cron_delivery(recorder.clone())
        .build()
        .await
        .unwrap();

    // Each user gets jobs created in their own thread, then the profile closes.
    let soon = chrono::Utc::now() + chrono::Duration::seconds(5);
    for (user, prompts) in [
        ("alice", vec!["alice reminder"]),
        (
            "bob",
            vec!["bob reminder", "bob quiet: otherwise reply NO_REPLY"],
        ),
    ] {
        let provisioned = runtime.provision(user).await.unwrap();
        runtime
            .set_credential(
                &provisioned.profile_id,
                ProfileCredentialKind::ApiKey,
                "fixture-key",
            )
            .await
            .unwrap();
        let handle = runtime.open(user).await.unwrap();
        let config = handle.context().embedder_config().unwrap().clone();
        handle
            .scope(async {
                for prompt in prompts {
                    let mut spec = AgentJobSpec::new(Schedule::At { at: soon }, prompt.to_string());
                    spec.delete_after_run = true;
                    spec.delivery = Some(DeliveryConfig {
                        mode: "origin".into(),
                        channel: None,
                        to: None,
                        best_effort: true,
                    });
                    spec.origin = Some(JobOrigin::Web {
                        thread_id: format!("t-{user}"),
                        agent_id: None,
                    });
                    cron::add_agent_job_from_spec(&config, spec).unwrap();
                }
            })
            .await;
        let id = handle.id().clone();
        drop(handle);
        assert!(runtime.release(&id).await.unwrap());
    }
    assert!(runtime.list().await.unwrap().iter().all(|p| !p.open));

    // Nothing is due yet; two sweepers race for each job once it is.
    assert_eq!(runtime.sweep_cron_at(chrono::Utc::now()).await.ran, 0);
    common::wait_until(soon).await;
    let (a, b) = tokio::join!(
        runtime.sweep_cron_at(chrono::Utc::now()),
        runtime.sweep_cron_at(chrono::Utc::now())
    );
    assert_eq!(a.ran + b.ran, 3, "each occurrence runs once: {a:?} {b:?}");

    let mut delivered: Vec<_> = recorder
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r.profile_id.clone(),
                r.thread_id.clone(),
                r.text.clone(),
                r.origin.clone(),
            )
        })
        .collect();
    delivered.sort_by(|x, y| x.0.cmp(&y.0));
    assert!(
        recorder
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.agent_id.is_none()),
        "a profile's default agent is not named"
    );
    let web = |user: &str| {
        Some(JobOrigin::Web {
            thread_id: format!("t-{user}"),
            agent_id: None,
        })
    };
    assert_eq!(
        delivered,
        [
            (
                Some("alice".into()),
                Some("t-alice".into()),
                "ran: alice".into(),
                web("alice")
            ),
            (
                Some("bob".into()),
                Some("t-bob".into()),
                "ran: bob".into(),
                web("bob")
            ),
        ],
        "NO_REPLY is not delivered and no reply crosses profiles"
    );

    // One-shot jobs are gone; a later sweep finds nothing.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(runtime.sweep_cron_at(chrono::Utc::now()).await.ran, 0);
    assert_eq!(recorder.0.lock().unwrap().len(), 2);
}
