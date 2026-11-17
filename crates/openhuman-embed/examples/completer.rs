//! Title: Stateless completion without booting an agent runtime
//! Summary: Stateless completion without booting an agent runtime.
//! Run: offline; optional live via OPENHUMAN_EXAMPLE_LIVE=1 and BASE_URL/API_KEY/MODEL.
//! Feature: default

mod support;
fn main() -> anyhow::Result<()> {
    support::run(run())
}
async fn run() -> anyhow::Result<()> {
    let provider = support::provider("completion reply").await;
    // ANCHOR: completer
    let route = support::example_provider(&provider)?;
    let completer = openhuman_embed::Completer::new(route.route().expect("explicit route").clone());
    let response = completer
        .complete(openhuman_embed::CompletionRequest::new(
            route.model_id().expect("explicit model"),
            vec![openhuman_embed::ChatMessage::user("Say hello")],
        ))
        .await?;
    assert!(!response.text.is_empty());
    if support::offline() {
        assert_eq!(response.text, "completion reply");
        assert_eq!(support::chat_requests(&provider).await.len(), 1);
    }
    println!("{}", response.text);
    // ANCHOR_END: completer
    support::passed("completer");
    Ok(())
}
