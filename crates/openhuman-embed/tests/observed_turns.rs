//! Actual model/tool observations survive owned runtime dispatch. All model
//! traffic uses loopback fixtures; content capture requires explicit consent.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::{offline_config, runtime, stub_backend};
use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentSpec, HostTurnTools, Provider, Runtime, Tool, ToolScopeSpec,
    Workspace,
};
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

static RUNTIME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct ReadFile(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl Tool for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "Read a document supplied by the host"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"path": {"type": "string"}}})
    }
    async fn execute(&self, args: Value) -> anyhow::Result<openhuman_embed::ToolResult> {
        self.0.fetch_add(1, Ordering::SeqCst);
        if args["path"] == "blocked" {
            return Ok(openhuman_embed::ToolResult::error("read denied"));
        }
        Ok(openhuman_embed::ToolResult::success(
            "Sample document content.",
        ))
    }
}

struct Script {
    bodies: Vec<Value>,
    next: AtomicUsize,
}

impl Respond for Script {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let index = self.next.fetch_add(1, Ordering::SeqCst);
        ResponseTemplate::new(200)
            .set_body_json(self.bodies[index.min(self.bodies.len() - 1)].clone())
    }
}

fn completion(message: Value, finish_reason: &str, model: &str, reasoning: u64) -> Value {
    json!({
        "id": "chatcmpl-structured",
        "object": "chat.completion",
        "created": 1_700_000_000_u64,
        "model": model,
        "choices": [{ "index": 0, "message": message, "finish_reason": finish_reason }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15,
            "completion_tokens_details": { "reasoning_tokens": reasoning }
        }
    })
}

async fn provider(bodies: Vec<Value>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(Script {
            bodies,
            next: AtomicUsize::new(0),
        })
        .mount(&server)
        .await;
    server
}

async fn build_runtime(backend: &MockServer) -> Runtime {
    Runtime::builder()
        .config(offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .build()
        .await
        .expect("runtime")
}

fn routed(spec: AgentSpec, provider: &MockServer) -> AgentSpec {
    spec.provider(
        Provider::openai_compatible(format!("{}/v1", provider.uri()), "fixture").model("fixture"),
    )
}

fn analyst(id: &str, provider: &MockServer, reads: Arc<AtomicUsize>) -> AgentSpec {
    routed(AgentSpec::new(id), provider)
        .access(Access::readonly())
        .definition(
            AgentDefinitionSpec::new()
                .bare_prompt("You must call read_file on documents/sample.txt before analyzing the document. Answer with the analysis JSON.")
                .tools(ToolScopeSpec::HostOnly),
        )
        .tools(move |_| HostTurnTools::advertised(vec![Box::new(ReadFile(reads.clone()))]))
}

#[derive(Default)]
struct Records {
    events: std::sync::Mutex<Vec<openhuman_embed::observe::TurnObservation>>,
    terminal: std::sync::Mutex<Vec<(bool, Option<String>)>>,
}
impl openhuman_embed::observe::TurnObserver for Records {
    fn on_event(&self, event: &openhuman_embed::observe::TurnObservation) {
        self.events.lock().unwrap().push(event.clone());
    }
    fn on_turn(&self, trace: &openhuman_embed::observe::TurnTrace<'_>) {
        self.terminal
            .lock()
            .unwrap()
            .push((trace.success, trace.run_id.clone()));
    }
}
// Native dispatch must survive a worker task that does not inherit task locals.
async fn dispatch_on_worker(
    turn: openhuman_embed::Turn,
) -> Result<openhuman_embed::TurnOutcome, openhuman_embed::CoreError> {
    use tinyagents_harness::observability::turn_observer::{current_scope, with_observer};
    let run = turn.send();
    match current_scope() {
        Some(observer) => tokio::spawn(with_observer(observer, run)).await,
        None => tokio::spawn(run).await,
    }
    .expect("worker task")
}
#[test]
fn model_and_tool_observations_capture_payloads_only_with_consent() {
    use openhuman_embed::observe::{observe_turn, TraceContent, TurnObservation};
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    runtime().block_on(async { tokio::spawn(async {
        let backend = stub_backend().await;
        let runtime = build_runtime(&backend).await;
        for (index,capture) in [TraceContent::MetadataOnly, TraceContent::Include].into_iter().enumerate() {
            let provider = provider(vec![
                completion(json!({"role":"assistant","content":null,"tool_calls":[{"id":"read-1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"SECRET-PATH\"}"}}]}),"tool_calls","actual-model",0),
                completion(json!({"role":"assistant","content":"SECRET-REPLY"}),"stop","actual-model",3),
            ]).await;
            let reads = Arc::new(AtomicUsize::new(0));
            let agent = runtime.agent(analyst(&format!("observed-{index}"),&provider,reads.clone())).unwrap();
            let records = Arc::new(Records::default());
            let result = observe_turn(records.clone(),capture,"session","SECRET-PROMPT",dispatch_on_worker(agent.turn("Read the file. SECRET-PROMPT"))).await.unwrap();
            assert_eq!(result.reply,"SECRET-REPLY");
            let terminal = records.terminal.lock().unwrap();
            assert_eq!(terminal.len(),1);
            assert!(terminal[0].0);
            let events = records.events.lock().unwrap();
            let models: Vec<_> = events.iter().filter_map(|event| match event {
                TurnObservation::Model {answered_model,finish_reason,usage,input,output,failed,..} => Some((answered_model,finish_reason,usage,input,output,failed)), _ => None,
            }).collect();
            assert_eq!(models.len(),2);
            assert!(terminal[0].1.is_some());
            assert!(events.iter().all(|event| match event {
                TurnObservation::Model {run_id,..} => Some(run_id) == terminal[0].1.as_ref(),
                TurnObservation::Tool {run_id,..} => run_id == &terminal[0].1,
            }));
            assert_eq!(models[1].0.as_deref(),Some("actual-model"));
            assert_eq!(models[1].1.as_deref(),Some("stop"));
            assert_eq!(models[1].2.as_ref().unwrap().reasoning_tokens,3);
            assert!(!models[1].5);
            let tools: Vec<_> = events.iter().filter(|event| matches!(event,TurnObservation::Tool {..})).collect();
            assert_eq!(tools.len(),2);
            assert!(matches!(tools[1],TurnObservation::Tool {failed:Some(false),..}));
            let debug = format!("{events:?}");
            assert_eq!(debug.contains("SECRET-PATH"),capture==TraceContent::Include);
            assert_eq!(debug.contains("SECRET-PROMPT"),capture==TraceContent::Include);
            assert_eq!(debug.contains("SECRET-REPLY"),capture==TraceContent::Include);
        }
        let provider = provider(vec![completion(json!({"role":"assistant","content":"SECRET-PREMATURE"}),"stop","refused-model",7)]).await;
        let agent = runtime.agent(analyst("refused-observed", &provider, Arc::new(AtomicUsize::new(0)))).unwrap();
        let records = Arc::new(Records::default());
        observe_turn(records.clone(), TraceContent::default(), "session", "prompt", dispatch_on_worker(agent.turn("Read before replying").require_tool_call(true))).await.unwrap_err();
        let terminal = records.terminal.lock().unwrap();
        assert_eq!(terminal.len(),1);
        assert!(!terminal[0].0);
        let events = records.events.lock().unwrap();
        assert!(events.iter().any(|event| matches!(event, TurnObservation::Model { answered_model:Some(model), usage:Some(usage), failed:false, .. } if model == "refused-model" && usage.reasoning_tokens == 7)));
        assert!(!format!("{events:?}").contains("SECRET-PREMATURE"));
    }).await.unwrap(); });
}
