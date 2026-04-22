# Agent Observability for E2E

This doc describes the artifact-capture layer that makes the desktop app
inspectable by coding agents (Codex, Claude Code, Cursor) through the
existing WDIO/Appium/tauri-driver harness.

It is intentionally narrow: one canonical onboarding + privacy flow with
on-disk screenshots, page-source dumps, and mock backend request logs.
See `AGENT_OBSERVABILITY_PLAN.md` at the repo root for the broader plan.

## TL;DR

```bash
bash app/scripts/e2e-agent-review.sh
```

Artifacts land under:

```
app/test/e2e/artifacts/<ISO-timestamp>-agent-review/
  01-welcome.png
  01-welcome.source.xml
  02-post-welcome.png
  02-post-welcome.source.xml
  03-post-onboarding.png
  03-post-onboarding.source.xml
  04-privacy-panel.png
  04-privacy-panel.source.xml
  mock-requests-after-welcome.json
  mock-requests-after-onboarding.json
  mock-requests-after-privacy.json
  failure-<test>.png              # only on failure
  failure-<test>.source.xml       # only on failure
  meta.json                       # run metadata + checkpoint index
```

The script prints the resolved artifact directory at the end.

## Pieces

| Piece | Path | Role |
|-------|------|------|
| Helper | `app/test/e2e/helpers/artifacts.ts` | Run dir, `captureCheckpoint`, `captureFailureArtifacts`, `saveMockRequestLog` |
| WDIO hook | `app/test/wdio.conf.ts` (`afterTest`) | Always dumps screenshot + source on any failing test |
| Canonical spec | `app/test/e2e/specs/agent-review.spec.ts` | Welcome → onboarding → privacy panel with named checkpoints |
| Wrapper script | `app/scripts/e2e-agent-review.sh` | Build + run + print artifact dir |
| Stable selectors | `data-testid` on `OnboardingNextButton`, `Onboarding` overlay + skip button, `WelcomeStep`, `PrivacyPanel` | Agent-reliable navigation anchors |

## Environment overrides

| Variable | Effect |
|----------|--------|
| `E2E_ARTIFACT_DIR` | Force a specific run dir (skips auto-timestamped name) |
| `E2E_ARTIFACT_ROOT` | Parent dir for auto-generated run dirs (default: `app/test/e2e/artifacts`) |
| `E2E_ARTIFACT_LABEL` | Label used in the auto-generated run dir name (default: `run`; wrapper sets `agent-review`) |

## Using the helper from new specs

```ts
import {
  captureCheckpoint,
  saveMockRequestLog,
} from '../helpers/artifacts';
import { getRequestLog } from '../mock-server';

await captureCheckpoint('after-connect-click');
saveMockRequestLog('after-connect-click', getRequestLog());
```

`captureCheckpoint` numbers captures so the run dir reads chronologically.
`captureFailureArtifacts` is wired into `wdio.conf.ts` and fires
automatically on any failing test — specs should not call it directly.

## What is intentionally out of scope

- Visual baselines / image diffs across every component state.
- Screenshot capture on every click (too noisy).
- Live integrations (Gmail, Notion, Telegram); mock server only.
- New test framework / reporter.

Widen to more flows only after this loop proves out.
