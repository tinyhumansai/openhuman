//! Structured agent turns on a `HostOnly` agent.
//!
//! An analyst reads a document with host tools and answers in a schema the host
//! parses. These read the requests the provider received (the response format
//! and output cap must reach every call of the tool loop) and the outcome the
//! host gets back (the parsed answer, why the model stopped, which model
//! answered, and the reasoning it spent). The document is untrusted data, so a
//! host-only agent may take it past the prompt guard; no other agent may.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::{chat_requests, offline_config, runtime, stub_backend};
use openhuman_embed::complete::ResponseFormat;
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
        .tools(move |_| {
            let reads = reads.clone();
            Box::pin(async move {
                HostTurnTools::advertised(vec![Box::new(ReadFile(reads))])
            })
        })
}

fn answer_schema() -> ResponseFormat {
    ResponseFormat::JsonSchema {
        name: "analysis".to_string(),
        schema: json!({
            "type": "object",
            "properties": { "summary": { "type": "string" } },
            "required": ["summary"]
        }),
    }
}

fn body(request: &Request) -> Value {
    serde_json::from_slice(&request.body).expect("json body")
}

#[test]
fn required_exploration_refuses_premature_answers_and_preserves_gateway_options() {
    let _guard = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            let runtime = build_runtime(&backend).await;
            for (index, model) in ["openai/gpt-4.1-mini", "moonshotai/kimi-k2.5", "minimax/minimax-m3"].iter().enumerate() {
                let premature = provider(vec![completion(
                    json!({"role":"assistant","content":"{\"summary\":\"complete\"}"}),"stop",model,0,
                )]).await;
                let reads = Arc::new(AtomicUsize::new(0));
                let spec = analyst(&format!("premature-{index}"), &premature, reads.clone())
                    .provider(Provider::openai_compatible(format!("{}/v1",premature.uri()),"fixture").model(*model));
                let agent = runtime.agent(spec).expect("agent");
                // Prompt-only exploration accepts premature schema-valid JSON.
                let advisory = agent.turn("Read documents/sample.txt before answering.")
                    .response_format(answer_schema()).max_tokens(1024).untrusted_input(true)
                    .send().await.expect("advisory prompt accepts provider reply");
                assert!(advisory.structured.is_some());
                assert_eq!(reads.load(Ordering::SeqCst),0);
                let outcome = agent.turn("Read documents/sample.txt before answering.").response_format(answer_schema())
                    .max_tokens(1024).require_tool_call(true).untrusted_input(true)
                    .provider_options(json!({"provider":{"only":["fixture"]},"reasoning":{"effort":"low"}}))
                    .send().await;
                assert!(outcome.is_err(), "{model} must not accept an unexplored schema-valid answer");
                assert_eq!(reads.load(Ordering::SeqCst),0);
                let requests = chat_requests(&premature).await;
                assert_eq!(requests.len(),2);
                let wire = body(&requests[1]);
                assert_eq!(wire["tool_choice"],"required");
                assert!(wire.get("response_format").is_none());
                assert!(wire["tools"].as_array().unwrap().iter().any(|tool|tool["function"]["name"]=="read_file"),"native host tools must survive for {model}");
                assert_eq!(wire["provider"]["only"],json!(["fixture"]));
                assert_eq!(wire["reasoning"]["effort"],"low");
            }
            let rejected = provider(vec![
                completion(json!({"role":"assistant","content":null,"tool_calls":[{"id":"call-blocked","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"blocked\"}"}}]}),"tool_calls","fixture",0),
                completion(json!({"role":"assistant","content":"{\"summary\":\"complete\"}"}),"stop","fixture",0),
            ]).await;
            let rejected_reads = Arc::new(AtomicUsize::new(0));
            let rejected_agent = runtime.agent(analyst("rejected-read",&rejected,rejected_reads.clone())).expect("agent");
            assert!(rejected_agent.turn("Read before answering.").response_format(answer_schema()).require_tool_call(true).send().await.is_err());
            assert_eq!(rejected_reads.load(Ordering::SeqCst),1);
            let rejected_requests = chat_requests(&rejected).await;
            assert_eq!(rejected_requests.len(),2);
            assert_eq!(body(&rejected_requests[1])["tool_choice"],"required");
            assert!(body(&rejected_requests[1]).get("response_format").is_none());
            let served = provider(vec![
                completion(json!({"role":"assistant","content":null,"tool_calls":[{"id":"call-read","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"documents/sample.txt\"}"}}]}),"tool_calls","fixture",0),
                completion(json!({"role":"assistant","content":"{\"summary\":\"complete\"}"}),"stop","actual-answered",0),
            ]).await;
            let reads = Arc::new(AtomicUsize::new(0));
            let agent = runtime.agent(analyst("explored",&served,reads.clone())).expect("agent");
            let outcome = agent.turn("Read before answering.").response_format(answer_schema())
                .require_tool_call(true).max_tokens(1024).send().await.expect("explored answer");
            assert_eq!(reads.load(Ordering::SeqCst),1);
            assert_eq!(outcome.answered_model.as_deref(),Some("actual-answered"));
            let requests = chat_requests(&served).await;
            assert!(body(&requests[0]).get("response_format").is_none());
            assert_eq!(body(&requests[1])["response_format"]["type"],"json_schema");
        }).await.expect("test task");
    });
}
