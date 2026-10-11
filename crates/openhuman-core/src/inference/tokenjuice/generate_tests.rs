use super::*;

fn request(token: &str) -> GenerateRequest {
    GenerateRequest {
        context_token: token.to_string(),
        purpose: "tool_output_summary".into(),
        system: "system".into(),
        prompt: "prompt".into(),
        max_output_tokens: 16,
    }
}

#[tokio::test]
async fn an_unknown_token_declines() {
    assert_eq!(serve(request("never-registered")).await, Ok(None));
}

#[tokio::test]
async fn a_registered_call_runs_once_with_the_request() {
    let ticket = register(Box::new(|request: GenerateRequest| {
        Box::pin(async move { Ok(format!("{} / {}", request.system, request.prompt)) })
    }));
    let token = ticket.token().to_string();
    assert_eq!(
        serve(request(&token)).await,
        Ok(Some("system / prompt".to_string()))
    );
    // Used up: a second call for the same token declines.
    assert_eq!(serve(request(&token)).await, Ok(None));
}

#[tokio::test]
async fn a_dropped_ticket_withdraws_its_call() {
    let ticket = register(Box::new(|_| {
        Box::pin(async { panic!("a withdrawn call must not run") })
    }));
    let token = ticket.token().to_string();
    drop(ticket);
    assert_eq!(serve(request(&token)).await, Ok(None));
}

#[tokio::test]
async fn a_failed_model_call_is_an_error() {
    let ticket = register(Box::new(|_| {
        Box::pin(async { Err(anyhow::anyhow!("provider offline")) })
    }));
    let token = ticket.token().to_string();
    assert_eq!(
        serve(request(&token)).await,
        Err("provider offline".to_string())
    );
}
