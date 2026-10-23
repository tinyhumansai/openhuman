use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bound_dispatch_can_write_memory_without_nesting_runtimes() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    let dispatch = DispatchCtx::new(crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config.clone(),
    ));

    let result = run_action(
        &dispatch,
        &config,
        &json!({"action":"learn","text":"The user likes oolong tea"}),
        &facts(),
    )
    .await;

    assert!(!result.is_error, "{}", text(&result));
}
