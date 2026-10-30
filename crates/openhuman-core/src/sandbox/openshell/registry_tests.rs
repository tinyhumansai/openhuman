use super::*;
use crate::sandbox::openshell::scripted_driver::{ok_output, Call, ExecStep, ScriptedDriver};
use std::collections::BTreeMap;
use tokio::sync::Notify;

const TTL: Duration = Duration::from_secs(900);

fn limits(max_live: usize) -> RegistryLimits {
    RegistryLimits {
        idle_ttl: TTL,
        max_live,
    }
}

fn settings() -> BoxSettings {
    BoxSettings {
        image: "registry.example/agent:1".into(),
        cpu_millis: 1_000,
        memory_bytes: 256 * 1024 * 1024,
    }
}

fn request(command: &str) -> BoxRequest {
    BoxRequest {
        command: command.into(),
        env: BTreeMap::new(),
        timeout: Duration::from_secs(30),
    }
}

/// An embedded user: no profile, the user's id as the agent.
fn user(id: &str) -> Tenant {
    Tenant {
        profile: None,
        agent: Some(id.into()),
    }
}

fn registry(driver: &Arc<ScriptedDriver>, max_live: usize) -> Arc<BoxRegistry<ScriptedDriver>> {
    BoxRegistry::new(Arc::clone(driver), limits(max_live))
}

#[tokio::test(start_paused = true)]
async fn one_tenant_reuses_its_box_across_commands() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver, 10);

    let first = boxes
        .exec(&user("u1"), &settings(), &request("echo hi"))
        .await
        .unwrap();
    boxes
        .exec(&user("u1"), &settings(), &request("pwd"))
        .await
        .unwrap();

    assert_eq!(first.stdout, "ok");
    assert_eq!(first.exit_code, 0);
    assert!(!first.timed_out);
    assert_eq!(driver.created(), 1);
    assert_eq!(driver.exec_resources(), vec!["box-0", "box-0"]);
    assert_eq!(boxes.live(), 1);
}

#[tokio::test(start_paused = true)]
async fn the_box_enforces_the_deadline_from_its_own_workspace() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver, 10);
    let mut request = request("ls | wc -l");
    request.env.insert("LANG".into(), "C.UTF-8".into());

    boxes
        .exec(&user("u1"), &settings(), &request)
        .await
        .unwrap();

    let exec = driver.last_exec().unwrap();
    assert_eq!(
        exec.argv,
        vec!["timeout", "-k", "5", "30", "sh", "-c", "ls | wc -l"]
    );
    assert_eq!(exec.cwd, "/sandbox");
    assert_eq!(exec.env, request.env);
    assert_eq!(exec.deadline, Duration::from_secs(30) + BACKSTOP_MARGIN);
    assert_eq!(driver.calls()[0], Call::Create(settings()));
}

#[test]
fn deadlines_round_up_to_whole_seconds_of_at_least_one() {
    assert_eq!(whole_seconds(Duration::from_millis(1_500)), 2);
    assert_eq!(whole_seconds(Duration::from_secs(30)), 30);
    assert_eq!(whole_seconds(Duration::ZERO), 1);
}

#[tokio::test(start_paused = true)]
async fn embedded_users_of_one_host_get_separate_boxes() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver, 10);

    boxes
        .exec(&user("alice"), &settings(), &request("id"))
        .await
        .unwrap();
    boxes
        .exec(&user("bob"), &settings(), &request("id"))
        .await
        .unwrap();

    assert_eq!(driver.created(), 2);
    assert_eq!(driver.exec_resources(), vec!["box-0", "box-1"]);
}

#[tokio::test(start_paused = true)]
async fn concurrent_first_commands_create_one_box() {
    let driver = ScriptedDriver::new();
    driver.slow_create(Duration::from_secs(3));
    let boxes = registry(&driver, 10);

    let runs = (0..5).map(|n| {
        let boxes = Arc::clone(&boxes);
        tokio::spawn(async move {
            boxes
                .exec(&user("u1"), &settings(), &request(&format!("echo {n}")))
                .await
        })
    });
    for run in futures::future::join_all(runs).await {
        run.unwrap().unwrap();
    }

    assert_eq!(driver.created(), 1);
    assert!(driver.exec_resources().iter().all(|id| id == "box-0"));
}

#[tokio::test(start_paused = true)]
async fn reap_closes_a_box_once_it_has_been_idle_for_the_ttl() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver, 10);
    boxes
        .exec(&user("u1"), &settings(), &request("true"))
        .await
        .unwrap();
    let finished = Instant::now();

    assert_eq!(boxes.reap(finished + TTL - Duration::from_secs(1)).await, 0);
    assert!(driver.closed().is_empty());
    assert_eq!(boxes.reap(finished + TTL).await, 1);
    assert_eq!(driver.closed(), vec!["box-0"]);
    assert_eq!(boxes.live(), 0);

    boxes
        .exec(&user("u1"), &settings(), &request("true"))
        .await
        .unwrap();
    assert_eq!(driver.created(), 2);
}

#[tokio::test(start_paused = true)]
async fn reap_never_closes_a_box_with_a_command_in_flight() {
    let driver = ScriptedDriver::new();
    let gate = Arc::new(Notify::new());
    driver.then(ExecStep::WaitThenReply(Arc::clone(&gate)));
    let boxes = registry(&driver, 10);
    let running = {
        let boxes = Arc::clone(&boxes);
        tokio::spawn(async move {
            boxes
                .exec(&user("u1"), &settings(), &request("sleep 3600"))
                .await
        })
    };
    tokio::task::yield_now().await;
    while driver.exec_resources().is_empty() {
        tokio::task::yield_now().await;
    }

    assert_eq!(boxes.reap(Instant::now() + TTL * 4).await, 0);
    assert!(driver.closed().is_empty());

    gate.notify_one();
    running.await.unwrap().unwrap();
    assert_eq!(boxes.reap(Instant::now() + TTL).await, 1);
}

#[tokio::test(start_paused = true)]
async fn the_background_reaper_closes_idle_boxes() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver, 10);
    boxes.start_reaper();
    boxes.start_reaper();
    boxes
        .exec(&user("u1"), &settings(), &request("true"))
        .await
        .unwrap();

    tokio::time::sleep(TTL + REAP_EVERY_AT_MOST * 2).await;

    assert_eq!(driver.closed(), vec!["box-0"]);
}

#[test]
fn the_reaper_ticks_at_half_the_ttl_capped_and_never_zero() {
    assert_eq!(reap_interval(TTL), REAP_EVERY_AT_MOST);
    assert_eq!(
        reap_interval(Duration::from_secs(20)),
        Duration::from_secs(10)
    );
    assert_eq!(reap_interval(Duration::ZERO), Duration::from_secs(1));
}

#[tokio::test(start_paused = true)]
async fn a_new_tenant_is_refused_when_no_idle_box_can_be_reaped() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver, 1);
    boxes
        .exec(&user("alice"), &settings(), &request("true"))
        .await
        .unwrap();

    let refused = boxes
        .exec(&user("bob"), &settings(), &request("true"))
        .await
        .unwrap_err();

    assert_eq!(refused, BoxError::AtCapacity);
    assert_eq!(
        refused.to_string(),
        "the sandbox service is at capacity; try again shortly"
    );
    assert_eq!(driver.created(), 1);
    assert!(driver.closed().is_empty());
}

#[tokio::test(start_paused = true)]
async fn reaping_an_idle_box_makes_room_for_a_new_tenant() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver, 1);
    boxes
        .exec(&user("alice"), &settings(), &request("true"))
        .await
        .unwrap();
    tokio::time::advance(TTL).await;

    boxes
        .exec(&user("bob"), &settings(), &request("true"))
        .await
        .unwrap();

    let calls = driver.calls();
    let closed = calls
        .iter()
        .position(|call| *call == Call::Close("box-0".into()))
        .unwrap();
    let created = calls
        .iter()
        .rposition(|call| matches!(call, Call::Create(_)))
        .unwrap();
    assert!(closed < created);
    assert_eq!(boxes.live(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_failed_create_does_not_hold_a_slot() {
    let driver = ScriptedDriver::new();
    driver.fail_next_creates(1);
    let boxes = registry(&driver, 1);

    let failed = boxes
        .exec(&user("alice"), &settings(), &request("true"))
        .await
        .unwrap_err();
    assert_eq!(failed, BoxError::Driver("create refused".into()));
    assert_eq!(boxes.live(), 0);

    boxes
        .exec(&user("bob"), &settings(), &request("true"))
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn an_unknown_box_is_replaced_once_and_the_command_retried() {
    let driver = ScriptedDriver::new();
    driver.then(ExecStep::Fail(BoxError::UnknownResource));
    let boxes = registry(&driver, 10);

    let result = boxes
        .exec(&user("u1"), &settings(), &request("true"))
        .await
        .unwrap();

    assert_eq!(result.stdout, "ok");
    assert_eq!(driver.created(), 2);
    assert_eq!(driver.exec_resources(), vec!["box-0", "box-1"]);
}

#[tokio::test(start_paused = true)]
async fn a_box_reported_gone_is_still_closed_before_it_is_replaced() {
    // "Gone" is inferred from a probe after a failed exec, so the box may
    // still exist; dropping it without a close would leak it on the gateway.
    let driver = ScriptedDriver::new();
    driver.then(ExecStep::Fail(BoxError::UnknownResource));
    let boxes = registry(&driver, 10);

    boxes
        .exec(&user("u1"), &settings(), &request("true"))
        .await
        .unwrap();

    assert_eq!(driver.closed(), vec!["box-0"]);
}

#[tokio::test(start_paused = true)]
async fn a_second_unknown_box_fails_the_command() {
    let driver = ScriptedDriver::new();
    driver.then(ExecStep::Fail(BoxError::UnknownResource));
    driver.then(ExecStep::Fail(BoxError::UnknownResource));
    let boxes = registry(&driver, 10);

    let failed = boxes
        .exec(&user("u1"), &settings(), &request("true"))
        .await
        .unwrap_err();

    assert_eq!(failed, BoxError::UnknownResource);
    assert_eq!(driver.created(), 2);
    assert_eq!(boxes.live(), 0);
}

#[tokio::test(start_paused = true)]
async fn other_exec_failures_keep_the_box() {
    let driver = ScriptedDriver::new();
    driver.then(ExecStep::Fail(BoxError::Driver("bus down".into())));
    let boxes = registry(&driver, 10);

    let failed = boxes
        .exec(&user("u1"), &settings(), &request("true"))
        .await
        .unwrap_err();
    boxes
        .exec(&user("u1"), &settings(), &request("true"))
        .await
        .unwrap();

    assert_eq!(failed, BoxError::Driver("bus down".into()));
    assert_eq!(driver.created(), 1);
}

#[tokio::test(start_paused = true)]
async fn an_exit_of_124_is_the_box_timing_the_command_out() {
    let driver = ScriptedDriver::new();
    driver.then(ExecStep::Reply(BoxOutput {
        exit_code: 124,
        stdout: b"partial".to_vec(),
        stderr: Vec::new(),
    }));
    let boxes = registry(&driver, 10);

    let result = boxes
        .exec(&user("u1"), &settings(), &request("sleep 99"))
        .await
        .unwrap();

    assert!(result.timed_out);
    assert_eq!(result.stdout, "partial");
    assert!(driver.closed().is_empty());
}

#[tokio::test(start_paused = true)]
async fn the_backstop_discards_a_box_that_does_not_answer() {
    let driver = ScriptedDriver::new();
    driver.then(ExecStep::Hang);
    let boxes = registry(&driver, 10);
    let started = Instant::now();

    let result = boxes
        .exec(&user("u1"), &settings(), &request("sleep 99"))
        .await
        .unwrap();

    assert_eq!(started.elapsed(), Duration::from_secs(30) + BACKSTOP_MARGIN);
    assert!(result.timed_out);
    assert_eq!(result.exit_code, 124);
    assert_eq!(driver.closed(), vec!["box-0"]);
    boxes
        .exec(&user("u1"), &settings(), &request("true"))
        .await
        .unwrap();
    assert_eq!(driver.created(), 2);
}

#[tokio::test(start_paused = true)]
async fn release_closes_the_tenants_box_only() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver, 10);
    for id in ["alice", "bob"] {
        boxes
            .exec(&user(id), &settings(), &request("true"))
            .await
            .unwrap();
    }

    boxes.release(&user("alice")).await;
    boxes.release(&user("nobody")).await;

    assert_eq!(driver.closed(), vec!["box-0"]);
    assert_eq!(boxes.live(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_released_box_is_not_recreated_by_a_command_still_running() {
    let driver = ScriptedDriver::new();
    let gate = Arc::new(Notify::new());
    driver.then(ExecStep::WaitThenFail(
        Arc::clone(&gate),
        BoxError::UnknownResource,
    ));
    let boxes = registry(&driver, 10);
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

    boxes.release(&user("u1")).await;
    gate.notify_one();

    assert_eq!(running.await.unwrap().unwrap_err(), BoxError::Released);
    assert_eq!(driver.created(), 1);
    assert_eq!(boxes.live(), 0);
}

#[tokio::test(start_paused = true)]
async fn shutdown_closes_every_box() {
    let driver = ScriptedDriver::new();
    let boxes = registry(&driver, 10);
    for id in ["alice", "bob"] {
        boxes
            .exec(&user(id), &settings(), &request("true"))
            .await
            .unwrap();
    }

    boxes.shutdown().await;

    let mut closed = driver.closed();
    closed.sort();
    assert_eq!(closed, vec!["box-0", "box-1"]);
    assert_eq!(boxes.live(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_failed_close_is_logged_not_raised() {
    let driver = ScriptedDriver::new();
    driver.fail_closes();
    let boxes = registry(&driver, 10);
    boxes
        .exec(&user("u1"), &settings(), &request("true"))
        .await
        .unwrap();

    boxes.release(&user("u1")).await;
    boxes.shutdown().await;

    assert_eq!(driver.closed(), vec!["box-0"]);
    assert_eq!(boxes.live(), 0);
}

#[tokio::test(start_paused = true)]
async fn an_abandoned_command_stops_holding_its_box() {
    let driver = ScriptedDriver::new();
    driver.then(ExecStep::Hang);
    let boxes = registry(&driver, 10);
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

    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());

    assert_eq!(boxes.reap(Instant::now() + TTL).await, 1);
    assert_eq!(driver.closed(), vec!["box-0"]);
}

#[test]
fn output_bytes_become_lossy_text() {
    let result = finished(BoxOutput {
        exit_code: 2,
        stdout: vec![b'a', 0xff],
        stderr: b"oops".to_vec(),
    });
    assert_eq!(result.exit_code, 2);
    assert_eq!(result.stdout, "a\u{fffd}");
    assert_eq!(result.stderr, "oops");
    assert!(!result.timed_out);
    assert_eq!(finished(ok_output()).stdout, "ok");
}
