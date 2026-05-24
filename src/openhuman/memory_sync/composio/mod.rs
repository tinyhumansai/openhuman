//! Composio-backed sync pipelines.
//!
//! New home for the per-provider sync code that lives across
//! `composio/providers/{gmail,slack,github,notion,linear,clickup,...}/`.
//! Each provider gets a submodule here whose job is to:
//!
//! 1. Resolve the user's Composio connection for the provider.
//! 2. Paginate through the provider's upstream surface (messages,
//!    issues, docs, …).
//! 3. Hand each record to `memory::ingest_pipeline` so it lands as raw
//!    md → chunks → tree leaves like any other ingest.
//!
//! ## Status
//!
//! Scaffold only. The actual per-provider sync code still lives under
//! `composio/providers/*/ingest.rs` and is invoked from
//! `bin/slack_backfill.rs` / `bin/gmail_backfill_3d.rs`. Migration plan
//! is a per-provider PR per submodule below.
//!
//! ## Provider submodules (planned)
//!
//! | Submodule | Source | Notes |
//! | --- | --- | --- |
//! | `gmail`    | `composio/providers/gmail/ingest.rs` | Backfill + incremental |
//! | `slack`    | `composio/providers/slack/ingest.rs` | Channel + DM |
//! | `github`   | `composio/providers/github/`         | Issues + PRs + comments |
//! | `notion`   | `composio/providers/notion/`         | Pages + databases |
//! | `linear`   | `composio/providers/linear/`         | Issues + comments |
//! | `clickup`  | `composio/providers/clickup/`        | Tasks + comments |
