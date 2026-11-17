# Connected integration cache scope (#7433)

Canonical base: `3dbdd07e5f07d917dabbbc93fb0a9dd2a24a4347`.

The existing credential-only cache key collides when two profile contexts use
one configuration and service identity. Invalidation clears every entry, and
connection reconciliation compares every cached profile with one response.

## Executed baseline

The three sibling regressions use actual `CoreContext` profile overlays, an
identical fake pinned Composio credential, and the production cache read,
invalidation and reconciliation functions. They seed cached snapshots and make
no backend request; this is not an authenticated backend-flow proof.

The supported fixture graph builds successfully in 5m43s and executes all three:
zero passes, three behavioral failures, zero ignored. Alice's snapshot is
visible to Bob; Bob's invalidation removes Alice's snapshot; Bob's reconciliation
removes Alice's snapshot. The failures are recorded in
`.validation/cache-scope-red-supported.log`.

The first minimal-feature attempt fails before execution because three existing
fixtures require the MCP, modules and channels gates. That compilation failure
is not counted as RED.

## Candidate and acceptance

The candidate binds cache entries to the profile/operator storage identity,
preserves the existing credential digest, and uses per-scope epochs for fetch
admission. Desktop agents continue to share their operator's catalog. Missing
SaaS profiles fail closed, including before the disabled-mode shortcut.
Settings refresh preserves its caller context with `spawn_scoped`.

The original three assertions remain unchanged. Additional controls cover
in-flight fetch admission, missing SaaS profiles and desktop agent sharing.
The first GREEN attempt catches an obsolete helper import from the retained
branch; the canonical `contract::toolkit_description` path has been restored.
The fresh run after restoring that path builds successfully in 3m47s and passes
all six controls: zero failures and zero ignored. The isolated SaaS child also
passes its one selected test; it is included in the six parent cases, not an
additional distinct case. The full log is
`.validation/cache-scope-green-restored.log`.

Broader acceptance runs the complete Composio module test filter: 361 passes,
zero failures and zero ignored. This includes credential rotation, mocked fetch
flows, existing reconciliation controls and all six new scope cases. The five
scoped-spawn controls also pass. Their logs are
`.validation/cache-composio-acceptance.log` and
`.validation/cache-spawn-acceptance.log`.

All three successful commands use `cargo test --locked -j1 -p openhuman
--no-default-features --features storage-sqlite,mcp,modules,channels --lib`, with
the corresponding module filter and `-- --test-threads=1`, through
`scripts/ci-cancel-aware.sh`. These process-cache tests do not prove MongoDB,
multi-core routing or complete cloud operation. Changed-line coverage remains
for CI to measure.

Independent source review found no Critical or Important issues. It checked
cache/epoch lock ordering, scope admission and the refresh context capture.
The inherited reconciliation read-to-write gap can still evict a newer snapshot
within the same scope, causing a safe cache miss; that gap is not fixed here.

Review follow-up also exercises enabled direct-mode fetch without a SaaS
profile, before the existing disabled-mode assertion. Identical fake credentials
use loopback-only endpoints, and the seeded profiled snapshot uses the same
enabled configuration. A fresh supported-graph run passes all six controls,
with zero failures and zero ignored; the isolated child is a repeated case.
The full log is `.validation/cache-review-enabled-green.log` (build 2m59s).

The next review identifies missing scope diagnostics and retained idle epochs.
Before either correction, the supported graph builds in 2m18s and reports six
passes and two genuine failures: 128 dormant epochs remain after sequential
scope churn, and absent-scope fetch/mutation emits no refusal diagnostics.
The isolated child repeats the diagnostic failure; it is not a third case.
The log is `.validation/cache-review-epoch-log-red.log`.

Epoch acquisition now reclaims only map-owned Arcs under the epoch-map write
lock. Every in-flight fetch keeps its external Arc across the backend await and
cache admission, so reclamation cannot reset its invalidation fence. Epoch
history is bounded by live holders plus the latest idle entry; snapshot cache
residency is a separate concern. The refusal paths log fixed operation names,
error kinds or the content-free NoTenant error, without tenant identities,
credentials or paths.

With the same assertions, the fresh focused run builds in 2m19s and passes all
eight cases, with zero failures and zero ignored. A fresh complete Composio run
passes 363 cases, also with zero failures and zero ignored. The child repeats
one case. Logs are `.validation/cache-review-epoch-log-green.log` and
`.validation/cache-review-composio-green.log`. Independent source review finds
no issues in epoch reclamation, lock ordering or sanitized diagnostics.

Feature forwarding passes. Storage-bypass and Rust layout checks fail on files
unchanged from the canonical base: the browser-profile lock probe is reported as
a write, and three security files exceed the line limit. Exact base-to-candidate
diffs of the scripts and reported files are empty.

## Related boundaries

The storage `KEYS` cache holds the process master provider, while native secret
operations derive keys for their captured scopes. Source review found no tenant
data-key cache there. Existing explicit-provider tests do not establish the
complete production `current()` path.

Legacy secret adoption still needs source ownership admission. Serialized
connection events also need trustworthy profile propagation for scoped cache
refresh; a missing profile is refused. This bounded cache change does not
complete the singleton inventory or the cloud-storage epic.
