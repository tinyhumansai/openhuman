# Legacy candidate decisions for core shrink

Decision date: 2026-10-11. Parent: [#7390](https://github.com/tinyhumansai/openhuman/issues/7390); decision task: [#7411](https://github.com/tinyhumansai/openhuman/issues/7411).
Evidence was inspected at OpenHuman upstream main `1f2e59a238` and openhuman-benchmarks main `3ac3de418eb8ff94498552724456ee201dd0c117`.

This record selects the defaults for six follow-ups. It changes no runtime behavior, feature selections, capability entries, or gitlinks. Each behavior change needs its own reviewed PR and regression coverage. A module migration first lands in its canonical owner, then ships compatible artifacts before OpenHuman consumes its bus contract.

| Candidate               | Decision                                                                                                                      | Follow-up                                                                                                                                    |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| TinyMemes               | Keep the opt-in capability; move remix behavior into the TinyMemes compiled module. Preserve the current runtime-off default. | [TinyMemes #2](https://github.com/tinyhumansai/tinymemes/issues/2), [OpenHuman #7446](https://github.com/tinyhumansai/openhuman/issues/7446) |
| File-based hooks        | Keep for the CLI, with an explicit host compile/runtime gate; omit from the standard desktop product.                         | [OpenHuman #7447](https://github.com/tinyhumansai/openhuman/issues/7447)                                                                     |
| WhatsApp Web            | Remove the unshipped OpenHuman feature and its unsupported message-reading advertisement.                                     | [OpenHuman #7448](https://github.com/tinyhumansai/openhuman/issues/7448)                                                                     |
| rss-bench               | Keep the off-by-default core instrumentation gate used by the external profiling binaries.                                    | [openhuman-benchmarks #36](https://github.com/tinyhumansai/openhuman-benchmarks/issues/36)                                                   |
| Four ComingSoon entries | Remove these unimplemented or redundant promises; retain implemented capability entries.                                      | [OpenHuman #7449](https://github.com/tinyhumansai/openhuman/issues/7449)                                                                     |
| Profiles                | Keep shared desktop identity/layout; gate only SaaS profile orchestration behind an opt-in feature.                           | [OpenHuman #7450](https://github.com/tinyhumansai/openhuman/issues/7450)                                                                     |

## TinyMemes: keep and move to its owner

[The host adapter](../../crates/openhuman-core/src/tinymemes/mod.rs) runs on the final web-chat reply, leaves the harness transcript original, and falls back to the original reply on failure or timeout. [The web-chat caller](../../crates/openhuman-core/src/web_chat/run_task.rs) holds treatment-arm streaming and invokes the remix after the turn. This is an implemented capability, despite having no dedicated frontend controller.

[The experiment selector](../../crates/openhuman-core/src/tinymemes/arm.rs) reads `OPENHUMAN_TINYMEMES`, supports stable per-thread A/B assignment, and falls back to `OPENHUMAN_TINYMEMES_DEFAULT` if supplied during compilation; neither flag means off. [Core defaults](../../crates/openhuman-core/Cargo.toml), [the product feature list](../../scripts/ci/product-features.txt), and [the shell manifest](../../crates/openhuman-app/Cargo.toml) still compile it.

The [TinyMemes repository](https://github.com/tinyhumansai/tinymemes) owns its engine, evaluation, indexes and sources. Its vendored [manifest](https://github.com/tinyhumansai/tinymemes/blob/46d5fb3b8dfe43b523c79c68975f8d3e524e8cc8/Cargo.toml) is a linked library, with no bus contract or loadable module. Add that boundary upstream rather than delete the experiment to shed a dependency. OpenHuman keeps credential resolution, approved inference/search callbacks, acting identity, delivery and lifecycle policy; the module owns remix execution and index behavior. Preserve opt-in settings and off/control/treatment semantics, original transcripts, timeout fallback and content-free experiment telemetry.

The storage work in [#7431](https://github.com/tinyhumansai/openhuman/issues/7431) already covers TinyMemes index/remixed persistence. Coordinate a module-owned query/migration implementation over storage ports, with an import regression for existing files; do not file a duplicate storage rewrite.

Required proof in the follow-up: exact minimal contract and actual compiled-module invocation; mocked off/control/treatment, timeout/failure and transcript-preservation tests; legacy index import without data loss; enabled/disabled dependency graphs and product feature forwarding. Do not promote a gitlink before released platform artifacts are available.

## File-based hooks: keep the CLI capability

[The schemas](../../crates/openhuman-core/src/hooks/schemas.rs) register `hooks.list`, `hooks.reload` and `hooks.test`. [The generic CLI namespace dispatcher](../../crates/openhuman-core/src/core/cli.rs) resolves registered controllers, so no hardcoded `hooks` CLI match arm is needed. [The contributor guide](../gitbooks/en/developing/hooks.md) documents the three actual `openhuman-core hooks ...` commands. No `app/src` caller for those controller names was found.

The implementation has already partly moved: [the host module](../../crates/openhuman-core/src/hooks/mod.rs) names `tinyagents_runtime::command_hooks` as owner of parsing, layering, matching and execution. OpenHuman retains product environment, shell/model callbacks and bridge policy. Do not repeat this generic migration or confuse it with the separate in-process `agent::hooks` and `stop_hooks` APIs embedders use.

Absence of frontend RPC callers does not mean hooks are currently inert on desktop: [bootstrap](../../crates/openhuman-core/src/core/runtime/bootstrap.rs) initializes them, [web chat](../../crates/openhuman-core/src/web_chat/ops/start_chat.rs) invokes prompt hooks, and [subagent execution](../../crates/openhuman-core/src/agent/subagent_host/ops/runner.rs) invokes subagent-start hooks. CLI-only is therefore a deliberate surface change.

Add an explicit `command-hooks` feature through the crate chain, enabled by the CLI's normal feature selection and omitted from the shipped desktop product. Gate the host domain, its registrations, bootstrap and lifecycle invocations consistently. Keep `[hooks] enabled = false` as the runtime opt-out where compiled in. Preserve the wire names and CLI behavior; leave in-process embedder hooks intact. The CLI can also serve RPC, so “CLI-only” means the CLI host/build, not removing its controllers from a CLI server.

Required proof: CLI list/reload/synthetic-test dispatch and configured deny behavior; disabled build exposes no file-hook controllers and performs no script loading/execution; desktop feature graph excludes the host adapter; in-process embedder callbacks still work. Generic engine regressions belong in TinyAgents, host composition regressions in OpenHuman.

## WhatsApp Web: remove the unshipped host surface

The [product features](../../scripts/ci/product-features.txt) omit `whatsapp-web`, and the [desktop manifest](../../crates/openhuman-app/Cargo.toml) does not request it. Yet [about_app](../../crates/openhuman-core/src/platform/about_app/catalog_auth_channels_team.rs) marks `channels.whatsapp_read_messages` Beta and tells the user to connect WhatsApp Web for local message reading.

The opt-in core feature forwards to `tinychannels/whatsapp-web`; [channel re-exports](../../crates/openhuman-core/src/channels/mod.rs) and [runtime feature reporting](../../crates/openhuman-core/src/core/runtime/features.rs) still name it. The issue's references to runtime context/domain-set code are historical: no current `whatsapp-web` branch was found there. [Memory sender handling](../../crates/openhuman-core/src/memory/lifecycle/sender.rs) accepts the generic `whatsapp` channel identity without this feature and must remain.

Remove OpenHuman's unused feature forwarding/re-exports/reporting and the unsupported `channels.whatsapp_read_messages` capability. Keep TinyChannels' separately owned implementation, legitimate hosted WhatsApp/channel routing, sender identity normalization, and ordinary web browsing. Reintroducing local WhatsApp Web reading would require a separate approved product capability and compiled-module/desktop integration proof; this decision does not ship it.

Required proof: product about_app list/search/lookup no longer offer the unsupported id; generic WhatsApp origin/sender and hosted-channel tests still pass; Cargo metadata has no OpenHuman `whatsapp-web` feature; feature-forwarding and shipped product checks remain green. No new scanner or webview injection.

## rss-bench: keep the instrumentation gate

Moving binaries did not eliminate their host-side instrumentation. OpenHuman [declares the off-by-default gate](../../crates/openhuman-core/Cargo.toml), [enables the deterministic provider override under it](../../crates/openhuman-core/src/inference/provider/factory.rs), and [allows profiling-only UTC forcing](../../crates/openhuman-core/src/agent/prompts/render_helpers/section_renderers.rs). None is a default or desktop product feature.

The external [profile manifest](https://github.com/tinyhumansai/openhuman-benchmarks/blob/3ac3de418eb8ff94498552724456ee201dd0c117/profile/Cargo.toml) requires `rss-bench` for both `rss-bench` and `library-profile`, forwarding it to `openhuman-core/rss-bench`. [Its mock providers](https://github.com/tinyhumansai/openhuman-benchmarks/blob/3ac3de418eb8ff94498552724456ee201dd0c117/profile/src/bin/library_profile/mock.rs) explicitly depend on the gated deterministic override. [The feature-forwarding checker](../../scripts/ci/check-feature-forwarding.mjs) already documents this intentional core-only exception.

Keep the core instrumentation; keep all benchmark binaries/scripts in openhuman-benchmarks. The follow-up belongs to that repository: add a focused offline check that its slim profiling builds can still install/use the deterministic provider, and document the OpenHuman gate and compatible pin. Do not restore bins, profiling dependencies or a default profiling override to OpenHuman.

Required proof in that follow-up: profile crate metadata/check for both bins with `--no-default-features --features rss-bench`; an offline deterministic smoke with no provider credentials or network; OpenHuman default/product builds exclude profiling-only controls. A documentation/check PR is sufficient; removal of this live gate is not the selected change.

## ComingSoon: remove four unsupported promises

[The workflow catalog](../../crates/openhuman-core/src/platform/about_app/catalog_workflows_automation.rs) lists `workflows.connect_google`, `workflows.connect_notion` and `workflows.connect_crypto_exchange` as ComingSoon. No controller or frontend implementation matching these advertised entries and their instructions was found. Generic implemented Composio/OAuth connections remain separate entries; do not promote these broad promises merely because a provider may appear in a connector catalog.

[The settings catalog](../../crates/openhuman-core/src/platform/about_app/catalog_localai_settings_mobile.rs) already has Stable `settings.clear_app_data` at “Log Out & Clear App Data”, followed by ComingSoon `settings.delete_all_data` at an absent “Delete All Data” setting. [The UI](../../app/src/components/settings/LogoutAndClearActions.tsx), [Tauri helper](../../app/src/utils/tauriCommands/core.ts), and [core reset implementation](../../crates/openhuman-core/src/config/ops/loader/reset_local_data.rs) implement the existing active-user local reset. Keep that capability and describe its actual scope; do not imply deletion of all tenants or remote backend data.

Remove the four named ComingSoon entries and their stale test expectations. Preserve implemented connection/reset entries and explicit documented limitations such as the Composio direct-mode trigger gap tracked by [#6128](https://github.com/tinyhumansai/openhuman/issues/6128). This is a decision about these four entries, not blanket removal of the ComingSoon status or a roadmap commitment.

Required proof: list/search/category/lookup/tool rendering omit all four ids; supported connection and active-user clear-data entries remain discoverable; reset isolation tests and UI confirmation behavior remain intact. Coordinate cloud reset work with [#7432](https://github.com/tinyhumansai/openhuman/issues/7432) without duplicating it.

## Profiles: gate SaaS orchestration, preserve desktop identity

There are two distinct kinds of “profiles”. [The SaaS domain](../../crates/openhuman-core/src/profiles/mod.rs) owns `ProfileHost`, provisioned profile registry, leases/fencing, credentials per tenant, gateway authentication, background recovery and `profiles.*` operator controllers. [The SaaS builder](../../crates/openhuman-core/src/core/runtime/saas.rs) installs that host and its background tasks.

[The shared ProfileLayout](../../crates/openhuman-core/src/config/schema/profile_layout.rs) also defines the desktop signed-in/pre-login user paths. [Desktop config directory resolution](../../crates/openhuman-core/src/config/schema/load/dirs.rs) uses it. [AuthProfilesStore](../../crates/openhuman-core/src/security/credentials/profiles.rs) is the independent provider/session credential store, not SaaS orchestration. Neither can be removed or gated out of desktop.

Some SaaS helpers are referenced from shared dispatch, tool assembly, approvals, sandbox, web-chat and detached completion paths. [Surface policy](../../crates/openhuman-core/src/profiles/surface.rs) explicitly leaves non-SaaS dispatch unrestricted, and [ensure_hosted](../../crates/openhuman-core/src/profiles/host.rs) is a no-op without a profile host. These cross-calls need a deliberate feature boundary; a feature around the entire directory alone would break compilation or isolation.

Introduce opt-in `saas-profiles` for the SaaS builder, operator domain/registry/gateway/leases/background orchestration and its tenant-only paths. Forward it through embed, TinyHumans and RPC, with deliberate CLI/server opt-in; exclude it from normal desktop/default product selection. Preserve shared config layout, credential store and ordinary single-user host behavior. Guard shared call sites explicitly, and reject SaaS startup when the capability is unavailable; never turn missing tenancy enforcement into a permissive SaaS stub.

Required proof: disabled root/app builds retain sign-in/local-user activation, user-directory isolation, BYOK/auth profiles, chat and approvals; enabled SaaS builds retain missing-scope rejection, operator/user method separation, tool allowlists, sandbox boundaries, lease-loss admission and completion fencing. Dependency measurement must demonstrate what is actually excluded. Coordinate existing lease/fencing changes and storage cleanup [#7435](https://github.com/tinyhumansai/openhuman/issues/7435) without replacing their scope.

## Follow-up verification and record validation

Behavior PRs run focused regression tests, `pnpm rust:layout`, root and desktop-manifest Cargo checks, affected Rust filters, and `node scripts/ci/check-feature-forwarding.mjs`. Enabled/disabled feature changes also run product graph checks and `scripts/assert-shed.sh` or `scripts/dep-sim.py` before claiming dependency reduction. Module migrations test actual platform artifacts. Host composition uses the existing mock/compose E2E runner; the issue names `scripts/e2e-core.sh`, which is absent in this checkout, so use `scripts/test-rust-e2e.sh` / `scripts/test-rust-with-mock.sh` as appropriate.

For this decision-only PR, validate every source link, follow-up URL and decision against the inspected files, Markdown formatting, `pnpm docs:check`, `pnpm rust:layout`, and the existing feature-forwarding check. Rust builds and behavioral E2E results belong to the implementation PRs; this record does not claim they have run.
