# Add WeChat message scraping into context and memory

## Summary

Ingest messages from the embedded WeChat webview into OpenHuman so the agent can use recent WeChat conversations as context and persist useful history into memory.

## Problem

We can embed WeChat Web in the Tauri shell for sign-in and manual use, but OpenHuman does not yet extract message data from that surface. That leaves a gap for users who coordinate through WeChat, especially for China-based communities and teams. The work needs to respect platform limits in WeChat Web, avoid broad DOM scraping that breaks easily, and keep private message content scoped to explicit product surfaces and consented storage paths.

## Solution (optional)

Add a dedicated WeChat webview ingestion path, following the existing Franz-style account model but with provider-specific extraction for chats, messages, unread state, and stable account/chat identifiers. Scope should cover:

- Tauri/app shell:
  Add a WeChat-specific extractor path for the embedded webview, with clear load-state handling and bounded retries when login/session state changes.
- Core:
  Define a WeChat webview ingest contract that can write normalized message snapshots into context/memory namespaces, similar to other webview-account providers.
- Product safeguards:
  Gate persistence behind existing memory/write surfaces, document what is stored, and make it easy to disable or purge.

## Acceptance criteria

- [ ] **Embedded WeChat extraction** — OpenHuman can detect the active WeChat chat surface and extract normalized message data from the embedded webview.
- [ ] **Context + memory ingestion** — extracted WeChat messages can be written into the existing context/memory pipeline with provider/account provenance.
- [ ] **Unread/respond surfaces** — unread or actionable WeChat activity appears in the same provider-surface/respond-queue flows used by other messaging integrations where applicable.
- [ ] **Privacy + control** — users can understand, disable, and purge stored WeChat-derived data using existing account/memory controls or clearly-scoped additions.
- [ ] **Tests** — add focused Vitest and Rust coverage for the WeChat provider path, including at least one failure/edge case.
- [ ] **Diff coverage ≥ 80%** — the implementing PR meets the changed-lines coverage gate (Vitest + cargo-llvm-cov, enforced by [`.github/workflows/pr-ci.yml`](../.github/workflows/pr-ci.yml)).

- Evaluate whether WeChat needs a native scanner path, injected bridge, or hybrid approach after validating what the embedded web client exposes at runtime.

## Related

- Follows the initial Tauri WeChat webview support added in `feat/wechat-webview-support`.
