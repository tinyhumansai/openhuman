# Release Manual Smoke Checklist

Run this checklist on every release-cut. Sign-off lives as a GitHub commit comment on the `v<version>-staging` tagged commit that QA validated (paste the checklist with checked items + the sign-off block at the bottom). Before approving the production run, the `Release-Approval` reviewer checks the sign-off exists **and** that the run targets that validated commit (staging SHA passed as `commit_sha`, or separated from the target only by `[skip ci]` version-bump commits). Owns OS-level surfaces that drivers cannot assert — everything else is automated under WDIO, Vitest, or Rust integration tests (see [Testing Strategy](../gitbooks/developing/testing-strategy.md)).

This is the **only** acceptable substitute for a `🚫` row in [`TEST-COVERAGE-MATRIX.md`](./TEST-COVERAGE-MATRIX.md). If a feature has neither automated coverage nor an entry on this checklist, treat it as untested and open a coverage gap.

---

## How to use

1. Build the release artifact for each platform you ship.
2. On a clean machine (or fresh user account), walk through `## Per-release smoke` then the section for the active release line.
3. Tick each box only after you have verified the expected outcome with your own eyes.
4. Paste the completed checklist + sign-off block as a commit comment on the `v<version>-staging` tagged commit.
5. Any item that is genuinely not applicable for this release: mark `N/A` with a one-line reason; do not silently skip.

---

## Per-release smoke

Applies to every release, all platforms.

### Conversation resume

- [ ] **Default agent traces reach Langfuse** — With a signed-in staging test account, send a synthetic chat turn that spawns a subagent. Expected: the parent and child traces share one conversation session, include the intended input/output and model usage, and carry the authenticated user. Confirm `share_usage_data = false` stops export.
- [ ] **An existing chat accepts another turn after restart** — Send a message and wait for its reply, fully quit OpenHuman, then reopen that conversation and send a second message. Expected: the second reply streams normally, retains the earlier context, and does not show a generic error. Repeat after the conversation has compacted if a long-running test profile is available (#6608).
- [ ] **A failed chat turn does not offer an invalid regenerate action** — Trigger a provider failure in a test profile and inspect its error card. Expected: the diagnostic text remains visible, with no Retry or Refresh button on that failed message. A completed assistant reply still offers Refresh (#6613).
- [ ] **A failed turn leaves its thread usable** — In a test profile, get one successful reply, trigger a streamed provider failure on the next turn, then send another message in the same thread. Expected: the error card appears, the composer re-enables, the next reply streams normally, and the agent still has the first turn's context. If a queued follow-up starts as the failed turn ends, its stream and composer state stay active.

### Native desktop control

- [ ] **Connections enables the published desktop module on an unlocked macOS or Windows session** — Open Connections → Integrations → Desktop Control, verify the Early Alpha notice, enable it, grant Accessibility if prompted, and run the read-only test. Expected: the module loads from the pinned release, the panel reports the actual permission state, and the test sees an accessibility snapshot. Screen Recording is required only when testing capture. A locked macOS screen must not be reported as a successful probe.
- [ ] **An agent completes a disposable desktop task without an approval wait by default** — In a fresh thread, ask the orchestrator to use a harmless app such as TextEdit or Calculator, make one visible change, and verify it through a new accessibility snapshot. Expected: `tool_search` discovers the deferred desktop tool, the released module performs the action, no approval card parks the turn, and the observed app state matches the request. Disable Desktop afterward and verify a subsequent tool call is refused.
- [ ] **A scoped message goal sends exact text once** — With a consenting test contact and an unlocked desktop session, ask the agent to send one unique message through the native chat app. Expected: it confirms the active conversation header, preserves the requested text exactly, and verifies the outgoing message inside that conversation. A chat-list row alone must not count as success; an unverified delivered send must stop as uncertain without a second click. Inspect the chat independently for duplicates.

### Browser module

- [ ] **Browser readiness and setup** — Open Connections → Integrations → Browser Control on each desktop platform and verify the Early Alpha notice. Expected: the checksum-pinned TinyBrowser module loads from the installer on Windows or the release cache on other platforms and passes TinyBus admission, module and Chrome readiness are reported separately, Test works, and saved viewport, profile, download folder, task limits, and allowed websites survive relaunch.
- [ ] **Browser task and policy** — With an allowed Selenium test site, use a conversation to submit its web form and download File 1. Expected: `tool_search` discovers `browser`, consequential actions wait for the exact host approval, the submitted page shows “Received!”, and a completed download is verified on disk. Then restrict allowed websites and confirm a disallowed navigation is blocked.

### Wallet balances

- [ ] **Connections wallet layout and setup** — Open Connections → Integrations → Wallet Balances at the default window size and a narrower size. Expected: the heading, Early Alpha notice, toolbar, and cards share one horizontal gutter; an unconfigured wallet shows a setup notice and supported network cards without a balance table. On a configured test wallet, Refresh, network filtering, address copy, Send, and Receive remain available.

### Checkbox appearance

- [ ] **Skill source filters show their selection** — In Connections → Skills, open the catalog source filter and toggle a source off and on. Verify that the rows filter correctly, the menu stays open, and the selected source has a visible checkmark. Repeat in light and dark themes, including Matrix, Ocean and Sepia dark; selected and indeterminate shared checkboxes must show a contrasting mark, while unchecked boxes remain empty.

### Public installer script

- [ ] **`scripts/install.sh` downloads the latest asset on a proxy/VPN network** — From a clean checkout, run `bash scripts/install.sh --dry-run --verbose`, then run the public `curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash` flow on one macOS or Linux host. Expected: release metadata resolves, the asset downloads successfully, and transient GitHub/CDN HTTP/2 failures retry over HTTP/1.1 instead of surfacing `curl: (16) Error in the HTTP2 framing layer`.

### Terminal agent cockpit

- [ ] **`openhuman-tui --last` completes an interactive agent turn and restores the terminal** — In a real PTY, first run `openhuman-tui --new`, send `Remember marker TUI-SMOKE-42`, wait for completion, and exit with Ctrl+C. Run `openhuman-tui --last`, verify that marker and its answer are restored, send a multiline prompt, steer the active turn with Enter, then type a follow-up and press Tab while streaming; verify the queued follow-up executes after the active turn. Open `/help` and `/status`, exit with Ctrl+C, and confirm shell echo/line editing still work. Repeat the resume, one turn, and exit flow with `openhuman-tui --last --no-alt-screen`; confirm history renders in the current buffer and the shell is usable afterward.

### macOS

- [ ] **Screenshot paste in normal chat (4.2.8)** — Copy a screenshot to the clipboard, focus the chat message input, and press Cmd+V. Expected: one image preview appears, the draft text is preserved, and no message is sent. Repeat via Edit → Paste, then paste plain text and confirm normal insertion.

- [ ] **Gatekeeper accepts the signed `.app` on first launch** — Double-click the `.app` from a fresh download (Quarantine attribute set). Expected: app opens without `"OpenHuman" cannot be opened because the developer cannot be verified` dialog. If it appears, the build is unsigned or the notarization stapler is missing.
- [ ] **`codesign --verify --deep --strict <path-to-OpenHuman.app>` exits 0** — Run from terminal. Expected: no output, exit 0. Any `code object is not signed at all` or `invalid signature` output blocks the release.
- [ ] **DMG drag-to-Applications flow works** — Mount the `.dmg`, drag `OpenHuman.app` to the `Applications` alias. Expected: copy completes; eject succeeds; first launch from `/Applications` does not re-prompt Gatekeeper.
- [ ] **Accessibility permission prompt fires on first agent run** — Trigger an agent action that uses Accessibility (e.g. window-control skill). Expected: macOS prompts `OpenHuman would like to control this computer using accessibility features`. Granting it allows the action; denying it surfaces a clear in-app fallback.
- [ ] **Input Monitoring prompt fires on first hotkey use** — Press the registered global hotkey for the first time. Expected: `Input Monitoring` prompt; granting it makes the hotkey trigger; denying it does not crash the app.
- [ ] **Microphone prompt fires on first voice capture** — Start a voice session. Expected: standard mic prompt; granted → capture begins; denied → fallback message, no panic.
- [ ] **File picker does not crash on Documents/Downloads/Desktop selections** — From an embedded app (Slack, Discord, Telegram), trigger a file upload and pick a file from `Documents`, `Downloads`, and `Desktop` in turn. Expected: macOS prompts `OpenHuman would like to access files in your <Folder> folder` the first time per folder; deny + retry must not crash.

### Windows

- [ ] **Bundled native modules work offline** — Install from the signed MSI or NSIS package on a fresh Windows account, disconnect the network, then open Desktop Control and Browser Control and load their modules. Expected: both modules reach Ready without a GitHub request, and the log records `[modules] loaded '<id>' from the installer bundle`. Reconnect before testing hosted features. Check both installer formats when both are shipped.
- [ ] **Custom window frame and exit** — Launch with no saved window geometry. Expected: the window opens near 800 × 720 with compact rounded corners; the top-right controls minimize, maximize/restore, and close. Drag the window from the top strip on both the loading screen and main app. Closing exits the host and its embedded core (no lingering `OpenHuman.exe` or core listener).
- [ ] **SmartScreen does not block install** — Run the installer from a fresh download. Expected: SmartScreen passes (signed binary). If `Windows protected your PC` appears, the EV signature is missing or the reputation has not built up — escalate before shipping.
- [ ] **Installer creates Start Menu + Desktop shortcuts** — Defaults preserved. Expected: both shortcuts launch the app.
- [ ] **Chat links preserve the desktop UI** — Click an HTTPS PR link in an assistant reply. Expected: the default browser opens the PR and OpenHuman stays on the same conversation. If the OS opener fails, the app must remain visible instead of navigating to the remote page. Check internal chat/settings navigation still works.
- [ ] **Logout retires external channel listeners** — With a channel configured for a local or Claude CLI model, log out while leaving the core process running, then send a channel message. Expected: the old listener no longer processes the message or returns prior-account memory. Start a fresh local workspace/runtime separately and verify local model use still works without backend login.
- [ ] **App registers `openhuman://` URL scheme** — From a browser, click an `openhuman://oauth/success?...` link. Expected: OS prompts to open in OpenHuman; clicking through delivers the deep link.

### Linux

- [ ] **Public `install.sh` prefers the `.deb` path on clean Ubuntu 24.04** — Run `curl -fsSL https://raw.githubusercontent.com/tinyhumansai/openhuman/main/scripts/install.sh | bash` on a host with `apt-get` and `dpkg`. Expected: the script resolves `OpenHuman_*_amd64.deb` or `OpenHuman_*_arm64.deb`, installs it with `apt-get`, and launch does not fail on missing CEF runtime libraries such as `libgbm.so.1`.
- [ ] **`.deb` and/or `.AppImage` install on a clean Ubuntu 22.04** — `sudo apt-get install -y --no-install-recommends ./OpenHuman_*.deb` or `chmod +x OpenHuman_*.AppImage && ./OpenHuman_*.AppImage`. Expected: no missing-dependency errors; app launches.
- [ ] **`.AppImage` launches on a clean Ubuntu 24.04 host without a sibling extracted tree** — Run the downloaded AppImage directly from an empty directory. Expected: no `Interpreter not found!` error; `sharun` finds its bundled dynamic linker and the app reaches the first window.
- [ ] **OS-native notification toasts fire** — Trigger a notification from inside the app (e.g. memory captured, agent finished). Expected: a libnotify-style toast appears outside the app window. (CI Linux sees only Xvfb; this surface verifies on a real desktop.)
- [ ] **Headless supervisor update stages without self-exit** — On a Linux service deployment with `[update] restart_strategy = "supervisor"` and `rpc_mutations_enabled = false`, stage a new core binary through the documented operator flow. Expected: the running process stays up until the supervisor restart, the staged binary is present on disk, and `systemctl restart openhuman` (or equivalent) picks up the new version.

### Cross-platform

- [ ] **Caller-owned inference works without an OpenHuman session** — In a local workspace without an OpenHuman login, configure Ollama/LM Studio/MLX/oMLX/local-openai or an independently authenticated Claude Code/Agent SDK provider. Run chat and an agent flow routed entirely to that provider. For a named harness agent, also configure the summarization route to managed inference and verify that the agent still uses its local route; reversing those routes must retain the managed agent's session requirement. Expected: no OpenHuman session requirement. Select managed inference instead: it must still require a backend session. With LocalOnly privacy enabled, local runtimes remain allowed and Claude subprocesses remain blocked as external inference.

- [ ] **Chat links open in the default browser and the app stays on the chat** — Ask the agent for a GitHub URL and click the link in its reply. Expected: the default browser opens the page and OpenHuman stays on the same conversation (no remote page inside the app window, no stranded screen). Then click a link in Settings > About. Expected: same result, and in-app navigation (Chat, Settings) still works.
- [ ] **ChatGPT sign-in works after onboarding** — In desktop Settings > AI > Providers, add OpenAI and complete ChatGPT sign-in from its provider dialog. Expected: OpenAI is registered without an API key and existing workload routes are preserved. Reopen the provider dialog and disconnect. Expected: the connected badge clears, OpenAI is removed, and workloads no longer reference it. A failed callback shows a localized error without logging the redirect URL.
- [ ] **OpenRouter sign-in can be cancelled, denied, and retried** — In desktop Settings > Connections > LLM, add OpenRouter and click **Sign in with OpenRouter**. While the dialog reads "Connecting…", press Cancel. Expected: the dialog closes. Reopen it, click Sign in again, and press Esc. Expected: the dialog closes, and a third Sign in opens a fresh OpenRouter page. On that page click **Deny**. Expected: the browser tab reads "Sign-in was not completed." (never "You're signed in."), the dialog shows an error with Sign in enabled, and no OpenRouter provider is added.
- [ ] **First launch flow completes for a brand-new user** — Fresh OS user account, no `~/.openhuman` directory. Walk through onboarding to first agent reply. Expected: no crashes, no permission deadlocks, no stale-config errors.
- [ ] **A background sub-agent result lands in Chat exactly once** — Ask for something that delegates to a background sub-agent (e.g. "how's my day looking?" with a calendar connected, or any `delegate_*` archetype), then wait for it to finish. Expected: the delivered reply appears once, as a normal agent message; no user-side (right-aligned) bubble showing raw `**markdown**`, and still one copy after switching to another thread and back (#5933).
- [ ] **Auto-update download + relaunch succeeds** — Install the previous release, point the updater feed at this release, trigger an update check. Expected: download completes, relaunch installs the new binary, version string in `Settings > About` matches the release tag.
- [ ] **GitHub Release notes are AI-generated from the previous release tag** — Before publishing the draft production release, inspect the GitHub Release body. Expected: notes start with a thematic H1 title, include high-level highlight sections with PR links and contributor thanks, list only PRs merged on the mainline (no branch or direct commits), omit a separate pull-request dump, include new-contributor thanks only when applicable, and the full compare URL is previous release tag → current release tag.
- [ ] **Logging out + logging back in preserves nothing private** — Sign out, sign in as a different user. Expected: no leaked memory, threads, or skill state from the previous session (regression watch — see #900).
- [ ] **A Docker core gateway connects, serves the app, and is torn down on switch-back** — Build `openhuman-core:local` (`docker compose build openhuman-core`). In `Settings > Core connection > Run the core somewhere else`, add a location running in a container and choose it. Expected: the status advances through its named steps and settles on Connected; chat, memory, and an agent run work against it; `docker ps` shows one `tinybox-*` container. Switch back to This computer. Expected: the local core answers again and the container is gone. Then relaunch the app while the gateway is selected. Expected: it re-provisions and reconnects rather than silently falling back to the local core — that fallback is the failure this row exists to catch, because everything keeps working against the wrong core.
- [ ] **`memory_tree` migrates WAL→TRUNCATE on upgrade with memory intact** — Install a previous (WAL-era) build, use it enough to populate memory so a `chunks.db-wal`/`-shm` pair exists under `~/.openhuman/.../workspace/memory_tree/`, then upgrade to this build. Expected on first launch: `PRAGMA journal_mode` on `chunks.db` reports `truncate`, the `-wal`/`-shm` side-files are gone, previously-captured memories still surface in recall, and no `Failed to initialize memory_tree schema` errors appear.
- [ ] **Home connectivity chip reflects the core's hosted link** — Sign in on a desktop build and wait for the chip on Home to read green "Connected". Block the core's traffic to the hosted backend in a way that also kills the *established* socket, not only new connections: a firewall DROP rule for the backend host (e.g. Linux `sudo iptables -I OUTPUT -d <backend-ip> -j DROP`, macOS a pf or Little Snitch/LuLu deny rule for `api.tinyhumans.ai`), or a VPN route change that blackholes the active flow and the handshakes that follow. A hosts-file entry does not touch an established socket; if that is all you have, add it first and then force a reconnect (toggle Wi-Fi off and on). Use DROP rather than REJECT so the reconnect attempts hang the way the field failure did. Expected, in `~/.openhuman/logs/openhuman.<date>.log`: within ~50 s of the block, `[socket] No server ping received … connection age Ns …` and `Connection lost: Ping timeout`; then `Connection failed (attempt N/5): WebSocket connect: IO error: operation timed out after 10s …` lines never more than ~10 s plus the backoff apart. The chip turns amber "Disconnected" within ~30 s of the loss while chat keeps working. Remove the rule: the chip returns to green within ~5 s of the next handshake and the log shows `[socket] Reconnected after …` (#6256).

---

## Active release line

> If multiple stable release lines are in flight (security backports, LTS), add a sub-section per line and check the same boxes for each. As of writing, `0.52.x` is the only active line — older minor versions are end-of-life. Fold this section to suit when more release lines exist.

### 0.52.x — current

- [ ] **OAuth gate respects `VITE_MINIMUM_SUPPORTED_APP_VERSION`** (per [Release Policy](../gitbooks/developing/release-policy.md)) — Set the variable to a value above this build's version, build, attempt OAuth from the older binary. Expected: gate blocks the deep link; opens `VITE_LATEST_APP_DOWNLOAD_URL`.
- [ ] **Gmail connect succeeds on a fresh install from `releases/latest`** — Per release-policy step 4. Expected: token exchange completes, inbox lists in-app.

---

## Sign-off

```text
Release: vX.Y.Z
Tester: @<github-handle>
Date: YYYY-MM-DD
Platforms tested: [macOS arm64] [macOS x64] [Windows] [Linux .deb] [Linux .AppImage]
Notes:
```

Paste the filled block as a commit comment on the `v<version>-staging` tagged commit before promoting to production.
