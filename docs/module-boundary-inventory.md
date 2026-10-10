# Loadable module boundary inventory

Status: migration incomplete. This inventory records the pinned source at
`ad89cddbca`; it does not assert that the host already uses only contracts.
The machine-readable dependency policy is
[`scripts/ci/module-boundaries.json`](../scripts/ci/module-boundaries.json).

Every owner below is the canonical upstream under
`https://github.com/tinyhumansai/`. Contract changes land there first. OpenHuman
host adapters switch only after compatible per-platform artifacts are released
and their digests pinned. Do not remove a dependency by removing its capability,
copying its implementation into a host, or accepting weaker isolation.

| Owner | Linked implementation packages | Contract | Host consumers | Required module work |
| --- | --- | --- | --- | --- |
| tinyjuice | tinyjuice | tinyjuice-bus | core tools, compression, CCR REPL | HTML extraction, query supplied artifacts against module CCR store, schema and tool declarations; retain turn-bound model callbacks |
| tinyconnectors | tinyconnectors, tinyconnectors-sync | tinyconnectors-bus | core integrations, credentials and triggers | Argument preparation, calendar defaults, task windows, structured provider errors, trigger archives; preserve sign-in/out reconciliation |
| tinymcp | tinymcp | tinymcp-bus | core MCP registry, supervisor, CLI stdio/HTTP server | Supervisor notifications, server protocol operations and callbacks for approved host tools/resources/prompts |
| tinychannels | tinychannels, tinychannels-runtime; runtime/crypto code in contract | tinychannels-bus | core channels and podcast email, TinyHumans host, CLI REPL | Move providers, signing and pairing out of contract; relay config, pairing, start/stop/send/status, inbound/status callbacks, bounded delivery/draining |
| tinyhosts | tinyhosts | tinyhosts-bus (missing at pin) | core hosting tools | Extract vocabulary and tool declarations; consume Execute/Providers after host validation/approval |
| tinywallet | tinywallet-crypto, tinywallet-web3, tinywallet-x402 via bus and direct imports | tinywallet-bus | core wallet/web3/x402 | Move behavioral re-exports out of bus; validation, transaction construction, quotes, swaps, payments, budgets, ledger; retain host custody/approval and confidential attestation |
| tinybox | tinybox-core, tinybox-jail, tinybox-docker, tinybox-host, tinybox-ssh | tinybox-bus (missing at pin) | core sandbox and security, desktop gateways | Discovery-only module needs handle-based sandbox/exec/streams/cancel/files/forward/status/close and shell-analysis facts; pre-core gateways use process loader |
| tinycomputer | tinycomputer-accessibility | tinycomputer-bus | core voice, paste/focus and permission checks | Permission, focus, paste, Globe listener vocabulary/operations; preserve native thread and target rules |
| tinyvoice | tinyvoice, cpal (also through accessibility probe) | tinyvoice-bus | core voice capture/hotkeys | Devices, recording/capture and hotkey lifecycle, bounded event batches; computer module permission decisions |
| tinyruntime | tinyruntime-pyserver | tinyruntime-bus | core Python worker, optional TinyJuice ML | Excluded from this migration: TinyRuntime is being removed separately |
| tinydocs / tinymemory | pdf-extract, calamine through tinymemory-integrations/documents-office | tinydocs-bus | core memory converter and file sources | Replace OfficeConverter with bus DocumentConverter; XLSX extraction plus existing PDF/DOCX/PPTX metadata/format coverage |
| tinysearch | none observed | tinysearch-bus | core module search proxy | Preserve existing bus adapter; enforce contract and host dependency graphs |

## Gate behavior

`pnpm rust:module-boundaries` checks the resolved normal and build dependency
closure of core, CLI, TUI and the excluded desktop workspace with all features.
Dev-only dependencies are excluded, including the test-only TinyWallet key
implementation. Package IDs, not dependency aliases, determine graph identity.
Target-specific dependencies are retained by Cargo metadata without a platform
filter. A new implementation package within an inventoried owner namespace
fails unless it is a registered contract.

Contracts are resolved separately in generated probe workspaces, once with
default features and once with every declared feature. Only serialization,
schema and error-derive closures are approved. This catches optional runtime or
implementation imports independently of host feature selection. Probes do not
compile native module code, and their manifests and lockfiles are kept under
`target/module-boundaries/` in the checkout.

Exceptions are exact package/scope pairs with reasons. Host exceptions still
traverse their descendants. Contract exceptions stop at the first unsafe edge:
its implementation closure is not approved, and that edge must be removed with
the owning migration. The channel and wallet contract migrations remain
explicitly outstanding. An unused exception fails, prompting removal instead
of leaving future regressions silently exempted.

`pnpm rust:module-boundaries:complete` also fails while any exception or pending
contract exists. Passing the transitional gate means no unlisted boundary
violation was observed; it does not mean this plan is complete.

## Remaining acceptance work

The shared `openhuman_rpc::embed::modules::ModuleClient` takes explicit runtime
configuration and uses the same process-wide loader before or after core
startup. It has no linked fallback. Resolution reports and client errors use a
closed reason vocabulary and registry metadata. Cached resolution failures and
native terminal unavailability are deduplicated; independent failed invocations
each produce an event. Cached callers revisit reporting if resolution finished
before a Sentry client was bound. An unfinished loader wait produces no terminal
event. Reports clear request scope and exclude raw loader paths and remote fault
prose. Existing domain adapters still need to migrate through this reporting path.

Upstream operations and artifacts, gateway integration, frozen tool restoration,
lifecycle regressions and coverage of all migrated adapter failures are still
required. No capability migration has landed in this change. The minimal/default/product/individual-feature build matrix,
platform compilation, runtime bus fixtures and shed measurements remain
separate acceptance checks. Dependency counts alone do not demonstrate a
build-time or binary-size improvement.

## Upstream migration work

Owner changes are independently reviewable; host dependencies and artifact pins
remain unchanged until compatible upstream releases are available.

| Change | Canonical PR | Local verification |
| --- | --- | --- |
| Async HTML extraction seam, deadlines and generic tool-metadata sanitization in TinyTools | [tinytools#60](https://github.com/tinyhumansai/tinytools/pull/60) | 1,183 workspace tests and six doctests; clippy/build; independent sanitizer review accepted; all 113 source files at least 90% coverage |
| TinyJuice typed CCR/content queries, HTML extraction, pure schemas and declarations | [tinyjuice#59](https://github.com/tinyhumansai/tinyjuice/pull/59) | 737 tests; dynamic artifact E2E; module input/limit guards |
| Complete TinyDocs Markdown conversion for memory ingestion | [tinydocs#31](https://github.com/tinyhumansai/tinydocs/pull/31) | 183 tests; dynamic artifact E2E; per-file coverage at least 90% |
| TinyHosts pure vocabulary and authorized source preparation with captured deployment bytes | [tinyhosts#21](https://github.com/tinyhumansai/tinyhosts/pull/21) | 210 all-feature and 179 default tests including doctests; compiled snapshot and large legacy deployment probes; pure contract audit; all 19 implementation files at least 90% coverage; independent preparation review accepted |
| TinyComputer native permissions, confidential focus/paste and reliable Globe leases/read/shutdown | [tinycomputer#87](https://github.com/tinyhumansai/tinycomputer/pull/87) | 1,372 all-feature and 1,367 default tests; 90-member compiled artifact verification; independent ownership and event-loss review accepted; macOS cross-check and 233-file coverage gate pass; physical macOS helper not exercised on Linux |
| TinyConnectors argument preparation, task filtering, structured provider errors and leased archives | [tinyconnectors#46](https://github.com/tinyhumansai/tinyconnectors/pull/46) | 423 tests; dynamic artifact calls; user archive lifecycle fixtures; pure contract audit; per-file coverage at least 90% |
| TinyChannels contract vocabulary separated from provider, relay, pairing and runtime behavior | [tinychannels#56](https://github.com/tinyhumansai/tinychannels/pull/56) | 1,174 default and 1,180 all-feature tests; independent contract audit and relocation review accepted; bus files at least 98.65% coverage; legacy provider/worker coverage gaps disclosed |
| TinyBox pure contract, shell facts, reserved sandbox/process handles and acknowledged cleanup/shutdown | [tinybox#30](https://github.com/tinyhumansai/tinybox/pull/30) | 710 default and all-feature tests; 57 source files at least 90% coverage; compiled native-process artifact verification; independent lifecycle review accepted; supervised execution currently limited to Unix passthrough |
| TinyWallet pure contracts, address validation and stateless EVM construction with exact approval facts | [tinywallet#56](https://github.com/tinyhumansai/tinywallet/pull/56) | 745 all-feature and 601 default tests; four-chain signing and native/ERC-20/contract construction through the compiled artifact; pure contract audit; all 95 source files at least 90% coverage; independent construction review accepted |
| TinyVoice device enumeration, recording/continuous capture, reserved hotkey leases, replayable batches and acknowledged shutdown | [tinyvoice#23](https://github.com/tinyhumansai/tinyvoice/pull/23) | Contract 1.4; 32-member compiled artifact; 88 module tests; independent hotkey, Windows owner and Xvfb cleanup fixtures; pure contract audit, own-module formatting and Windows GNU test cross-check; formatted module source coverage 1,592/1,743 (91.34%), with the existing physical-device exclusion; no physical Windows/MSVC or macOS input validation |
| TinyRuntime pure JSONL worker vocabulary, persistent cache recipes, optional provider preparation and module-owned lifecycle | [tinyruntime#29](https://github.com/tinyhumansai/tinyruntime/pull/29) | 244 router unit tests, 6 default / 7 all-feature public API tests, 64 contract tests, 35 pyserver tests and doctests; 13-member compiled artifact verifies old-provider compatibility, preparation, cache reuse, adoption and rebuilding; per-file coverage gate passes; independent cleanup, cache and provider-bridge reviews accepted |
| TinyMCP pure shared vocabulary, supervisor observations, server callbacks and bounded text/argument/tool rendering operations | [tinymcp#54](https://github.com/tinyhumansai/tinymcp/pull/54) | 1,471 all-feature and 1,359 default tests; dynamic 44-member artifact verification; pure contract audit; all 93 source files at least 90% coverage; independent lifecycle, vocabulary and library-feature reviews accepted; external regressions cover module-disabled and host feature combinations |

The TinyDocs and TinyJuice operations need new published module artifacts.
TinyHosts preserves existing member arities and adds authorized preparation inside
Execute; consuming that operation requires a new published artifact. No local build digest
has been used as a release pin, and these PRs do not yet remove any host exception.

TinyChannels preserves its serialized vocabulary and moves behavioral APIs to
implementation crates, using compatibility extension traits where needed. Its
relay/pairing/delivery operations now have owning-module implementations in PR 56,
including bounded queues, replayable results and shutdown reconciliation. The
OpenHuman adapters remain outstanding.
Published v0.1.13 contains the earlier implementation-bearing contract, so that
release does not unblock this contract cut.

TinyBox currently refuses supervised execution on unsupported backends and platforms.
Docker supervision is implemented in PR 30 and still undergoing lifecycle review.
Namespace/SSH and native Windows supervision, streaming, file transfer, forwarding
and gateway operations remain required before migrating its host callers.

TinyRuntime is now excluded from the active migration because it is being removed
separately. The previously published owning PR remains recorded above; the frozen
Python-provider work has not been published or integrated. The boundary inventory
still records existing dependencies until the separate removal reaches OpenHuman.

TinyHosts’ bounded directory preparation currently returns up to 4 MiB of source.
Existing larger Launch/Deploy requests keep their transport budget. Streaming or
module-owned prepared artifacts remain required for larger host directory inputs.

TinyComputer’s reliable Globe reads retain one bounded snapshot until acknowledgment.
Native overflow, malformed events and unexpected EOF mark continuity loss. Hosts
must await its terminal shutdown before unloading. TinyVoice owns native Linux and
Windows hotkey listeners and accepts sequenced generic facts for macOS. Its reads
retain the previous batch across continuity loss, allowing a valid acknowledgment
to retrieve the inactive reset snapshot. Shared activation types live in the bus
contract and are re-exported by the implementation. OpenHuman still needs the
Computer-to-Voice adapter and compatible released artifacts before switching.

TinyDocs PR 35 moves PNG/JPEG header interpretation into the module and adds
bounded typed image inspection. Its parser regressions and rebuilt artifact
verification pass. The source API change requires a minor package release;
host presentation and document ingestion still await the approved release.

## Current delivery gates

The module PR babysitters repair CI and review findings and merge only after
checks, approval, resolved feedback and mergeability all pass. TinyRuntime is
excluded from that work. CI success does not clear an old changes-requested
review, and resolved threads do not establish independent approval.

TinyConnectors PR 46 is merged; its canonical main CI passed. The minor release
[run 38084368799](https://github.com/tinyhumansai/tinyconnectors/actions/runs/38084368799)
is building compatible platform artifacts. Host adoption remains gated on
published packages and verified digests. No host dependency cut is claimed here.
