# Goal: Finish Phase 7 Local Qwen Orchestration Roadmap & Upstream Delivery

**Target Branch**: `tinyhumansai/openhuman:main`  
**Source Branch**: `Unity7:fix/local-qwen-history`  
**Milestone**: Phase 7 Delivery & Verification

---

## 1. Goal Overview & Current Status

The objective of this goal is to bring the completed **Phase 7: Local Qwen Orchestration (Gates 1–8)** across the finish line into the upstream `tinyhumansai/openhuman` repository. 

All 8 technical implementation and verification gates have passed:
- **Gate 1**: Vendored pinned Goose state machine (`goose-agent`, Apache-2.0, commit `53672c3f`).
- **Gate 2**: Goose Turn Adapter with atomic checkpoints and durable resume.
- **Gate 3**: Pure deterministic mode selection (`chat`, `assist`, `agent`) with direct chat bypass.
- **Gate 4**: Typed capability catalog (`ToolCapability`) and `allow_metered_agent_tools: false` default spend lock.
- **Gate 5**: Qwen-Agent-compatible response normalizer, structured tool call extraction, and raw markup suppression.
- **Gate 6**: Typed completion contracts and loop guards.
- **Gate 7**: 12/12 live matrix scenarios verified against local LM Studio (`qwen38-openhuman`, `n_ctx=131072`).
- **Gate 8**: Windows MSVC release binaries built, signed checksums recorded.

---

## 2. Finish Line Checklist

### Stage A: Environment & Code Hygiene
- [x] **Reclaim disk space**: Purged `target/debug/` to recover 113+ GB free space on Drive `C:`.
- [x] **Extract inline test modules**: Moved inline `#[cfg(test)] mod tests` in `primary_tool_exposure.rs`, `primary_turn.rs`, `mode.rs`, `runtime.rs`, and `tool_exposure.rs` into sibling `*_tests.rs` files.
- [x] **Decompose oversized files (<750 line rule)**:
  - Decomposed `crates/openhuman-core/src/agent/goose/tests.rs` (1,731 lines) into descriptive `adapter_tests.rs`, `runner_tests.rs`, and `store_tests.rs`.
  - Split `planner_tests.rs`, `gate4_tests.rs`, `catalog_tests.rs`, `qwen_tests.rs`, `middleware_loop_guard_tests.rs`, `ops_voice_and_autonomy_tests.rs`.
  - Extracted helper routines from `model.rs` (down from 781 to 616 lines) into `model_helpers.rs`.
- [x] **Verify layout gate**: `node scripts/ci/check-openhuman-rust-layout.mjs` passed cleanly.

### Stage B: Submodule & Git Reconciliation
- [ ] **Reconcile `vendor/tinyagents` pointer**: Update submodule reference to merge `d750eb7` (Qwen history normalization) with upstream `89256cc` (non-consecutive repeat tracking).
- [ ] **Commit layout fixes & submodule update** cleanly to `fix/local-qwen-history`.
- [ ] **Push updated branch to fork**: `git push unity7 fix/local-qwen-history`.

### Stage C: Upstream Submission & CI Validation
- [ ] **Create upstream Pull Request**: Execute `gh pr create` with pre-generated specification linking all gate evidence and artifact checksums.
- [ ] **Monitor `CI Lite`**: Confirm GitHub Actions workflows pass (`rust-quality`, `layout`, `token-scan`, `cargo test`).
- [ ] **Engage Maintainer Review**: Notify repository maintainers for merge approval.

---

## 3. Key Reference Artifacts

- **Roadmap Specification**: [docs/roadmaps/local-qwen-orchestration.md](file:///C:/Bay/OpenHuman-src/docs/roadmaps/local-qwen-orchestration.md)
- **Live Evidence Matrix**: [target/phase7-live/evidence.md](file:///C:/Bay/OpenHuman-src/target/phase7-live/evidence.md)
- **Release Checksums**: [target/phase7-windows-release-evidence.json](file:///C:/Bay/OpenHuman-src/target/phase7-windows-release-evidence.json)
- **PR Description Payload**: [target/phase7-pr-description.md](file:///C:/Bay/OpenHuman-src/target/phase7-pr-description.md)
