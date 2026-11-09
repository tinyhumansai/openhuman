use tinytools::Tool;

pub(super) async fn make_tool_registry<Fut>(
    host_only: bool,
    build: impl FnOnce() -> Fut,
) -> (
    Vec<Box<dyn Tool>>,
    Option<tinyagents_harness_tools::skills::PackRegistryHandle>,
)
where
    Fut: std::future::Future<
        Output = (
            Vec<Box<dyn Tool>>,
            tinyagents_harness_tools::skills::PackRegistryHandle,
        ),
    >,
{
    if host_only {
        (Vec::new(), None)
    } else {
        let (tools, handle) = build().await;
        (tools, Some(handle))
    }
}
