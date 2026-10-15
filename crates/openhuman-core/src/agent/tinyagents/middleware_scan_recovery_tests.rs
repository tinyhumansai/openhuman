use super::*;

#[tokio::test]
async fn broad_scan_can_recover_but_repeated_scan_and_actual_denial_stop() {
    let error = "Blocked: [policy-blocked] Unbounded system scan of '/' was blocked. Narrow the search to the project or a specific permitted SDK/toolchain directory.";
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    for (id, command) in [
        ("scan1", "find / -name raylib.h"),
        ("scan2", "find /usr -name SDL.h"),
    ] {
        let mut call = TaToolCall::new(id, "shell", serde_json::json!({"command":command}));
        mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
        let mut result = failing_result("shell", error);
        mw.after_tool(&mut ctx(), &(), &invocation(id, "shell"), &mut result)
            .await
            .unwrap();
        assert_eq!(
            drain_pause_count(&handle),
            if id == "scan1" { 0 } else { 1 }
        );
    }
    let policy = super::super::repeated_failure::recovery_policy;
    assert_eq!(
        policy("shell", "[policy-denied] User denied command", false),
        Some(("policy", 0))
    );
    assert_eq!(
        policy("shell", "[policy-blocked] Forbidden path", false),
        Some(("blocked_by_policy", 1))
    );
    assert_eq!(
        policy("web_fetch", error, false),
        Some(("blocked_by_policy", 1))
    );
    assert_eq!(
        policy(
            "shell",
            &format!("Command failed (exit code 1)\n{error}"),
            false
        ),
        None
    );
}

#[tokio::test]
async fn bounded_inspection_after_scan_rejection_continues() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let mut call = TaToolCall::new(
        "broad",
        "shell",
        serde_json::json!({"command":"find / -name SDL.h"}),
    );
    mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
    let mut rejection = failing_result(
        "shell",
        "[policy-blocked] Unbounded system scan of '/' was blocked. Narrow the search to the project or a specific permitted SDK/toolchain directory.",
    );
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("broad", "shell"),
        &mut rejection,
    )
    .await
    .unwrap();
    let mut call = TaToolCall::new(
        "bounded",
        "shell",
        serde_json::json!({"command":"clang++ --version"}),
    );
    mw.before_tool(&mut ctx(), &(), &mut call).await.unwrap();
    let mut result = TaToolResult::success("Apple clang version 16");
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("bounded", "shell"),
        &mut result,
    )
    .await
    .unwrap();
    assert_eq!(drain_pause_count(&handle), 0);
    assert!(slot.lock().unwrap().is_none());
}
