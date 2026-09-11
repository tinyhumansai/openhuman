//! Unit tests for the pooled-execution types.

use std::time::Duration;

use tinyruntime_bus::{ExecResponse, Language};

use super::{PoolExecOutcome, PoolLang, PoolSettings};
use crate::openhuman::config::RuntimePoolLangConfig;

fn outcome(exit_code: Option<i32>) -> PoolExecOutcome {
    PoolExecOutcome {
        stdout: String::new(),
        stderr: String::new(),
        exit_code,
        timed_out: false,
        elapsed: Duration::ZERO,
        queue_wait: Duration::ZERO,
    }
}

#[test]
fn success_requires_a_clean_exit_and_no_timeout() {
    assert!(outcome(Some(0)).success());
    assert!(outcome(None).success());
    assert!(!outcome(Some(1)).success());

    let timed_out = PoolExecOutcome {
        timed_out: true,
        ..outcome(Some(0))
    };
    assert!(
        !timed_out.success(),
        "a job aborted at its deadline did not succeed"
    );
}

#[test]
fn a_module_reply_keeps_run_time_and_queue_wait_apart() {
    // A host that cannot tell a slow job from a busy pool will tune the wrong
    // thing, so the two never collapse into one number.
    let response = ExecResponse::new("out", "err", Some(0), "22.11.0").with_timings(12, 900);
    let adapted = PoolExecOutcome::from_module(&response);

    assert_eq!(adapted.stdout, "out");
    assert_eq!(adapted.stderr, "err");
    assert_eq!(adapted.elapsed, Duration::from_millis(12));
    assert_eq!(adapted.queue_wait, Duration::from_millis(900));
    assert!(adapted.success());
}

#[test]
fn a_timed_out_reply_stays_timed_out_through_the_adaptation() {
    let response = ExecResponse::new("", "", None, "22.11.0").with_timed_out(true);
    let adapted = PoolExecOutcome::from_module(&response);
    assert!(adapted.timed_out);
    assert!(!adapted.success());
}

#[test]
fn each_pool_language_round_trips_through_its_bus_language() {
    for lang in [PoolLang::Node, PoolLang::Python] {
        assert_eq!(PoolLang::from_language(&lang.language()), Some(lang));
    }
}

#[test]
fn an_unfamiliar_language_is_skipped_rather_than_guessed() {
    // The module routes whatever its own configuration routes; a status surface
    // should skip an entry this build has no pool concept for.
    assert_eq!(PoolLang::from_language(&Language::new("ruby")), None);
}

#[test]
fn settings_disable_idle_reaping_on_zero() {
    let cfg = RuntimePoolLangConfig {
        enabled: Some(true),
        max_workers: 3,
        idle_ttl_secs: 0,
        recycle_after_jobs: 5,
        max_queue_depth: 10,
    };
    let settings = PoolSettings::from_lang_config(&cfg);
    assert_eq!(settings.max_workers, 3);
    assert!(settings.idle_ttl.is_none());
    assert_eq!(settings.recycle_after_jobs, 5);
}
