# OpenHuman

OpenHuman is a React/Tauri desktop assistant with an in-process Rust core,
JSON-RPC server, CLI and terminal client. The Linux scanner image builds the
headless core and web frontend; native desktop integration still needs review
of the source in crates/openhuman-app.

## Trust boundaries and priorities

Untrusted input includes chat messages and attachments, model output and tool
arguments, web content, integration and MCP responses, imported files and
archives, and requests to the core's HTTP/JSON-RPC server.

Prioritize authentication and authorization, RPC origin and bearer validation,
credential handling, path traversal, archive extraction, tool execution and
sandbox escapes, cross-agent/user data access, and untrusted content reaching
frontend rendering or native IPC. Review native module admission and integrity
checks and the host adapters under crates/openhuman-core/src/modules.

Business logic lives in crates/openhuman-core/src; transport and server behavior
in crates/openhuman-rpc; hosted integration and login ownership in
crates/openhuman-tinyhumans; frontend rendering in app/src. Recursive vendor/
submodules contain the agent harness, tool types and loadable components.
Report integration defects and reachable dependency flaws with the owning
component identified.

## Intended execution policy

The assistant intentionally executes tools, including shell commands, with the
operator's granted access. The optional in-process autonomy policy is disabled
by default. Ordinary authorized tool execution is expected behavior. A useful
finding demonstrates crossing an actual boundary: bypassing authentication,
escaping a configured sandbox, accessing another identity's state, exposing
credentials, or turning untrusted content into execution without the required
user authorization. When testing the optional autonomy policy, explicitly
state the configuration and privileges used.

## Build and offline reproduction

The source is at /src; the core binary is target/debug/openhuman-core and the
frontend build is app/dist. Cargo and pnpm caches remain available. Registry
native modules are staged in target/oss-scanner-modules and selected through
OPENHUMAN_BUNDLED_MODULES.

Run `pnpm test` for frontend unit tests. Run focused Rust tests with
`cargo test --offline -p openhuman <filter> --lib`; root tests/ targets instead
belong to openhuman-cli (`cargo test --offline -p openhuman-cli --test <name>`).
Cargo dependencies are prefetched, but Rust test binaries are not precompiled.
Use scripts/mock-api-server.mjs and the repository's test helpers for scenarios
that need backend responses. Do not use production accounts, keys or services.
The full desktop GUI and platform-specific E2E suites are not built by this
headless recipe.

## Reporting

Provide the affected revision, configuration, attacker capabilities, concrete
reproducer, observed impact and a regression test where practical. Assess
severity by demonstrated impact and required privileges: unauthenticated code
execution or cross-user credential compromise is a priority; a crash limited
to an explicitly imported local file has a different impact. Avoid speculative
severity claims based solely on dangerous APIs or on intended tool access.
Do not publish findings or include real credentials or user data in reports.
