//! Tests for the pooled-execution client.
//!
//! The pool's behaviour — warm workers, backpressure, recycling — belongs to the
//! `tinyruntime` module and is tested there. What is this core's decision is
//! whether a language pools by default, and how a module failure is classified,
//! because that classification is what keeps a job from running twice.

use super::{classify, node, python, PoolRunError};
use crate::openhuman::config::Config;
use crate::openhuman::runtime::client::RuntimeCallError;

#[test]
fn node_pools_by_default_and_python_does_not() {
    // Not an oversight: a pooled Node job runs in its own worker thread with a
    // fresh module graph, while a pooled Python job shares the interpreter with
    // every other job on that worker.
    let config = Config::default();
    assert!(node::enabled(&config.runtime_pool));
    assert!(!python::enabled(&config.runtime_pool));
}

#[test]
fn opting_python_in_turns_it_on() {
    let mut config = Config::default();
    config.runtime_pool.python.enabled = Some(true);
    assert!(python::enabled(&config.runtime_pool));
}

#[test]
fn turning_the_pool_off_wholesale_turns_it_off_for_every_language() {
    let mut config = Config::default();
    config.runtime_pool.enabled = false;
    config.runtime_pool.python.enabled = Some(true);
    assert!(!node::enabled(&config.runtime_pool));
    assert!(
        !python::enabled(&config.runtime_pool),
        "an explicit per-language opt-in must not survive the master switch"
    );
}

#[test]
fn a_saturated_pool_is_recognised_so_the_caller_does_not_spawn() {
    // Falling back to a per-call spawn here would reintroduce exactly the
    // resident memory the pool exists to cap.
    let error = RuntimeCallError::Failed("the `nodejs` runtime pool is at capacity".to_string());
    assert!(matches!(classify(&error), PoolRunError::Saturated));
}

#[test]
fn a_post_dispatch_failure_is_recognised_so_the_job_is_not_re_run() {
    // The distinction that matters most: this job may already have had its side
    // effects, and a fallback spawn would repeat them.
    let error = RuntimeCallError::Failed(
        "the `nodejs` job failed after dispatch: the worker closed its protocol stream".to_string(),
    );
    assert!(matches!(classify(&error), PoolRunError::PostDispatch(_)));
}

#[test]
fn anything_else_is_pre_dispatch_so_the_caller_may_fall_back() {
    // Including the module simply not being loaded: the job provably never ran,
    // so the legacy per-call spawn is safe and is what keeps the tool working.
    for error in [
        RuntimeCallError::Unavailable("no artifact for this host".to_string()),
        RuntimeCallError::Failed("the `nodejs` job could not be dispatched: no worker".to_string()),
        RuntimeCallError::InvalidRequest("no runtime provider for `ruby`".to_string()),
    ] {
        assert!(
            matches!(classify(&error), PoolRunError::PreDispatch(_)),
            "`{error}` was not classified as pre-dispatch"
        );
    }
}

#[test]
fn the_three_classifications_render_distinguishably() {
    // They drive opposite caller behaviour, so their messages must not blur.
    assert_eq!(
        PoolRunError::Saturated.to_string(),
        "runtime pool at capacity"
    );
    let pre = PoolRunError::PreDispatch(anyhow::anyhow!("spawn failed")).to_string();
    assert!(pre.starts_with("pre-dispatch pool failure:"), "got {pre}");
    let post = PoolRunError::PostDispatch(anyhow::anyhow!("read wedged")).to_string();
    assert!(
        post.starts_with("post-dispatch pool failure:"),
        "got {post}"
    );
}
