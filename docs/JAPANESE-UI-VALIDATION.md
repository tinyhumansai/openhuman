# Japanese UI validation

Validation performed on 2026-10-04, then repeated after merging upstream
`9c475c174ca5506f67ed471ec34ef14617316122` and refreshing the Japanese memory
lifecycle strings at `c261b29e6ac5eabb00c84fb7d87554632d0982d7`.
The final locale has 4,541 keys, with no missing or extra keys and no English
leak findings. The repeat native build and standalone remote Core use that
application source. Later CI repairs also change Core behavior as described
below; the native observations above belong to the recorded application revision.
The remote Core binary's SHA-256 matched the local build:
`fe82c895bb33d3e6b595345ee6c7284c0217a164c8b4ff48b4442c5d0466b6fb`.

## Automated browser E2E

`app/test/playwright/specs/japanese-locale.spec.ts` imports the shared suite in
`app/test/e2e/specs/japanese-locale.browser.ts`. The existing Playwright web lane
discovers the wrapper. Run the focused suite with the repository's mock backend
and standalone Core:

```sh
pnpm --filter openhuman-app test:e2e:web:build
bash app/scripts/e2e-web-session.sh test/playwright/specs/japanese-locale.spec.ts
```

All three tests passed again on the refreshed source (4.4 seconds):

- Select 日本語 through Settings, verify Japanese Chat labels and `html[lang=ja]`,
  reload and verify persistence, then select English and reload again.
- Start a fresh `ja-JP` browser context, verify automatic Japanese selection,
  and verify that a saved English choice wins after reload.
- At 1280×720, navigate to Memory v2 Explorer and Token Usage. Verify `7件`,
  `Qwen3.8-Flash-Next`, and `3 回の圧縮で` replace the interpolation tokens and
  remain visible. Only data-dependent read RPC results are fixtures; locale
  changes, storage, navigation and authentication use the shared Core harness.

E2E TypeScript, ESLint, Prettier and the coverage-matrix guard passed. Using Node
24, the focused LanguageSelect, localeSlice, i18n and attachment suites passed
150 tests. A prior run with mixed Node versions failed one gzip attachment test;
that failure disappeared when the runtime matched CI.

## Native macOS smoke

Built the production frontend and unsigned debug `.app` with the PR's pinned
Rust toolchain and recursive submodules. Used a separate application identifier
and disposable workspace, leaving the installed app and production memory
intact. No WebDriver or mocked model was used for these native checks.

Observed:

- Fresh first-run UI displayed 日本語 in the language/runtime picker.
- Settings, Chat, Connections, Memory v2 and Gateway labels rendered in Japanese.
- After the upstream refresh, the new Brain, Background and Memory Settings tabs
  rendered their Japanese headings, descriptions, controls and empty states.
- Settings switched Japanese → English → Japanese immediately.
- Actual app quit/relaunch checks preserved English and Japanese.
- The remote Core connection test succeeded over an SSH loopback tunnel.
- The Japanese routing dialog tested a custom OpenAI-compatible provider through
  the real model router and received `Hello! How can I help you today?` from
  Qwen3.8-Flash-Next. The provider and model name were visible in the response.
- After assigning that custom provider to Chat, a new native conversation sent
  a Japanese request for `17×19` and streamed the correct answer `323`.
  The UI → remote Core → actual router → model → UI path completed.
  After another quit/relaunch, both Japanese labels and that conversation
  remained visible. The upstream-refresh repeat used a newly created conversation
  and again returned `323`, then preserved that conversation after quit/relaunch.

## Integration limits

The existing production Core rejected the new UI's `reasoning_effort` parameter
on `channel.web_chat`. Therefore the native smoke used a separately built Core
at the same revision, running on the remote Mac Studio with isolated data.
Production Core and memory were not upgraded or migrated.

The legacy `local-openai` agent path reached the actual model but did not complete
a simple arithmetic request: its main wire request contained a synthetic
`Continue with the task described above.` user turn and the model attempted
unrelated tools. That legacy-path run was unsuccessful. The same real
endpoint configured through the current custom-provider UI did complete the
native chat E2E above; production routing was not changed. The isolated Memory
service was not configured, so Memory Explorer
showed its connection error; successful item/count rendering is covered by the
browser fixture test above. No iOS or Android native smoke was performed.

## Upstream integration checks

The upstream memory lifecycle update added 111 English keys and removed 30;
Japanese was refreshed to the same set rather than hiding coverage failures.
The same update reduced the Linux `flows` dependency graph to 332 packages,
311 names and 3 native dependencies. Both the kernel-floor ratchet and dep-sim
calibration were tightened to that measured graph and passed with
`CARGO_BUILD_TARGET=x86_64-unknown-linux-gnu`. The native app lockfile was aligned
to the pinned tinymemory 1.23.1 path crates. A fresh build cache avoided stale
Rust metadata from the earlier dependency graph; both the native bundle and
standalone Core built successfully. Production data remained untouched.

The enabled Rust lanes also exposed two stale upstream static baselines. The
agent-runtime boundary inventory now contains 204 exact entries (previously
205): seven obsolete fingerprints were removed and six were verified against
unchanged upstream Core source. They reflect moved integration/builder code,
a revised user-origin check, and two existing memory task-local accesses; no
Core implementation or checker rule was changed. The ignored-test caps were
tightened from 17 to 8 for Core and 15 to 14 for the top-level tests because
upstream removed their old ignored tests. Both exact-inventory guards passed.

## Repairs for the latest upstream CI failures

The hosted run on `8fae68aa6156457ec440ac5e373b3452566f194f` passed
frontend/static, Rust lint and Tauri, but failed seven Core coverage suites.
The following repairs preserve the original test assertions:

- Install the pinned pnpm workspace dependencies in the independent Rust
  coverage job. Its Memory integration fixture imports `ws`; without the
  install, the backend exits before its health endpoint starts.
- Update the Memory RPC catalogue and reachability tests to the current
  controller contract; explicitly assert that retired read methods are unknown.
- Give vision-delegation fixtures an actual inline PNG, as required by the
  production dispatcher. Request-count and child-response assertions remain.
- Point the Composio fixture at the resolved workspace directory and continue
  checking that its scope file exists.
- Align the test-only agent handler's stack with CI's 64 MiB test stack;
  its instrumented Discord full-pipeline test overflowed at both 8 and 16 MiB.
- Wire the existing iteration-cap resolver into the factory so explicit
  overrides are honored. Both cap-reporting/error integration tests pass.
- Preserve permanent attachments during tool-surface refresh and add their
  names to the session definition's extra tool scope. This fixes their omission
  from the native provider wire for named scopes. All three attached-tool
  integration tests pass, including clones and session resume.

These repairs were verified in an isolated x86_64 Ubuntu 22.04 container on
an Apple Silicon Mac Studio using the exact hosted CI image digest
`sha256:c3c20d625bcc9f75e50c3c2c2b75c88d3e60a3f4352b0f19f5327b490438722d`.
The VM has no host mounts and the container has no published ports or production
credentials. Node 24.21.0, pnpm 10.10.0 and Rust 1.96.1 were used. The complete
Node script suite passes (471 passed, one existing skip), as do the targeted
Memory, Composio, delegation, attached-tools and Discord coverage tests.
MacBook Core/CLI/Tauri Clippy passes with warnings denied. Full Rust coverage
and the new GitHub run are being checked separately; targeted passes do not
establish a green final CI gate.
