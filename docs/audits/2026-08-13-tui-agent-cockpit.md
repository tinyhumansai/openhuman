# TUI agent-cockpit audit

Reference: the interaction model of OpenAI Codex's terminal UI, adapted to
OpenHuman's existing RPCs and four-tab product structure. This is behavioral
parity, not a visual clone.

## Gaps found in the previous TUI

The previous terminal client opened on Logs, created a new thread on every
launch, accepted only a single-line prompt, and exposed only send/cancel/new.
It did not restore transcripts, scope events by thread, steer an active turn,
surface approvals or plan review, select models or agent profiles, expose
goals/tasks/skills/MCP/artifacts, inspect Git changes, insert workspace paths,
copy/export answers, or accept a launch prompt.

## Implemented cockpit surface

- Chat is the default tab; Logs, Config, and Settings remain available through
  `Alt+1-4` and `Ctrl+Tab`.
- Saved-thread resume (`/resume`, `--resume`, `--last`), transcript restoration,
  rename/delete/new, and strict client-and-thread event isolation.
- Unicode-safe multiline composer, history, fuzzy slash-command completion,
  paste, cursor editing, and bounded `@path` insertion from `action_dir`.
- Active-turn steering on Enter, queued follow-up on Tab, cancel on Escape, and
  queue status in `/status`.
- Blocking approval and plan-review overlays, including approve-once, durable
  tool/flow approval, deny, approve/reject plan, and revise with feedback.
- OpenHuman-native views for usage, goals, task board, agent profiles, Skills,
  MCP servers, and artifacts; agent profiles and autonomy tier can be changed
  in place.
- Structured live tool, sub-agent, artifact, reasoning, retry, and error rows.
- Optional Git status/diff and review prompt, OSC-52 answer copy, Markdown
  transcript export, model override, direct positional prompts, and
  `--no-alt-screen`.

## Intentional differences

- The four established OpenHuman tabs remain the top-level navigation.
- File references are text paths rooted in `action_dir`; binary/image
  attachments are not introduced.
- Git controls appear on demand and report a clear non-repository state.
- Skills and MCP views follow their existing compile-time feature gates and
  return the normal RPC error surface in slim builds.

## Verification

The TUI reducer, composer, rendering, picker, and RPC-name contract have focused
unit coverage. The slim feature test uses `--no-default-features --features tui`;
an additional registry test enables `tui,skills,mcp` to cover gated commands.
