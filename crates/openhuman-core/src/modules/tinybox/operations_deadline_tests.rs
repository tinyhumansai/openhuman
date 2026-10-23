use super::*;

#[test]
fn docker_control_transport_deadline_tracks_the_bounded_module_operation() {
    assert_eq!(
        docker_control_deadline(10_000, DOCKER_CALL_GRACE),
        std::time::Duration::from_secs(15)
    );
    assert_eq!(
        docker_control_deadline(1, DOCKER_CALL_GRACE),
        std::time::Duration::from_millis(5_001)
    );
    assert_eq!(
        docker_control_deadline(u64::MAX, DOCKER_CALL_GRACE),
        std::time::Duration::from_secs(65),
        "an invalid oversized request must not create an effectively unbounded bus wait"
    );
}

#[tokio::test]
async fn docker_status_transport_uses_its_bounded_deadline() {
    let (_service, client, _) =
        mock_client_with_docker_control_delay(std::time::Duration::from_millis(100)).await;
    let error = call_docker_status_with_grace(
        &client,
        contract::DockerStatusRequest { timeout_ms: 5 },
        std::time::Duration::from_millis(5),
    )
    .await
    .expect_err("transport must stop waiting at operation deadline plus grace");

    assert!(error.contains("MODULE_CALL_REPORTED"), "{error}");

    let error = call_cleanup_docker_with_grace(
        &client,
        contract::CleanupDockerRequest {
            label: "openhuman.sandbox=true".to_owned(),
            timeout_ms: 5,
        },
        std::time::Duration::from_millis(5),
    )
    .await
    .expect_err("cleanup transport must also stop at its bounded deadline plus grace");
    assert!(error.contains("MODULE_CALL_REPORTED"), "{error}");
}

#[test]
fn resolve_local_grants_is_a_member_of_the_canonical_contract() {
    assert_eq!(RESOLVE_LOCAL_GRANTS, "ResolveLocalGrants");
    assert!(contract::METHODS.contains(&RESOLVE_LOCAL_GRANTS));
}
