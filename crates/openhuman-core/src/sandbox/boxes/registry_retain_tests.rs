//! A retaining registry (agent37): boxes are created with their owner's key,
//! and every path that lets go of one forgets it, never closes it.

use super::*;
use crate::sandbox::boxes::scripted_driver::{ExecStep, ScriptedDriver};
use crate::sandbox::boxes::{RetainPolicy, Retention};
use std::collections::BTreeMap;

const TTL: Duration = Duration::from_secs(900);
const THIRTY_DAYS: u64 = 30 * 24 * 60 * 60;

fn policy() -> RetainPolicy {
    RetainPolicy {
        idle_sleep_secs: 900,
        delete_after_secs: THIRTY_DAYS,
    }
}

fn retaining(driver: &Arc<ScriptedDriver>, max_live: usize) -> Arc<BoxRegistry<ScriptedDriver>> {
    BoxRegistry::new(
        Arc::clone(driver),
        RegistryLimits {
            idle_ttl: TTL,
            max_live,
            retain: Some(policy()),
        },
    )
}

fn settings() -> BoxSettings {
    BoxSettings {
        image: "agent37-base".into(),
        cpu_millis: 2_000,
        memory_bytes: 4 * 1024 * 1024 * 1024,
        workdir: "/home/node".into(),
        owner_scope: OwnerScope::default(),
    }
}

fn request(command: &str) -> BoxRequest {
    BoxRequest {
        command: command.into(),
        env: BTreeMap::new(),
        timeout: Duration::from_secs(30),
    }
}

fn user(id: &str) -> Tenant {
    Tenant {
        profile: None,
        agent: Some(id.into()),
    }
}

fn kept_for(id: &str) -> Option<Retention> {
    Some(policy().for_owner(owner_key(&OwnerScope::default(), &user(id))))
}

#[test]
fn the_owner_key_is_stable_opaque_and_valid_for_the_backend() {
    let scope = OwnerScope::new("prod", std::path::Path::new("/srv/agent/workspace"));
    let key = owner_key(&scope, &user("+15551234567"));
    assert_eq!(key, owner_key(&scope, &user("+15551234567")));
    assert!(!key.contains("15551234567"), "{key}");
    assert!((1..=128).contains(&key.len()), "{key}");
    assert!(
        key.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "{key}"
    );
}

#[test]
fn every_part_of_the_tenant_changes_the_owner_key() {
    let tenants = [
        Tenant {
            profile: None,
            agent: None,
        },
        Tenant {
            profile: Some(String::new()),
            agent: None,
        },
        Tenant {
            profile: None,
            agent: Some(String::new()),
        },
        Tenant {
            profile: Some("a".into()),
            agent: Some("b".into()),
        },
        Tenant {
            profile: Some("ab".into()),
            agent: None,
        },
        Tenant {
            profile: None,
            agent: Some("ab".into()),
        },
        user("u1"),
        user("u2"),
    ];
    let keys: std::collections::HashSet<String> = tenants
        .iter()
        .map(|tenant| owner_key(&OwnerScope::default(), tenant))
        .collect();
    assert_eq!(keys.len(), tenants.len());
}

#[tokio::test(start_paused = true)]
async fn each_box_is_created_with_its_owners_key_and_policy() {
    let driver = ScriptedDriver::new();
    let boxes = retaining(&driver, 10);

    boxes
        .exec(&user("u1"), &settings(), &request("pwd"))
        .await
        .unwrap();
    boxes
        .exec(&user("u2"), &settings(), &request("pwd"))
        .await
        .unwrap();

    assert_eq!(driver.retentions(), vec![kept_for("u1"), kept_for("u2")]);
    let exec = driver.last_exec().unwrap();
    assert_eq!(exec.cwd, "/home/node");
}

#[tokio::test(start_paused = true)]
async fn reaping_forgets_an_idle_box_and_the_next_command_finds_it_again() {
    let driver = ScriptedDriver::new();
    let boxes = retaining(&driver, 10);
    boxes
        .exec(&user("u1"), &settings(), &request("touch a"))
        .await
        .unwrap();

    assert_eq!(boxes.reap(Instant::now() + TTL).await, 1);
    assert_eq!(driver.forgotten(), vec!["box-0"]);
    assert!(driver.closed().is_empty());

    boxes
        .exec(&user("u1"), &settings(), &request("ls"))
        .await
        .unwrap();
    assert_eq!(driver.exec_resources(), vec!["box-0", "box-0"]);
    assert_eq!(driver.retentions(), vec![kept_for("u1"), kept_for("u1")]);
}

#[tokio::test(start_paused = true)]
async fn release_and_shutdown_forget_retained_boxes() {
    let driver = ScriptedDriver::new();
    let boxes = retaining(&driver, 10);
    for id in ["u1", "u2", "u3"] {
        boxes
            .exec(&user(id), &settings(), &request("pwd"))
            .await
            .unwrap();
    }

    boxes.release(&user("u1")).await;
    boxes.shutdown().await;

    let mut forgotten = driver.forgotten();
    forgotten.sort();
    assert_eq!(forgotten, vec!["box-0", "box-1", "box-2"]);
    assert!(driver.closed().is_empty());
    assert_eq!(boxes.live(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_box_reported_gone_is_forgotten_not_closed() {
    let driver = ScriptedDriver::new();
    let boxes = retaining(&driver, 10);
    driver.then(ExecStep::Fail(BoxError::UnknownResource));

    boxes
        .exec(&user("u1"), &settings(), &request("pwd"))
        .await
        .unwrap();

    assert_eq!(driver.forgotten(), vec!["box-0"]);
    assert!(driver.closed().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_box_that_misses_the_deadline_is_forgotten_not_closed() {
    let driver = ScriptedDriver::new();
    let boxes = retaining(&driver, 10);
    driver.then(ExecStep::Hang);

    let result = boxes
        .exec(&user("u1"), &settings(), &request("sleep 999"))
        .await
        .unwrap();

    assert!(result.timed_out);
    assert_eq!(driver.forgotten(), vec!["box-0"]);
    assert!(driver.closed().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_full_registry_lets_go_of_its_least_recently_used_idle_box() {
    let driver = ScriptedDriver::new();
    let boxes = retaining(&driver, 2);
    boxes
        .exec(&user("u1"), &settings(), &request("pwd"))
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(5)).await;
    boxes
        .exec(&user("u2"), &settings(), &request("pwd"))
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(5)).await;

    // Neither box has been idle for the TTL, but a retained box let go of
    // is only asleep, so a new tenant takes the oldest one's place.
    boxes
        .exec(&user("u3"), &settings(), &request("pwd"))
        .await
        .unwrap();

    assert_eq!(driver.forgotten(), vec!["box-0"]);
    assert!(driver.closed().is_empty());
    assert_eq!(boxes.live(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_full_registry_never_lets_go_of_a_box_with_a_command_in_flight() {
    let driver = ScriptedDriver::new();
    let boxes = retaining(&driver, 1);
    let gate = Arc::new(tokio::sync::Notify::new());
    driver.then(ExecStep::WaitThenReply(Arc::clone(&gate)));
    let running = {
        let boxes = Arc::clone(&boxes);
        tokio::spawn(async move { boxes.exec(&user("u1"), &settings(), &request("make")).await })
    };
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    let refused = boxes
        .exec(&user("u2"), &settings(), &request("pwd"))
        .await
        .unwrap_err();

    assert_eq!(refused, BoxError::AtCapacity);
    assert!(driver.forgotten().is_empty());
    gate.notify_one();
    running.await.unwrap().unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_refused_retained_create_is_never_retried_as_a_disposable_box() {
    let driver = ScriptedDriver::new();
    let boxes = retaining(&driver, 10);
    driver.fail_next_creates(1);

    let refused = boxes
        .exec(&user("u1"), &settings(), &request("pwd"))
        .await
        .unwrap_err();

    assert!(matches!(refused, BoxError::Driver(_)), "{refused:?}");
    assert_eq!(driver.retentions(), vec![kept_for("u1")]);
}

#[tokio::test(start_paused = true)]
async fn a_command_cancelled_while_an_evicted_box_is_let_go_of_holds_no_slot() {
    let driver = ScriptedDriver::new();
    let boxes = retaining(&driver, 1);
    boxes
        .exec(&user("u1"), &settings(), &request("pwd"))
        .await
        .unwrap();
    let gate = Arc::new(tokio::sync::Notify::new());
    driver.hold_next_forget(Arc::clone(&gate));

    // u2 takes u1's place and waits while u1's box is forgotten; the caller
    // gives up right there.
    let abandoned = tokio::time::timeout(
        Duration::from_secs(1),
        boxes.exec(&user("u2"), &settings(), &request("pwd")),
    )
    .await;

    assert!(abandoned.is_err(), "the command was still waiting");
    assert_eq!(driver.forgotten(), vec!["box-0"]);
    assert_eq!(boxes.live(), 0, "the abandoned command left its entry held");
    gate.notify_one();
    boxes
        .exec(&user("u3"), &settings(), &request("pwd"))
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_cancelled_command_lets_go_of_its_retained_box_and_the_next_finds_it_again() {
    let driver = ScriptedDriver::new();
    let boxes = retaining(&driver, 10);
    driver.then(ExecStep::Hang);

    let abandoned = tokio::time::timeout(
        Duration::from_secs(1),
        boxes.exec(&user("u1"), &settings(), &request("sleep 3600")),
    )
    .await;
    assert!(abandoned.is_err(), "the command was still running");
    tokio::task::yield_now().await;

    assert_eq!(driver.forgotten(), vec!["box-0"]);
    assert!(driver.closed().is_empty());
    boxes
        .exec(&user("u1"), &settings(), &request("ls"))
        .await
        .unwrap();
    assert_eq!(driver.exec_resources(), vec!["box-0", "box-0"]);
    assert_eq!(driver.retentions(), vec![kept_for("u1"), kept_for("u1")]);
}

fn scope(namespace: &str, workspace: &str) -> OwnerScope {
    OwnerScope::new(namespace, std::path::Path::new(workspace))
}

fn scoped(owner_scope: OwnerScope) -> BoxSettings {
    BoxSettings {
        owner_scope,
        ..settings()
    }
}

#[test]
fn the_namespace_and_the_workspace_each_change_the_owner_key() {
    let tenant = Tenant::default();
    let scopes = [
        OwnerScope::default(),
        scope("", "/home/a/.openhuman/users/u1/workspace"),
        scope("", "/home/a/.openhuman/users/u2/workspace"),
        scope("staging", "/srv/workspace"),
        scope("prod", "/srv/workspace"),
        // Length-prefixed: moving bytes between the parts is a new scope.
        scope("prod/", "srv/workspace"),
    ];
    let keys: std::collections::HashSet<String> = scopes
        .iter()
        .map(|scope| owner_key(scope, &tenant))
        .collect();
    assert_eq!(keys.len(), scopes.len());
    assert_eq!(
        owner_key(&scope("prod", "/srv/workspace"), &tenant),
        owner_key(&scope("prod", "/srv/workspace"), &tenant),
    );
    let key = owner_key(&scope("prod-eu", "/srv/workspace"), &tenant);
    assert!(!key.contains("prod") && !key.contains("srv"), "{key}");
}

#[tokio::test(start_paused = true)]
async fn equal_tenants_in_two_scopes_never_share_a_retained_box() {
    let driver = ScriptedDriver::new();
    let boxes = retaining(&driver, 10);
    let staging = scope("staging", "/srv/workspace");
    let prod = scope("prod", "/srv/workspace");

    for owner_scope in [staging.clone(), prod.clone(), staging.clone()] {
        boxes
            .exec(&user("u1"), &scoped(owner_scope), &request("pwd"))
            .await
            .unwrap();
    }

    assert_eq!(driver.exec_resources(), vec!["box-0", "box-1", "box-0"]);
    assert_eq!(boxes.live(), 2);
    assert_eq!(
        driver.retentions(),
        vec![
            Some(policy().for_owner(owner_key(&staging, &user("u1")))),
            Some(policy().for_owner(owner_key(&prod, &user("u1")))),
        ]
    );

    // Releasing a tenant lets go of its box in every scope.
    boxes.release(&user("u1")).await;
    let mut forgotten = driver.forgotten();
    forgotten.sort();
    assert_eq!(forgotten, vec!["box-0", "box-1"]);
    assert_eq!(boxes.live(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_new_box_waits_for_the_owners_forget_still_in_progress() {
    let driver = ScriptedDriver::new();
    let boxes = retaining(&driver, 10);
    boxes
        .exec(&user("u1"), &settings(), &request("pwd"))
        .await
        .unwrap();
    let gate = Arc::new(tokio::sync::Notify::new());
    driver.hold_next_forget(Arc::clone(&gate));

    // The idle box is reaped; its forget waits on the gate.
    let reaping = {
        let boxes = Arc::clone(&boxes);
        tokio::spawn(async move { boxes.reap(Instant::now() + TTL).await })
    };
    while driver.forgotten().is_empty() {
        tokio::task::yield_now().await;
    }

    // The owner runs again while the old handle is still closing: the new
    // box must not be created (and so must not adopt that instance) yet.
    let again = {
        let boxes = Arc::clone(&boxes);
        tokio::spawn(async move { boxes.exec(&user("u1"), &settings(), &request("pwd")).await })
    };
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(driver.created(), 1, "the new box waited for the old forget");

    gate.notify_one();
    again.await.unwrap().unwrap();
    reaping.await.unwrap();
    assert_eq!(driver.created(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_command_given_up_after_release_never_lets_go_of_its_replacements_box() {
    let driver = ScriptedDriver::new();
    driver.then(ExecStep::Hang);
    let boxes = retaining(&driver, 10);
    let running = {
        let boxes = Arc::clone(&boxes);
        tokio::spawn(async move {
            boxes
                .exec(&user("u1"), &settings(), &request("sleep 3600"))
                .await
        })
    };
    while driver.exec_resources().is_empty() {
        tokio::task::yield_now().await;
    }

    // Release lets go of the tenant's box while its command still runs; the
    // tenant's next command gets the same retained box back.
    boxes.release(&user("u1")).await;
    boxes
        .exec(&user("u1"), &settings(), &request("pwd"))
        .await
        .unwrap();

    // The stale command gives up now. It must not let go of the box the
    // replacement holds under the same owner key.
    running.abort();
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;

    assert_eq!(driver.forgotten(), vec!["box-0"], "only the release let go");
}

#[tokio::test(start_paused = true)]
async fn shutdown_waits_for_a_cleanup_already_in_flight() {
    let driver = ScriptedDriver::new();
    let boxes = retaining(&driver, 10);
    boxes
        .exec(&user("u1"), &settings(), &request("pwd"))
        .await
        .unwrap();
    let gate = Arc::new(tokio::sync::Notify::new());
    driver.hold_next_forget(Arc::clone(&gate));
    let reaping = {
        let boxes = Arc::clone(&boxes);
        tokio::spawn(async move { boxes.reap(Instant::now() + TTL).await })
    };
    while driver.forgotten().is_empty() {
        tokio::task::yield_now().await;
    }

    let shutting_down = {
        let boxes = Arc::clone(&boxes);
        tokio::spawn(async move { boxes.shutdown().await })
    };
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(
        !shutting_down.is_finished(),
        "shutdown returned while the forget of a reaped box was still running"
    );

    gate.notify_one();
    shutting_down.await.unwrap();
    reaping.await.unwrap();
}
