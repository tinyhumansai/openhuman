# Subconscious Loop Test Fixtures

Two temporal sets simulating state changes between ticks.

## Tick 1 (initial state)

- `tick1_gmail.txt` — 3 emails: deadline reminder (April 3), CI notification (routine), meeting invite
- `tick1_notion.txt` — Project tracker: 3 threads (memory=in progress, skills=blocked, ingestion=complete)
- `heartbeat.md` — 3 periodic tasks

### Expected tick 1 behavior

- **Escalate**: Deadline reminder (April 3) — actionable, time-sensitive
- **Noop**: CI notification — routine, no action needed
- **Noop or act**: Meeting invite — informational, could store to memory
- **Noop**: Notion tracker — no urgent changes
- Decision log should record the deadline escalation with source doc ID

## Tick 2 (state change — 6 hours later)

- `tick2_gmail.txt` — 2 new emails: deadline MOVED UP to April 2 (urgent), skills unblocked
- `tick2_notion.txt` — Tracker updated: Thread 2 unblocked, deadline decision changed

### Expected tick 2 behavior

- **Skip**: Original deadline email (tick1) — already surfaced in tick 1
- **Escalate**: New deadline-moved email — different doc, more urgent (tomorrow!)
- **Act**: Skills unblocked email — store to memory, update known state
- **Act or escalate**: Notion tracker change — Thread 2 status changed, deadline decision changed
- Decision log should NOT re-surface the original deadline

## Tick 3 (no new data)

- No new fixtures ingested
- **Expected**: Noop — delta is empty, skip inference entirely
