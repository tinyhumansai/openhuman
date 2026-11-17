use super::*;
use crate::core::runtime::{CoreContext, DomainSet, SaasConfig};
use crate::profiles::{ProfileHost, ProfileId, ProfileIdMode};

#[tokio::test]
async fn cron_completion_stays_in_each_profile_when_agent_ids_match() {
    const CHILD: &str = "OPENHUMAN_PROFILE_CRON_ROUTING_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "desktop::notifications::bus::profile_routing_tests::cron_completion_stays_in_each_profile_when_agent_ids_match",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stdout}\n{stderr}");
        assert!(
            stdout.contains("PROFILE_ROUTING_CHILD_COMPLETE"),
            "child did not run the profile routing assertions:\n{stdout}\n{stderr}"
        );
        return;
    }

    use std::sync::Arc;

    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(ProfileHost::new(
        SaasConfig::new(root.path()),
        CoreContext::for_test(DomainSet::full(), None),
    ));
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let profile_a =
        ProfileId::for_user(&format!("profile-a-{suffix}"), ProfileIdMode::Raw).unwrap();
    let profile_b =
        ProfileId::for_user(&format!("profile-b-{suffix}"), ProfileIdMode::Raw).unwrap();
    host.provision(&profile_a).await.unwrap();
    host.provision(&profile_b).await.unwrap();
    let state_a = host.open(&profile_a).await.unwrap();
    let state_b = host.open(&profile_b).await.unwrap();
    crate::profiles::host::install(Arc::clone(&host));

    let subscriber = NotificationBridgeSubscriber::new(crate::config::Config::default());
    for (profile, job_id) in [(&profile_a, "profile-job-a"), (&profile_b, "profile-job-b")] {
        subscriber
            .handle(&DomainEvent::CronJobCompleted {
                job_id: job_id.into(),
                success: true,
                output: "done".into(),
                tenant: Some(crate::core::runtime::Tenant {
                    profile: Some(profile.as_str().to_string()),
                    agent: Some("shared-agent-id".into()),
                }),
            })
            .await;
    }

    let config_a = host.provisioned_config(&profile_a).await.unwrap();
    let config_b = host.provisioned_config(&profile_b).await.unwrap();
    let events_a = CoreContext::scope(state_a.context().for_agent("shared-agent-id"), async {
        crate::desktop::notifications::store::list_core_notifications(&config_a, true, 10).unwrap()
    })
    .await;
    let events_b = CoreContext::scope(state_b.context().for_agent("shared-agent-id"), async {
        crate::desktop::notifications::store::list_core_notifications(&config_b, true, 10).unwrap()
    })
    .await;

    assert_eq!(events_a.len(), 1, "profile A owns its completion");
    assert!(events_a[0].id.starts_with("cron:profile-job-a:"));
    assert_eq!(events_b.len(), 1, "profile B owns its completion");
    assert!(events_b[0].id.starts_with("cron:profile-job-b:"));

    let missing_tenant = DomainEvent::CronJobCompleted {
        job_id: "missing-profile-job".into(),
        success: true,
        output: "done".into(),
        tenant: Some(crate::core::runtime::Tenant {
            profile: Some(format!("h-{}", "0".repeat(32))),
            agent: Some("shared-agent-id".into()),
        }),
    };
    assert!(event_dispatch(&missing_tenant).is_none());
    println!("PROFILE_ROUTING_CHILD_COMPLETE");
}
