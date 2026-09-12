use super::*;
use serde_json::json;

/// A bridge whose every read is scripted, so the dispatch table can be
/// exercised without a store.
#[derive(Default)]
struct FakeBridge {
    fail: bool,
    fail_list: bool,
    panic_on_get: bool,
    panic_on_copilot: bool,
}

#[async_trait]
impl WorkflowBridge for FakeBridge {
    fn list(&self) -> Vec<WorkflowDescriptor> {
        vec![WorkflowDescriptor {
            id: "wf-1".into(),
            name: "Deploy".into(),
            description: String::new(),
            node_count: 3,
            enabled: None,
            trigger_kind: None,
            agent_id: None,
            workspace_id: None,
            inputs: Vec::new(),
        }]
    }

    fn try_list(&self) -> Result<Vec<WorkflowDescriptor>, String> {
        if self.fail_list {
            return Err("store unavailable".into());
        }
        Ok(self.list())
    }

    fn get(&self, id: &str) -> Result<Value, String> {
        if self.panic_on_get {
            panic!("store exploded");
        }
        if self.fail {
            return Err("no such workflow".into());
        }
        Ok(json!({ "id": id, "graph": [] }))
    }

    fn node_kinds(&self, kind: Option<&str>) -> Result<Value, String> {
        Ok(json!({ "kind": kind }))
    }

    fn runs(&self, id: &str) -> Result<Value, String> {
        Ok(json!({ "runs": [id] }))
    }

    async fn copilot(
        &self,
        instruction: &str,
        workflow_id: Option<&str>,
    ) -> Result<CopilotOutcome, String> {
        if self.panic_on_copilot {
            panic!("copilot exploded");
        }
        if self.fail {
            return Err("copilot refused".into());
        }
        Ok(CopilotOutcome {
            reply: instruction.to_string(),
            changes: vec![],
            created: workflow_id.is_none().then(|| "wf-new".to_string()),
        })
    }

    fn agent_id(&self) -> Option<String> {
        Some("orchestrator".into())
    }
}

/// A scripted bridge, handed straight to `dispatch` — the tests never touch
/// the process-global registry, so they stay order- and thread-independent.
fn bridge_of(fail: bool, panic_on_get: bool) -> Arc<dyn WorkflowBridge> {
    Arc::new(FakeBridge {
        fail,
        panic_on_get,
        ..FakeBridge::default()
    })
}

/// A bridge whose authoring turn panics mid-`await` — the async twin of
/// `panic_on_get`, and the one arm `blocking()` does not cover.
fn panicking_copilot_bridge() -> Arc<dyn WorkflowBridge> {
    Arc::new(FakeBridge {
        panic_on_copilot: true,
        ..FakeBridge::default()
    })
}

fn request(op: WorkflowOp) -> WorkflowRequest {
    WorkflowRequest {
        request_id: "r1".into(),
        op,
        workflow_id: None,
        kind: None,
        instruction: None,
        agent_id: None,
    }
}

#[test]
fn result_frame_populates_exactly_one_arm() {
    let ok = result_frame("r1".into(), Ok(json!({ "a": 1 })));
    assert!(ok.ok);
    assert_eq!(ok.data, Some(json!({ "a": 1 })));
    assert!(ok.error.is_none());

    let failed = result_frame("r1".into(), Err("boom".into()));
    assert!(!failed.ok);
    assert!(failed.data.is_none());
    assert_eq!(failed.error.as_deref(), Some("boom"));
}

#[test]
fn connection_generation_cancels_work_at_each_lifetime_boundary() {
    let mut generation = ConnectionGeneration::disconnected();
    assert!(generation.snapshot().is_cancelled());

    generation.begin();
    let connected = generation.snapshot();
    assert!(!connected.is_cancelled());

    generation.end();
    assert!(connected.is_cancelled());
    assert!(generation.snapshot().is_cancelled());
}

#[test]
fn missing_workflow_id_is_reported_not_guessed() {
    let mut req = request(WorkflowOp::Get);
    assert_eq!(
        require_workflow_id(&req, "get"),
        Err("workflow get requires a workflowId".to_string())
    );
    // Blank is absent, not an id that will merely miss in the store.
    req.workflow_id = Some("   ".into());
    assert!(require_workflow_id(&req, "get").is_err());
    req.workflow_id = Some(" wf-1 ".into());
    assert_eq!(require_workflow_id(&req, "get"), Ok("wf-1".to_string()));
}

#[tokio::test]
async fn dispatch_routes_each_read_to_the_bridge() {
    let bridge = bridge_of(false, false);
    let mut get = request(WorkflowOp::Get);
    get.workflow_id = Some("wf-1".into());
    assert_eq!(
        dispatch(Arc::clone(&bridge), get).await.unwrap(),
        json!({ "id": "wf-1", "graph": [] })
    );

    let mut kinds = request(WorkflowOp::NodeKinds);
    kinds.kind = Some("agent".into());
    assert_eq!(
        dispatch(Arc::clone(&bridge), kinds).await.unwrap(),
        json!({ "kind": "agent" })
    );

    let mut runs = request(WorkflowOp::Runs);
    runs.workflow_id = Some("wf-1".into());
    assert_eq!(
        dispatch(bridge, runs).await.unwrap(),
        json!({ "runs": ["wf-1"] })
    );
}

#[tokio::test]
async fn a_read_missing_its_workflow_id_fails_without_reaching_the_store() {
    let outcome = dispatch(bridge_of(false, true), request(WorkflowOp::Get)).await;
    // `panic_on_get` proves the store was never consulted.
    assert_eq!(
        outcome,
        Err("workflow get requires a workflowId".to_string())
    );
}

#[tokio::test]
async fn copilot_requires_an_instruction_and_reports_creation() {
    let bridge = bridge_of(false, false);
    // A blank instruction never reaches the host's agent.
    let mut blank = request(WorkflowOp::Copilot);
    blank.instruction = Some("  ".into());
    assert!(dispatch(Arc::clone(&bridge), blank).await.is_err());

    let mut create = request(WorkflowOp::Copilot);
    create.instruction = Some("build a deploy flow".into());
    let data = dispatch(bridge, create).await.unwrap();
    assert_eq!(data["reply"], "build a deploy flow");
    assert_eq!(data["created"], "wf-new");
}

#[tokio::test]
async fn a_bridge_error_becomes_a_readable_failure() {
    let bridge = bridge_of(true, false);
    let mut get = request(WorkflowOp::Get);
    get.workflow_id = Some("wf-1".into());
    assert_eq!(
        dispatch(Arc::clone(&bridge), get).await,
        Err("no such workflow".to_string())
    );

    let mut copilot = request(WorkflowOp::Copilot);
    copilot.instruction = Some("change it".into());
    assert_eq!(
        dispatch(bridge, copilot).await,
        Err("copilot refused".to_string())
    );
}

#[tokio::test]
async fn a_list_error_is_not_converted_to_an_empty_registration() {
    let bridge: Arc<dyn WorkflowBridge> = Arc::new(FakeBridge {
        fail_list: true,
        ..FakeBridge::default()
    });
    assert_eq!(
        read_batch(bridge).await.unwrap_err(),
        "store unavailable".to_string()
    );
}

#[tokio::test]
async fn a_newer_registration_suppresses_an_older_snapshot() {
    let sequencer = RegistrationSequencer::new();
    let older = sequencer.begin();
    let newer = sequencer.begin();
    let emitted = std::sync::Mutex::new(Vec::new());

    assert_eq!(
        sequencer
            .emit_if_newer(newer, || async {
                emitted.lock().unwrap().push("newer");
                true
            })
            .await,
        SequencedEmit::Emitted
    );
    assert_eq!(
        sequencer
            .emit_if_newer(older, || async {
                emitted.lock().unwrap().push("older");
                true
            })
            .await,
        SequencedEmit::Superseded
    );
    assert_eq!(*emitted.lock().unwrap(), vec!["newer"]);
}

#[tokio::test]
async fn a_failed_newer_read_does_not_suppress_an_older_success() {
    let sequencer = RegistrationSequencer::new();
    let older = sequencer.begin();
    let _newer = sequencer.begin();
    let emitted = std::sync::Mutex::new(Vec::new());

    // The newer read failed, so it never calls `emit_if_newer`. The older
    // successful snapshot must still advance the backend from its
    // pre-mutation state.
    assert_eq!(
        sequencer
            .emit_if_newer(older, || async {
                emitted.lock().unwrap().push("older");
                true
            })
            .await,
        SequencedEmit::Emitted
    );
    assert_eq!(*emitted.lock().unwrap(), vec!["older"]);
}

#[tokio::test]
async fn a_failed_enqueue_does_not_advance_the_success_watermark() {
    let sequencer = RegistrationSequencer::new();
    let older = sequencer.begin();
    let newer = sequencer.begin();

    assert_eq!(
        sequencer.emit_if_newer(newer, || async { false }).await,
        SequencedEmit::Failed
    );
    assert_eq!(
        sequencer.emit_if_newer(older, || async { true }).await,
        SequencedEmit::Emitted
    );
}

#[tokio::test]
async fn a_panicking_bridge_still_answers() {
    let mut get = request(WorkflowOp::Get);
    get.workflow_id = Some("wf-1".into());
    let outcome = dispatch(bridge_of(false, true), get).await;
    // The request must not be dropped — a dropped one costs the server its
    // whole deadline.
    assert!(outcome
        .unwrap_err()
        .starts_with("the workflow store failed to answer"));
}

#[tokio::test]
async fn a_panicking_copilot_still_answers() {
    let mut copilot = request(WorkflowOp::Copilot);
    copilot.instruction = Some("build a deploy flow".into());
    let outcome = dispatch(panicking_copilot_bridge(), copilot).await;
    // A panic inside the authoring turn must come back as an answer, not
    // abort the task that owes the backend a frame: an unanswered copilot
    // costs the server its whole ten-minute deadline.
    assert!(outcome
        .clone()
        .unwrap_err()
        .starts_with("the workflow store failed to answer"));
    let frame = result_frame("r1".into(), outcome);
    assert!(!frame.ok);
    assert!(frame.error.is_some());
}

#[test]
fn an_undecodable_frame_without_a_request_id_is_only_logged() {
    // Nothing to correlate, so nothing can be answered — but it must not
    // panic on the socket read path either.
    reject_unparsed_request(&json!({ "op": "apply_ops" }), "unknown variant");
}
