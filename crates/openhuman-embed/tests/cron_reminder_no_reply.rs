//! A scheduled reminder always reaches the user. A run of a job whose prompt
//! does not offer `NO_REPLY` and answers it anyway (in any spelling) gets one
//! corrective turn and its real message is delivered; only a conditional job
//! that names the token may stay silent, and that run is recorded as
//! suppressed. One runtime and one test: SaaS operating mode is process-wide.

mod common;

use std::sync::{Arc, Mutex};

use common::{chat_completion, last_user_message, runtime_without_master_key, PointedInference};
use openhuman_core::cron::{self, AgentJobSpec, DeliveryConfig, DeliveryStatus, Schedule};
use openhuman_embed::profiles::ProfileCredentialKind;
use openhuman_embed::{CronDelivery, CronReply, JobOrigin, ProfileRuntime, SaasConfig};
use wiremock::{Mock, Request, Respond, ResponseTemplate};

const CORRECTION: &str = "which is not allowed for this task";
const REMINDER: &str = "Remind the user to drink water";
const NEWSLETTER: &str = "Send the user the weekly newsletter summary";
const CONDITIONAL: &str = "Check the inbox; reply NO_REPLY if nothing changed";

fn user_messages(body: &serde_json::Value) -> String {
    body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["role"] == "user")
        .map(|m| m["content"].to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

/// First answers are silent in three spellings; corrective turns get text.
struct Reply;
impl Respond for Reply {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let prompt = user_messages(&body);
        let corrected = last_user_message(&body).contains(CORRECTION);
        let text = if prompt.contains("drink water") {
            if corrected {
                "Time to drink water!"
            } else {
                "NO_REPLY"
            }
        } else if prompt.contains("newsletter") {
            if corrected {
                "Here is your weekly newsletter."
            } else {
                "No_reply."
            }
        } else if prompt.contains("inbox") {
            "NO_REPLY"
        } else {
            "unexpected request"
        };
        ResponseTemplate::new(200).set_body_json(chat_completion(text))
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
fn reminders_always_deliver_and_only_a_conditional_job_stays_silent() {
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
    openhuman_core::storage::install(Arc::new(tinystoragedrivers_testkit::MemoryBackend::new()));

    let root = tempfile::tempdir().unwrap();
    let recorder = Arc::new(Recorder::default());
    let runtime = ProfileRuntime::builder(SaasConfig::new(root.path()))
        .configure(|config| config.lease_ttl_secs = 3600)
        .cron_delivery(recorder.clone())
        .build()
        .await
        .unwrap();

    let user = "carol";
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
    let soon = chrono::Utc::now() + chrono::Duration::seconds(5);
    let conditional_id = handle
        .scope(async {
            let mut conditional_id = String::new();
            for (name, prompt) in [
                ("water", REMINDER),
                ("newsletter", NEWSLETTER),
                ("inbox", CONDITIONAL),
            ] {
                let mut spec = AgentJobSpec::new(Schedule::At { at: soon }, prompt.to_string());
                spec.name = Some(name.to_string());
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
                let job = cron::add_agent_job_from_spec(&config, spec).unwrap();
                if prompt == CONDITIONAL {
                    conditional_id = job.id;
                }
            }
            conditional_id
        })
        .await;
    let id = handle.id().clone();
    drop(handle);
    assert!(runtime.release(&id).await.unwrap());

    common::wait_until(soon).await;
    let swept = runtime.sweep_cron_at(chrono::Utc::now()).await;
    assert_eq!(swept.ran, 3, "{swept:?}");

    let mut delivered: Vec<String> = recorder
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|reply| reply.text.clone())
        .collect();
    delivered.sort();
    assert_eq!(
        delivered,
        ["Here is your weekly newsletter.", "Time to drink water!"],
        "both reminders deliver their real message; the conditional job stays silent"
    );

    let requests = inference.received_requests().await.unwrap();
    let bodies: Vec<serde_json::Value> = requests
        .iter()
        .filter(|request| request.url.path().ends_with("chat/completions"))
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect();
    let (corrective, _first): (Vec<_>, Vec<_>) = bodies
        .iter()
        .partition(|body| last_user_message(body).contains(CORRECTION));
    assert_eq!(
        corrective.len(),
        2,
        "one corrective turn per reminder, none for the check"
    );
    for body in &corrective {
        let last = last_user_message(body);
        assert!(
            !last.contains("drink water") && !last.contains("newsletter"),
            "the corrective turn must not replay the job prompt: {last}"
        );
    }

    let handle = runtime.open(user).await.unwrap();
    let config = handle.context().embedder_config().unwrap().clone();
    let runs = handle
        .scope(async {
            common::eventually("the conditional run's record", || {
                cron::list_runs(&config, &conditional_id, 10)
                    .ok()
                    .filter(|runs| runs.iter().any(|run| run.delivery_status.is_some()))
            })
            .await
        })
        .await;
    assert!(
        runs.iter()
            .any(|run| run.delivery_status == Some(DeliveryStatus::Suppressed)),
        "{runs:?}"
    );
}
