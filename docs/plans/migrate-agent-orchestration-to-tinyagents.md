# TinyAgents orchestration migration

The host-neutral orchestration primitives have moved to TinyAgents. OpenHuman
retains adapters for product policy, agent execution context, detached-task
projection, progress, and worktree isolation.

The remaining OpenHuman orchestration module is documented in
[`agent/orchestration/README.md`](../../crates/openhuman-core/src/agent/orchestration/README.md).
TinyFlows remains the supported workflow product for building and running
user-authored flows. The former graph workflow-run host and its RPC surface
have been retired.
