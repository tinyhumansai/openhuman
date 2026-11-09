use super::*;

/// The reference engine, recording how far each single store waited.
struct WaitRecorder {
    inner: tinymemory_api::conformance::ReferenceEngine,
    waits: std::sync::Mutex<Vec<tinymemory_api::WaitFor>>,
}

#[async_trait]
impl tinymemory_api::MemoryEngine for WaitRecorder {
    fn descriptor(&self) -> &tinymemory_api::EngineDescriptor {
        self.inner.descriptor()
    }
    async fn health(&self) -> tinymemory_api::EngineHealth {
        self.inner.health().await
    }
    async fn recall(
        &self,
        req: tinymemory_api::RecallRequest,
    ) -> tinymemory_api::Result<tinymemory_api::RecallAnswer> {
        self.inner.recall(req).await
    }
    async fn fetch(
        &self,
        req: tinymemory_api::FetchRequest,
    ) -> tinymemory_api::Result<tinymemory_api::FetchPage> {
        self.inner.fetch(req).await
    }
    async fn store(
        &self,
        item: tinymemory_api::StoreItem,
    ) -> tinymemory_api::Result<tinymemory_api::StoreReceipt> {
        self.store_with(item, tinymemory_api::WriteOptions::visible())
            .await
    }
    async fn store_with(
        &self,
        item: tinymemory_api::StoreItem,
        options: tinymemory_api::WriteOptions,
    ) -> tinymemory_api::Result<tinymemory_api::StoreReceipt> {
        self.waits.lock().unwrap().push(options.wait);
        self.inner.store(item).await
    }
    async fn forget(
        &self,
        target: tinymemory_api::ForgetTarget,
    ) -> tinymemory_api::Result<tinymemory_api::ForgetReport> {
        self.inner.forget(target).await
    }
    async fn list(
        &self,
        req: tinymemory_api::ListRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ListPage> {
        self.inner.list(req).await
    }
    async fn consolidate(
        &self,
        req: tinymemory_api::ConsolidateRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ConsolidateReceipt> {
        self.inner.consolidate(req).await
    }
}

#[tokio::test]
async fn learn_returns_on_accept_and_says_recall_may_lag() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = Arc::new(WaitRecorder {
        inner: tinymemory_api::conformance::ReferenceEngine::new(),
        waits: std::sync::Mutex::new(Vec::new()),
    });
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());

    let learned = run_action(
        &ambient(),
        &config,
        &json!({"action": "learn", "text": "The user drinks oolong"}),
        &facts(),
    )
    .await;

    assert!(!learned.is_error, "{}", text(&learned));
    // A turn must never wait on the engine indexing a learning.
    assert_eq!(
        *engine.waits.lock().unwrap(),
        vec![tinymemory_api::WaitFor::Accepted]
    );
    let view = serde_json::from_str::<Value>(&text(&learned)).unwrap();
    assert!(!view["id"].as_str().unwrap().is_empty(), "{view}");
    assert_eq!(view["status"], LEARN_STATUS);

    // The RPC path still reads its own write.
    crate::memory::ops::learn(
        &config,
        LearnParams {
            text: "The user drinks puerh".into(),
            kind: None,
            confidence: None,
            meta: None,
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        engine.waits.lock().unwrap().last(),
        Some(&tinymemory_api::WaitFor::Visible)
    );
}

#[tokio::test]
async fn a_failed_learn_is_reported_to_the_model() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    std::fs::write(
        config.workspace_dir.join("memory"),
        "blocks outbox directory",
    )
    .unwrap();

    let learned = run_action(
        &ambient(),
        &config,
        &json!({"action": "learn", "text": "The user drinks oolong"}),
        &facts(),
    )
    .await;

    assert!(learned.is_error, "{}", text(&learned));
    assert!(
        text(&learned).contains("queue unavailable"),
        "{}",
        text(&learned)
    );

    let credit_tmp = tempfile::tempdir().unwrap();
    let credit_config = config_in(&credit_tmp);
    crate::memory::test_fixtures::RefusingEngine::out_of_credits().bind(&credit_config);
    let credit_error = crate::memory::ops::learn(
        &credit_config,
        LearnParams {
            text: "The user drinks oolong".into(),
            kind: None,
            confidence: None,
            meta: None,
        },
        None,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        &credit_error,
        tinymemory_api::Error::InsufficientCredits(_)
    ));
    assert_eq!(credit_error.code(), "INSUFFICIENT_CREDITS");
}

/// The reference engine, recording the date hint each read carried.
struct HintRecorder {
    inner: tinymemory_api::conformance::ReferenceEngine,
    hints: std::sync::Mutex<Vec<Option<tinymemory_api::TimeHint>>>,
}

#[async_trait]
impl tinymemory_api::MemoryEngine for HintRecorder {
    fn descriptor(&self) -> &tinymemory_api::EngineDescriptor {
        self.inner.descriptor()
    }
    async fn health(&self) -> tinymemory_api::EngineHealth {
        self.inner.health().await
    }
    async fn recall(
        &self,
        req: tinymemory_api::RecallRequest,
    ) -> tinymemory_api::Result<tinymemory_api::RecallAnswer> {
        self.hints.lock().unwrap().push(req.refers_to.clone());
        self.inner.recall(req).await
    }
    async fn fetch(
        &self,
        req: tinymemory_api::FetchRequest,
    ) -> tinymemory_api::Result<tinymemory_api::FetchPage> {
        self.hints.lock().unwrap().push(req.refers_to.clone());
        self.inner.fetch(req).await
    }
    async fn store(
        &self,
        item: tinymemory_api::StoreItem,
    ) -> tinymemory_api::Result<tinymemory_api::StoreReceipt> {
        self.inner.store(item).await
    }
    async fn forget(
        &self,
        target: tinymemory_api::ForgetTarget,
    ) -> tinymemory_api::Result<tinymemory_api::ForgetReport> {
        self.inner.forget(target).await
    }
    async fn list(
        &self,
        req: tinymemory_api::ListRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ListPage> {
        self.inner.list(req).await
    }
}

#[tokio::test]
async fn refers_to_reaches_the_engine_in_the_users_time_zone() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.user_timezone = Some("Asia/Kolkata".into());
    let engine = Arc::new(HintRecorder {
        inner: tinymemory_api::conformance::ReferenceEngine::new(),
        hints: std::sync::Mutex::new(Vec::new()),
    });
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());

    let fetched = run_action(
        &ambient(),
        &config,
        &json!({"action": "fetch", "query": "dinner", "refers_to": {"from": "2026-10-03", "to": "2026-10-04"}}),
        &facts(),
    )
    .await;
    assert!(!fetched.is_error, "{}", text(&fetched));
    let recalled = run_action(
        &ambient(),
        &config,
        &json!({"action": "recall", "question": "dinner", "refers_to": {"from": "2026-10-03"}}),
        &facts(),
    )
    .await;
    assert!(!recalled.is_error, "{}", text(&recalled));
    let bad = run_action(
        &ambient(),
        &config,
        &json!({"action": "fetch", "query": "dinner", "refers_to": {"from": "last Saturday"}}),
        &facts(),
    )
    .await;
    assert!(bad.is_error, "a non-date is a model-visible error");

    let day = |d| chrono::NaiveDate::from_ymd_opt(2026, 10, d).unwrap();
    let hints = engine.hints.lock().unwrap().clone();
    assert_eq!(
        hints,
        vec![
            Some(
                tinymemory_api::TimeHint::new(day(3), day(4), Some("Asia/Kolkata".into())).unwrap()
            ),
            Some(
                tinymemory_api::TimeHint::new(day(3), day(3), Some("Asia/Kolkata".into())).unwrap()
            ),
        ]
    );
}

#[tokio::test]
async fn learning_is_durably_queued_even_when_the_backend_refuses_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    crate::memory::test_fixtures::RefusingEngine::out_of_credits().bind(&config);
    let tool = MemoryTool::new(Arc::new(config.clone()));
    let result = tool
        .execute(json!({"action":"learn","text":"Prefers tea"}))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.text());
    let value: Value = serde_json::from_str(&result.text()).unwrap();
    assert_eq!(value["status"], "queued; not yet saved to memory");
    assert_eq!(crate::memory::tool_writes::drain(&config).await, 0);
}
