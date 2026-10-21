use tinytools::Tool;

pub(super) fn make_tool_registry(
    host_only: bool,
    build: impl FnOnce() -> (
        Vec<Box<dyn Tool>>,
        tinyagents_harness_tools::skills::PackRegistryHandle,
    ),
) -> (
    Vec<Box<dyn Tool>>,
    Option<tinyagents_harness_tools::skills::PackRegistryHandle>,
) {
    if host_only {
        (Vec::new(), None)
    } else {
        let (tools, handle) = build();
        (tools, Some(handle))
    }
}
