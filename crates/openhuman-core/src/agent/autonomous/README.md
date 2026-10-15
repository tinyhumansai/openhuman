# Durable desktop work

The desktop pump admits bounded turns against a persisted work record. Productive
work has no overall duration cap; cancellation, goal budgets, approval policy and
stale lease checks still apply. `autonomous_work start` also creates the goal.
Do not require a separate packed goal tool to start work.

## Recovery

A settled slice with no successful activity or an unchanged outcome fingerprint
increments `stagnant_turns`. The next prompt first asks for a diagnostic experiment,
then for decomposition or another permitted approach. Three stagnant slices block
rather than claiming completion or spending indefinitely. This is deterministic
recovery guidance, not a guarantee that a model follows it. Discovery-only calls
do not count as productive activity.

The shell scan-scope refusal gets one recovery opportunity in the tool-failure
middleware: choose a permitted bounded inspection. The denied scan never runs.
Actual approval denial and other policy failures retain their existing behavior.

## Review evidence

A separate reviewing slice must request completion, supply evidence for every
criterion, observe successful execution activity and have no failed tools or
pending worker changes. `file_write` and `apply_patch` clear earlier execution
activity, so checks preceding an observed edit cannot qualify the review.

These are necessary activity gates, not semantic proof of the acceptance criteria.
A successful arbitrary shell command does not establish correctness. Shell edits
and external filesystem mutations are not yet tracked as artifact revisions here.
Worker reports, model review scores and tool counts cannot replace tests through
the real delivered interface, visual inspection or domain-specific measurements.

The plan can express nested, dependent work with independent component reviews.
Expand difficult work as evidence warrants; depth alone is not a quality metric.
The Atlas reference inspired experiment/review/replan guidance, but its Python
engine, model grading, skill loader and resource-only sandbox are not integrated.

## Regression coverage

`service_tests.rs` covers durable continuation, cancellation, stale leases,
review admission, invalidated execution activity and staged stagnation guidance.
`../tinyagents/middleware_scan_recovery_tests.rs` checks bounded recovery while
retaining policy stops. Prompt tests enforce tool reachability and the original
8 KiB hermetic prompt budget.
