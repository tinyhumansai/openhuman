# E2E Test Suite Status

Living tracking document for the OpenHuman E2E test suite. Updated whenever
specs are added, fixed, or start failing.

**Last updated:** 2026-05-20
**Total specs:** 66 (11 categories)
**Runner:** WDIO + Appium Chromium on the CEF desktop binary

---

## Suite health overview

| Category      | Specs | Known issues |
|---------------|-------|--------------|
| auth          | 6     | Hardcoded pauses replaced with condition waits (2026-05-20) |
| navigation    | 6     | channels-smoke and insights-dashboard are shallow/smoke only |
| chat          | 10    | chat-harness-wallet-flow has 6 sequential 30s waits |
| skills        | 6     | skill-execution-flow is RC-7 (ghost RPCs); 4 specs are shallow stubs |
| notifications | 4     | memory-roundtrip has async indexing race |
| webhooks      | 5     | webhooks-ingress-flow missing payload delivery assertion |
| providers     | 8     | telegram-flow is describe.skip; gmail/slack/whatsapp miss multi-account |
| payments      | 4     | rewards-progression-persistence has hardcoded pauses |
| settings      | 7     | settings-ai-skills uses OR-chain assertions |
| system        | 4+1L  | local-model-runtime is describe.skip; voice-mode has hardcoded pauses |
| journeys      | 3     | All moderate depth |

L = Linux-only spec

---

## How to update this document

- **Adding a spec**: add it to the coverage matrix below and to `e2e-run-all-flows.sh`
- **Fixing an issue**: strike through the entry or remove it from Known Issues
- **A spec starts failing**: add it to the Known Issues section with severity + status tag
- **Pre-flight check**: `bash app/scripts/e2e-preflight.sh`

---

## Coverage matrix

### Auth (6 specs)

| Spec | Feature covered | Coverage depth | Known issues |
|------|----------------|----------------|--------------|
| smoke.spec.ts | Harness bootstrap, app loads | deep | |
| login-flow.spec.ts | Deep-link auth → onboarding → home | deep | |
| auth-access-control.spec.ts | Billing dashboard handoff | moderate | Previously had hardcoded 5s/8s pauses — replaced 2026-05-20 |
| logout-relogin-onboarding.spec.ts | Logout + re-login round-trip | moderate | |
| onboarding-modes.spec.ts | Onboarding step sequence | moderate | config.toml write race on slow CI |
| runtime-picker-login.spec.ts | Core mode selection + login | moderate | Deep-link bootstrap race |

### Navigation (6 specs)

| Spec | Feature covered | Coverage depth | Known issues |
|------|----------------|----------------|--------------|
| navigation.spec.ts | Tab bar + route rendering | deep | |
| navigation-smoothness.spec.ts | Transition timing | moderate | |
| navigation-settings-panels.spec.ts | Settings panel routing | moderate | |
| command-palette.spec.ts | Command search | moderate | |
| channels-smoke.spec.ts | Channels surface mount | shallow | No channel feature validation |
| insights-dashboard.spec.ts | Insights panel | shallow | No data validation |

### Chat (10 specs)

| Spec | Feature covered | Coverage depth | Known issues |
|------|----------------|----------------|--------------|
| chat-harness-send-stream.spec.ts | Send → SSE stream → UI render | deep | |
| chat-harness-cancel.spec.ts | Cancel mid-stream | deep | |
| chat-harness-scroll-render.spec.ts | Scroll + render correctness | moderate | |
| chat-harness-subagent.spec.ts | Subagent invocation | moderate | |
| chat-harness-wallet-flow.spec.ts | Chat + wallet state | moderate | 6 sequential 30s waits; should use condition waits |
| chat-tool-call-flow.spec.ts | Function calling roundtrip | deep | |
| chat-multi-tool-round.spec.ts | Multi-turn tool loop | deep | |
| chat-tool-error-recovery.spec.ts | Tool error handling | deep | |
| agent-review.spec.ts | Agent review + feedback | moderate | |
| mega-flow.spec.ts | Full journey (auth/oauth/chat/logout) | deep | |

### Skills (6 specs)

| Spec | Feature covered | Coverage depth | Known issues |
|------|----------------|----------------|--------------|
| skills-registry.spec.ts | Install from URL | moderate | Post-install state not verified |
| skill-execution-flow.spec.ts | Ghost RPCs (RC-7) | skipped | **[RC-7 OPEN]** Runtime removed; spec calls non-existent RPC methods |
| skill-lifecycle.spec.ts | /skills page loads | shallow | No feature validation beyond page mount |
| skill-multi-round.spec.ts | /chat page loads | shallow | No multi-round skill behavior tested |
| skill-oauth.spec.ts | /skills page loads | shallow | No OAuth flow tested |
| skill-socket-reconnect.spec.ts | Home page loads | shallow | No socket reconnect behavior tested |

### Notifications (4 specs)

| Spec | Feature covered | Coverage depth | Known issues |
|------|----------------|----------------|--------------|
| notifications.spec.ts | Ingest + list + mark-read + UI | deep | |
| memory-roundtrip.spec.ts | Doc store + cross-namespace recall | moderate | Async indexing race on slow CI |
| cron-jobs-flow.spec.ts | Job creation UI | moderate | |
| autocomplete-flow.spec.ts | Chat autocomplete | shallow | |

### Webhooks & Tools (5 specs)

| Spec | Feature covered | Coverage depth | Known issues |
|------|----------------|----------------|--------------|
| webhooks-ingress-flow.spec.ts | RPC endpoints + debug panel | moderate | No actual payload delivery assertion |
| webhooks-tunnel-flow.spec.ts | Tunneling | moderate | |
| tool-browser-flow.spec.ts | Browser tool | moderate | |
| tool-filesystem-flow.spec.ts | Filesystem security | deep | |
| tool-shell-git-flow.spec.ts | Shell + git | moderate | |

### Providers (8 specs)

| Spec | Feature covered | Coverage depth | Known issues |
|------|----------------|----------------|--------------|
| telegram-flow.spec.ts | Telegram integration | skipped | **[SKIPPED OPEN]** describe.skip — no replacement spec |
| gmail-flow.spec.ts | Gmail OAuth | moderate | Token refresh path untested |
| accounts-provider-modal.spec.ts | Account connection modal | moderate | |
| slack-flow.spec.ts | Slack OAuth + Redux state | moderate | Multi-account scenario untested |
| whatsapp-flow.spec.ts | WhatsApp OAuth + state | moderate | Multi-account scenario untested |
| notion-flow.spec.ts | Notion OAuth | moderate | Scope upgrade path untested |
| conversations-web-channel-flow.spec.ts | Web channel messaging | moderate | Linux skip reason is stale |
| composio-triggers-flow.spec.ts | Trigger enable/disable + UI | moderate | No trigger event delivery tested |

### Payments (4 specs)

| Spec | Feature covered | Coverage depth | Known issues |
|------|----------------|----------------|--------------|
| card-payment-flow.spec.ts | Card payment + error handling | moderate | |
| crypto-payment-flow.spec.ts | Crypto payment | moderate | |
| rewards-unlock-flow.spec.ts | Rewards unlock | moderate | |
| rewards-progression-persistence.spec.ts | Rewards persistence | moderate | Hardcoded pauses; should use condition waits |

### Settings (7 specs)

| Spec | Feature covered | Coverage depth | Known issues |
|------|----------------|----------------|--------------|
| settings-channels-permissions.spec.ts | Channels + privacy settings | moderate | |
| settings-data-management.spec.ts | Data management | moderate | |
| settings-dev-options.spec.ts | Developer options | moderate | |
| settings-ai-skills.spec.ts | LLM config | shallow | OR-chain assertions (passes if any one LLM panel is present) |
| settings-account-preferences.spec.ts | Account preferences | moderate | |
| settings-advanced-config.spec.ts | Advanced config | moderate | |
| settings-feature-preferences.spec.ts | Feature toggles | moderate | |

### System (6 specs + 1 Linux-only)

| Spec | Feature covered | Coverage depth | Known issues |
|------|----------------|----------------|--------------|
| local-model-runtime.spec.ts | Ollama integration | skipped | **[SKIPPED OPEN]** describe.skip |
| voice-mode.spec.ts | Voice I/O | shallow | Hardcoded pauses |
| screen-intelligence.spec.ts | Screen awareness | shallow | |
| audio-toolkit-flow.spec.ts | Audio toolkit | shallow | |
| tauri-commands.spec.ts | Tauri IPC surface | moderate | |
| service-connectivity-flow.spec.ts | Service discovery | moderate | Requires OPENHUMAN_SERVICE_MOCK=1 |
| linux-cef-deb-runtime.spec.ts | Linux /usr/bin path | moderate | Linux only |

### User Journeys (3 specs)

| Spec | Feature covered | Coverage depth | Known issues |
|------|----------------|----------------|--------------|
| user-journey-full-task.spec.ts | Task completion end-to-end | moderate | |
| user-journey-settings-round-trip.spec.ts | Settings persistence round-trip | moderate | |
| chat-conversation-history.spec.ts | Conversation history | moderate | |

---

## Known Issues

| ID | Spec | Severity | Status | Description |
|----|------|----------|--------|-------------|
| RC-7 | skill-execution-flow.spec.ts | HIGH | **[RC-7 OPEN]** | Calls RPC methods that were removed when the QuickJS runtime was stripped. Spec will ghost-fail silently until updated or deleted. |
| SKIP-1 | telegram-flow.spec.ts | MEDIUM | **[SKIPPED OPEN]** | Entire suite is `describe.skip`. No replacement coverage. |
| SKIP-2 | local-model-runtime.spec.ts | LOW | **[SKIPPED OPEN]** | Entire suite is `describe.skip`. Ollama is optional — acceptable. |
| RACE-1 | memory-roundtrip.spec.ts | LOW | **[RACE]** | Async indexing race on slow CI machines. Intermittent. |
| RACE-2 | onboarding-modes.spec.ts | LOW | **[RACE]** | config.toml write race during core restart. Intermittent. |
| SHALLOW-1 | skill-lifecycle.spec.ts | MEDIUM | **[SHALLOW]** | Only asserts page mount, not any skill lifecycle behavior. |
| SHALLOW-2 | skill-multi-round.spec.ts | MEDIUM | **[SHALLOW]** | Only asserts /chat page loads. |
| SHALLOW-3 | skill-oauth.spec.ts | MEDIUM | **[SHALLOW]** | Only asserts /skills page loads. No OAuth. |
| SHALLOW-4 | skill-socket-reconnect.spec.ts | MEDIUM | **[SHALLOW]** | Only asserts home page loads. No socket reconnect. |
| PAUSE-1 | chat-harness-wallet-flow.spec.ts | LOW | **[PAUSE]** | Six sequential `browser.pause(30_000)` calls. Should be replaced with condition waits. |
| PAUSE-2 | rewards-progression-persistence.spec.ts | LOW | **[PAUSE]** | Hardcoded pauses. Should be replaced with condition waits. |
| PAUSE-3 | voice-mode.spec.ts | LOW | **[PAUSE]** | Hardcoded pauses in voice I/O flow. |
| STALE-1 | conversations-web-channel-flow.spec.ts | LOW | **[STALE]** | Linux skip condition uses a reason that no longer applies. |
| ASSERT-1 | settings-ai-skills.spec.ts | LOW | **[SHALLOW]** | OR-chain assertions: passes if any one LLM provider panel is present. |

---

## Mock API behavior flags

These flags are set via `setMockBehavior(key, value)` from `mock-server.ts` and
control the shared mock backend at `http://127.0.0.1:18473`.

| Flag | Type | Description |
|------|------|-------------|
| `seed` | string | Fuzzy randomization seed for mock data generation |
| `forceError503` | `'true'` / `'false'` | Force HTTP 503 on all non-admin endpoints |
| `llmStreamScript` | JSON string | Custom LLM response delta sequence. Array of `{delta: string}` objects |
| `composioConnections` | JSON string | Override Composio connections list (e.g. `'[]'` for empty) |
| `composioAvailableTriggers` | JSON string | Override available triggers returned by the API |
| `composioActiveTriggers` | JSON string | Override active triggers state |
| `purchaseError` | string | Trigger payment failure (value becomes the error message) |
| `plan` | `'FREE'` / `'BASIC'` / `'PRO'` | Override the billing plan returned by `/settings` |
| `planActive` | `'true'` / `'false'` | Override whether the plan is active |
| `planExpiry` | ISO date string | Override the plan expiry date |
| `session` | `'revoked'` / `'active'` | Force 401 on auth endpoints when set to `'revoked'` |

Reset all flags to defaults: `resetMockBehavior()`.

---

## How to run

```bash
# Full suite (all 66 specs)
bash app/scripts/e2e-run-all-flows.sh

# Single suite category
bash app/scripts/e2e-run-all-flows.sh --suite chat

# Stop after first failure
bash app/scripts/e2e-run-all-flows.sh --bail

# Single spec (fastest iteration)
bash app/scripts/e2e-run-session.sh test/e2e/specs/smoke.spec.ts smoke

# Pre-flight check only
bash app/scripts/e2e-preflight.sh

# With Appium/WDIO debug output
WDIO_LOG_LEVEL=debug bash app/scripts/e2e-run-all-flows.sh --suite auth

# Skip preflight (e.g. in CI where it ran as a separate step)
bash app/scripts/e2e-run-all-flows.sh --skip-preflight

# Use the debug runner (summary output + log tee)
pnpm debug e2e test/e2e/specs/smoke.spec.ts
pnpm debug e2e test/e2e/specs/notifications.spec.ts notifications --verbose
```

---

## How to add a new spec

1. **Create the spec file** in `app/test/e2e/specs/YOUR-SPEC.spec.ts`.

2. **Scaffold the harness:**
   ```typescript
   import { resetApp } from '../helpers/reset-app';
   import { startMockServer, stopMockServer } from '../mock-server';

   describe('Your feature', () => {
     before(async () => {
       await startMockServer();
       await resetApp('e2e-your-spec');
     });
     after(async () => {
       await stopMockServer();
     });

     it('does the thing', async () => { /* ... */ });
   });
   ```

3. **Register in the orchestrator** — add a `run(...)` call in the correct
   suite section of `app/scripts/e2e-run-all-flows.sh`.

4. **Add to this tracking doc** — add a row to the coverage matrix table
   for the appropriate category with an honest coverage depth.

5. **Add any new RPC methods** to `REQUIRED_RPC_METHODS` in
   `app/test/e2e/helpers/rpc-preflight.ts` if the spec calls RPC methods
   not already listed there.

6. **Run pre-flight** before executing: `bash app/scripts/e2e-preflight.sh`.
