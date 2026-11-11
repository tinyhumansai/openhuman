//! Scripted inference for `profile_host_capabilities`: each user turn names the
//! tools the model calls, so the test drives chat and relay deterministically.

use serde_json::{json, Value};
use wiremock::{Request, Respond, ResponseTemplate};

use super::common::{chat_completion, tool_call_completion, tool_names};

pub(crate) struct CallTools;
impl Respond for CallTools {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let messages = body["messages"].as_array().unwrap();
        let last_user = messages
            .iter()
            .rposition(|message| message["role"] == "user")
            .unwrap();
        let has_tool_results = messages[last_user + 1..]
            .iter()
            .any(|message| message["role"] == "tool");
        let completion = if super::common::last_user_message(&body)
            .contains("deny callback request")
            && !has_tool_results
        {
            tool_call_completion("tenant_probe", r#"{"deny":true}"#)
        } else if super::common::last_user_message(&body).contains("unsafe fixture request") {
            let mut completion = tool_call_completion("shell", "{}");
            completion["choices"][0]["message"]["tool_calls"] = json!(["shell", "spawn_subagent", "http_request", "run_workflow"].iter().enumerate().map(|(i, name)| json!({"id":format!("unsafe_{i}"),"type":"function","function":{"name":name,"arguments":"{}"}})).collect::<Vec<_>>());
            completion
        } else if has_tool_results
            || !tool_names(request)
                .iter()
                .any(|name| name == "tenant_probe")
        {
            let message = super::common::last_user_message(&body);
            let user = if message.contains("alice") {
                "alice"
            } else {
                "bob"
            };
            let channel = if message.contains("\"telegram\" channel") {
                "relay"
            } else {
                "chat"
            };
            let reopened = if message.contains("after reopening") {
                "reopened-"
            } else {
                ""
            };
            chat_completion(&format!("done: {reopened}{channel}:{user}"))
        } else {
            let mut completion = tool_call_completion("tenant_probe", "{}");
            let names = ["tenant_probe"];
            completion["choices"][0]["message"]["tool_calls"] = json!(names.iter().enumerate().map(|(i, name)| json!({"id":format!("call_{i}"),"type":"function","function":{"name":name,"arguments":"{}"}})).collect::<Vec<_>>());
            let message = super::common::last_user_message(&body);
            let user = if message.contains("alice") {
                "alice"
            } else {
                "bob"
            };
            completion["choices"][0]["message"]["tool_calls"].as_array_mut().unwrap().push(json!({
                "id":"native-memory", "type":"function", "function":{"name":"memory", "arguments":json!({"action":"learn", "text":format!("{user}-native-learning")}).to_string()}
            }));
            completion
        };
        if body.get("stream").and_then(Value::as_bool) != Some(true) {
            return ResponseTemplate::new(200).set_body_json(completion);
        }
        let mut delta = completion["choices"][0]["message"].clone();
        if let Some(calls) = delta["tool_calls"].as_array_mut() {
            for (index, call) in calls.iter_mut().enumerate() {
                call["index"] = json!(index);
            }
        }
        let chunk = json!({"id":"fixture", "object":"chat.completion.chunk", "created":1700000000_u64, "model":"fixture", "choices":[{"index":0,"delta":delta,"finish_reason":null}]});
        let done = json!({"id":"fixture", "object":"chat.completion.chunk", "created":1700000000_u64, "model":"fixture", "choices":[{"index":0,"delta":{},"finish_reason":completion["choices"][0]["finish_reason"]}], "usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}});
        ResponseTemplate::new(200).set_body_raw(
            format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n"),
            "text/event-stream",
        )
    }
}
