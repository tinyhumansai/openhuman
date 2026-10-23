# Security phase 0: characterization inventory (issue #7328)

Phase 0 pins today's host behaviour so the move into the `tinysecurity` bus
module cannot change it by accident. A test here describes what the code does
now. Behaviour we want but do not have is written as a `desired_*` test that
must panic with `DESIRED-RED`. When the module-owned implementation lands, the
panic stops, the test fails, and the owner of that change flips it to a normal
test. Nothing is deleted to go green.

## Targets

| Target | Covers |
| --- | --- |
| `tests/security_characterization_e2e.rs` | RPC wire shapes for approval, security, sandbox and encryption; in-process policy latency baseline |
| `tests/security_policy_characterization.rs` | shipped defaults, `from_config` grants, gate-decision matrix, disabled floor, trusted roots, hourly limiter |
| `tests/security_approval_characterization.rs` | park/decide, 600s and 180s windows, fail-closed expiry, restart survival, cron/unknown denial, `auto_approve_all`, flow trust |
| `tests/security_redaction_characterization.rs` | both redaction corpora across seven redactors, recorded gaps |
| `tests/security_sandbox_characterization.rs` | `resolve_sandbox_policy` matrix, host off switch, `command_requires_sandbox` precedence |
| `tests/security_crypto_characterization.rs` | Argon2id/AES-GCM and X25519/HKDF/XChaCha20 fixtures, keyring fixture digests |
| `security/policy/policy_characterization_tests.rs` | command classes, allowlist per tier, risk levels, always-forbidden table |
| `sandbox/ops_tests.rs` (`saas_requires_the_sandbox_...`) | SaaS sandboxes every command even with `OPENHUMAN_SANDBOX=off` (isolated process) |
| `modules/security_native_tests.rs` | real cdylib dispatch, representative-turn latency record and the release budget |

Redactors that are private to their module are covered beside it, on the same
corpus: `integrations/composio/tools_tests.rs` and
`agent/tinyagents/middleware/credential_scrub_tests.rs`. The URL helpers are
covered by the floor (`core/log_redaction_tests.rs`) and, for the module's
endpoint forms, `security/redact_tests.rs` against the released archive. The
`util::redact`, `security::scrub`, `config::schema::storage` and per-domain
URL helpers were deleted.

## Desired-RED tests (open)

| Test | Wanted behaviour |
| --- | --- |
| `desired_auto_approve_all_records_an_audit_row` | a bypassed call leaves an audit row (none today, accepted in #5634) |
| `desired_every_required_sink_removes_every_secret_and_pii_shape` | no secret/PII shape survives the log, scrub, denial, approval and pii sinks |
| `desired_allowlist_denial_does_not_echo_command_arguments` | a command-policy denial never echoes secret-bearing arguments |
| `desired_encrypt_refuses_payloads_over_the_binary_limit` | crypto has an explicit binary payload size limit (plan Task 10) |

The recorded gap list is `tests/fixtures/security-redaction-known-gaps.json`.
It may shrink as redactors improve. A regression that grows it fails
`known_missed_secret_gaps_are_recorded_exactly`.

## Fixtures

Committed fixtures are never regenerated to make a test pass:
`tests/fixtures/security-crypto-corpus.json`, the redaction corpora, and
`secret_storage/fixtures/{secrets_enc.bin,enc2_values.txt}` (digests pinned in
the crypto target).

## Latency gate

The owner commits the release-build budget in
`vendor/tinysecurity/docs/performance.md` (tinysecurity#8): p50 <= 1 ms and p99
<= 10 ms per Evaluate or Check. The host enforces it on its own path:
`OPENHUMAN_SECURITY_PERF_BUDGET=1 bash scripts/ci/security-native-fixture.sh`
runs the native test under `--release`, fails when the per-call p50 or p99 passes
the budget, and writes `target/security-perf.json` with the profile, runner,
workload, calls per turn and the p50/p99 per call and per turn. The CI job is
`Release latency budget` in `.github/workflows/security-native.yml`.

The workload is a scripted shell turn: three reads, one parent-create, one
denied probe, so five bus calls per turn with a warm registration. It is a
stand-in until product conversations are traced; replace it with a measured
turn once the callers move (plan Task 12).
