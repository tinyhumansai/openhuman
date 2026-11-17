# Detached-worker profile ownership

This bounded host correction starts from canonical
`3dbdd07e5f07d917dabbbc93fb0a9dd2a24a4347`. It addresses the process-local
detached-worker ownership boundary in #7433. It does not consume new TinyAgents
capabilities or change storage declarations, credentials, dependencies, or
the connected-integration cache.

## Boundary

The host adapts TinyAgents' opaque registry owner string using the existing
injective profile/session encoding. Registry ownership includes the original
profile and parent session. Durable task records, completion routing and
public responses retain the original unencoded session ID.

Registration captures the actual scoped profile. SaaS work without a task
context or admitted profile is refused; an unowned registration aborts the
spawned task before recording it. Live rosters, resolution, wait and agent
steering use the caller's profile/session key. Durable fallback paths also
require the caller's actual captured workspace.

Public user controls do not require a parent-session argument. They compare
the worker's captured profile with the caller's profile, then use TinyAgents'
owned cancellation/steering APIs with the snapshot's exact owner. Thread
stop/delete/purge filters retain their workspace restriction and also check
the captured profile. Desktop public controls retain the process user's
authority over all workers.

TinyAgents continues to own the registry, status channels, abort/cancellation
mechanics and steering handles. The host change supplies ownership metadata
and admission checks around those APIs.

## Regression evidence

Test-only checkpoint `26a7611387` executed against the uncorrected production
source through cancel-aware session `38040`. Compilation succeeded in 8m06s;
the seven selected tests finished with **1 passed, 6 failed, 0 ignored**.
The desktop control passed. All six tenant tests failed on actual behavior:

- identical parent-session IDs exposed both profiles' live workers;
- a foreign worker resolved under the other profile;
- foreign agent steering succeeded;
- foreign public steering and cancellation returned success;
- a missing SaaS task scope could read the live roster.

Each SaaS case uses a fresh libtest subprocess because process mode is fixed
by a `OnceLock`. The fixtures use actual `CoreContext` profile overlays,
distinct workspaces, live pending Tokio tasks, status watches and public
registered controller handlers. The parent requires both child exit success
and a one-test success result, preventing an empty test filter from passing.

Corrected source `9b1191ee4e` ran through session `65302`. Compilation succeeded
in 2m46s; the complete detached-worker suite finished with **29 passed,
0 failed, 0 ignored**, including the original seven cases and existing
desktop cancellation, steering, wait, roster, durable fallback and wire-shape
controls. The original failing assertions were retained. The missing-scope
case additionally checks a scoped kernel context without a profile.

The separate public-controller compatibility suite finished with **5 passed,
0 failed, 0 ignored**. These 34 results are distinct top-level tests; the seven
subprocess repetitions are not added to the count. Both runs used:

```sh
scripts/ci-cancel-aware.sh cargo test --locked -j 1 -p openhuman \
  --no-default-features --features storage-sqlite,mcp,modules,channels \
  --lib agent::orchestration::running_subagents::tests \
  -- --nocapture --test-threads=1

scripts/ci-cancel-aware.sh cargo test --locked -j 1 -p openhuman \
  --no-default-features --features storage-sqlite,mcp,modules,channels \
  --lib agent::orchestration::subagent_control::tests \
  -- --nocapture --test-threads=1
```

Complete outputs were read and retained outside disposable build output in
`.validation/detached-tenant-red.log`,
`.validation/detached-tenant-green-and-compat.log` and
`.validation/detached-control-compat.log`. Independent source review found no
Critical or Important findings in the bounded ownership correction.

## Review follow-up: actual dispatch and spawn refusal

The follow-up normally merged canonical `d1efa8bd58` without rewriting
the earlier checkpoints. Test-only source then exercised the actual registered
SaaS dispatch gate and the live `SpawnAsyncSubagentDispatch`, rather than
calling only the controller handlers. Session `8074` compiled successfully
in 2m07s and finished with **7 passed, 2 failed, 0 ignored**:
the SaaS steer method was hidden, and a missing-profile spawn returned
an accepted reference after persisting a Running reusable session.

Both original assertions were retained. The fix admits the Agent domain in
the SaaS/profile presets, while the exact user method allowlist admits only
the reviewed cancel/steer controls. The dispatch regression also proves an
unreviewed, registered Agent method remains denied. Spawn checks the actual
profile/workspace before durable session or worker-thread writes.
Registration now returns a typed refusal, aborts the unowned worker, and
creates its Running task ledger/watcher only after successful registration.
A late refusal settles an already persisted reusable session as Failed before
any accepted response is constructed.

The first postfix attempt `7255` failed compilation because the host adapter
used the lower registry's error type instead of the actual runtime API's
`TinyAgentsError`. That source type was corrected; no behavioral result is
claimed for the compiler failure. Retry `22704` compiled in 2m02s and passed
all nine tenant tests. Its two unused Result warnings in existing fixtures
were subsequently repaired by asserting admitted registration.

An overbroad orchestration run `1071` selected 339 tests and was explicitly
cancelled through the cancel-aware wrapper (exit 143) during an unrelated
team-worker retry test. Its incomplete output is preserved and excluded from
acceptance. The narrower runner `37427` passed running-worker 31, controller
5, spawn 20 and profile-surface 8 tests, including the actual late-refusal
abort/Failed-state control. Its existing SaaS preset equality test still
expected Agent to be disabled, producing 14 passes and one failure. The
expected preset was updated only for the deliberately admitted Agent domain;
the actual unrelated-Agent dispatch refusal remains enforced.

These outputs are retained outside disposable targets in
`.validation/detached-review-red.log`,
`.validation/detached-review-green.log`,
`.validation/detached-review-green-retry.log`,
`.validation/detached-review-orchestration-green.log` and the individual
`.validation/detached-review-{running,control,spawn,surface,saas}.log` files.
The final narrow rerun `78777` finished with exit 0 at tested source
`f64060e800`: **79 distinct top-level tests passed, 0 failed, 0 ignored**
(running-worker 31, controller 5, spawn 20, profile-surface 8, SaaS preset 15).
Compilation finished in 2m17s. Nested subprocess repetitions are excluded
from that total. The preset failure is separately preserved in
`.validation/detached-review-saas-preset-red.log`, and the final runner
summary is `.validation/detached-review-focused-final-progress.log`.
The 79 results do not establish full-workspace, default/all-feature, MSRV,
coverage or hosted acceptance.

The response/refusal helpers were mechanically extracted into a sibling file,
bringing the execution fragment from 807 to 749 lines. Its now-stale legacy
layout exception was removed. The final layout scan still exits 1 for five
unchanged canonical over-limit files (`security_gate.rs`, its tests,
`mcp/module_client.rs`, its tests and the policy paths/risk tests).
The output is preserved in `.validation/detached-review-layout-final.log`;
this follow-up does not claim a whole-repository layout pass.

## Remaining limits

These results cover the supported feature graph above. They do not assert a
fresh full-workspace, all-feature, MSRV, coverage or hosted CI result.

Abort handles, steering queues and live status remain process-local. A shared
durable ledger does not route a request to another executor or provide a
distributed turn claim. Cross-core routing, native module releases, host
consumption, storage migration and complete cloud acceptance remain separate
work.
