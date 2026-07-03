# 08.3 — Approvals as durable graph interrupts

Today approval pauses are steering-channel pauses (park the turn) and the
approval gate parks interactive chat turns with a 10-min TTL.

## Steps

1. For durable graphs (delegation review gate, workflow human-review
   phases): emit `NodeResult::Interrupt { id, node, payload }` with the
   approval request as payload; persist via checkpointer (Sync durability).
2. Resume path: approval RPC decision → `Command { resume: Some(decision) }`
   at the stored `ResumeTarget`; process restart between request and
   decision must survive (that's the point).
3. Keep the interactive chat-turn approval gate as-is (steering pause) —
   chat turns are not durable graphs; document the boundary here.
4. Surface pending interrupts in command center (`GraphRunStatus` +
   interrupt records); TTL expiry → resume-with-deny.

## Deletions

- Bespoke pause/park bookkeeping in workflow/delegation paths where the
  interrupt record replaces it.

## Acceptance

- e2e: delegation review pause → restart process → approve → run resumes
  from exact checkpoint; deny/TTL paths covered.
