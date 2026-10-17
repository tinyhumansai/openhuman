---
description: >-
  The React and Vite frontend in app/src/: state, services, providers,
  routing, components and hooks.
icon: browsers
---

# Frontend

`app/src/` is the OpenHuman desktop UI: a Vite and React 19 tree in the pnpm workspace `openhuman-app`. It uses Redux Toolkit with persistence for session state, talks to the in-process Rust core over JSON-RPC (`coreRpcClient` over local HTTP, with the Tauri `relay_http_rpc` command as a fallback relay) and socket.io (`socketService`), and reaches the cloud backend via REST (`apiClient`). Heavy logic lives in the core, not here.

This page is a single reference. Use the table of contents to jump between sections.

The tree also carries a mobile shell: `AppRoutesIOS.tsx`, `pages/ios/`, the `services/transport/` connection profiles and the `app/src-tauri-mobile/` host. That client is experimental and not part of the shipped desktop host, which targets Windows, macOS and Linux only. It appears here because those files sit in the same tree and share the `App.tsx` provider chain. See [iOS companion](../../features/ios-companion.md) for what it is and what still has no desktop surface.

## Quick reference

| Section                                      | Covers                                                          |
| -------------------------------------------- | --------------------------------------------------------------- |
| [Architecture](#architecture-overview)       | Provider chain, build, layout, conventions                      |
| [State management](#state-management)        | Redux Toolkit slices, selectors, persistence                    |
| [Services layer](#services-layer)            | `apiClient`, `socketService`, `coreRpcClient`                   |
| [Providers](#providers)                      | `ThemeProvider`, `CoreState`, `Socket`, `ChatRuntime` providers |
| [Pages and routing](#pages-and-routing)      | `HashRouter`, route guards, main routes                         |
| [Components](#components)                    | UI / settings component patterns                                |
| [Hooks and utilities](#hooks-and-utilities)   | Shared hooks, helpers, config                                   |

## Scale

| Metric                                  | How to read it                                                 |
| --------------------------------------- | -------------------------------------------------------------- |
| TypeScript / TSX files under `app/src/` | `find app/src -name '*.ts' -o -name '*.tsx' \| wc -l`           |
| App-level hooks                         | `ls app/src/hooks/*.ts app/src/hooks/*.tsx \| wc -l`            |
| Test runner                             | Vitest (`app/test/vitest.config.ts`)                           |

## Directory layout

```text
app/src/
├── App.tsx                 # Provider chain + HashRouter shell (desktop + mobile shells)
├── AppRoutes.tsx           # Desktop route table (AppRoutesIOS.tsx for mobile)
├── main.tsx                # Entry (polyfills, Sentry, store, styles)
├── store/                  # Redux slices, selectors, userScopedStorage persistence
├── providers/              # ThemeProvider, CoreStateProvider, SocketProvider, ChatRuntimeProvider
├── services/               # apiClient, socketService, coreRpcClient, transport/, api/*
├── lib/                    # AI prompt loaders, i18n, MCP helpers, platform, tunnel crypto
├── pages/                  # Route-level screens (incl. onboarding/, ios/, dev/)
├── features/               # Feature verticals (human/, conversations/, voice/, wallet/, skills/)
├── components/             # Shared UI (incl. settings/, layout/shell/, accounts/)
├── hooks/                  # App hooks
├── utils/                  # Config, Tauri command wrappers, routing utilities
└── assets/                 # Icons and static assets
```

## Architecture overview

### System architecture

OpenHuman's desktop UI is a React 19 app (`app/src/`) that:

- Uses Redux Toolkit with persistence for session-related state
- Connects to the backend with REST (`apiClient`) and to the local core with Socket.io (`socketService` → core socket endpoint)
- Calls the Rust core (embedded in the Tauri host as a tokio task) over HTTP via `coreRpcClient` (JSON-RPC methods implemented in `crates/openhuman-core/src/`); non-loopback plain-http runtimes are relayed through the Tauri `relay_http_rpc` command
- Leaves AI prompts to the core: bundled `crates/openhuman-core/src/agent/prompts` ship as Tauri resources (see `crates/openhuman-app/tauri.conf.json` resources) and are read core-side, not by the frontend
- Uses a minimal MCP-style helper layer under `lib/mcp/` (transport, validation)

### Entry points

| File                    | Purpose                                                                          |
| ----------------------- | -------------------------------------------------------------------------------- |
| `app/src/main.tsx`      | React root, polyfills, Sentry boundary, store, global styles                     |
| `app/src/App.tsx`       | Provider chain (see below) + desktop/mobile shells, Settings       |
| `app/src/AppRoutes.tsx` | `HashRouter` routes, `ProtectedRoute` / `PublicRoute` / `DefaultRedirect` guards |

### Provider chain

<!-- BEGIN GENERATED: provider-chain — source app/src/App.tsx via scripts/generate-architecture-docs.mjs; do not edit between markers (run `pnpm docs:generate`) -->

_Generated from `app/src/App.tsx` by `scripts/generate-architecture-docs.mjs`. Do not edit by hand — run `pnpm docs:generate` to refresh._

| # | Component | Role |
| --- | --- | --- |
| 1 | `Sentry.ErrorBoundary` | Crash boundary; renders ErrorFallbackScreen |
| 2 | `Provider` | Redux store; enables useAppSelector / dispatch app-wide |
| 3 | `PersistGate` | Holds UI until persisted Redux slices rehydrate |
| 4 | `ThemeProvider` | Theme tokens and dark-mode handling |
| 5 | `I18nProvider` | Localization context consumed via useT |
| 6 | `BootCheckGate` | Blocks render until the core boot snapshot resolves |
| 7 | `CoreStateProvider` | Core app snapshot: auth, session, onboarding state |
| 8 | `SocketProvider` | Core socket.io events; desktop only (mobile uses the TunnelTransport relay) |
| 9 | `ChatRuntimeProvider` | Chat runtime events, tool timeline, and approvals |
| 10 | `Router` | HashRouter navigation for all routes |
| 11 | `CommandProvider` | Command palette context |
| 12 | `ServiceBlockingGate` | Blocks the shell until required services are configured |

<!-- END GENERATED: provider-chain -->

Why this order

1. Redux `Provider` is outermost so `useAppSelector` / dispatch work everywhere.
2. `PersistGate` rehydrates persisted slices before children assume stable auth/session.
3. `BootCheckGate` / `CoreStateProvider` resolve the core boot snapshot (auth, onboarding) before feature providers mount.
4. `SocketProvider` (desktop only) and `ChatRuntimeProvider` depend on that core state for realtime events and approvals.
5. `Router` supplies navigation to all routes.

### Module relationships (simplified)

```text
App.tsx
  ├─ Redux store + persistor
  ├─ ThemeProvider / I18nProvider - theme tokens, useT() localization
  ├─ BootCheckGate - waits for the core boot snapshot
  ├─ CoreStateProvider - auth/session/onboarding snapshot (fetchCoreAppSnapshot RPC)
  ├─ SocketProvider - socket.io connection to the local core (desktop only)
  ├─ ChatRuntimeProvider - chat streaming, tool timeline, approvals → Redux
  └─ AppShell (desktop or mobile)
       ├─ AppRoutes - PublicRoute / ProtectedRoute / DefaultRedirect
       └─ SettingsModal - overlay mounted when the URL is /settings/*
```

### Services layer (conceptual)

```text
services/
  ├─ apiClient        → REST to a URL resolved at runtime via `services/backendUrl#getBackendUrl`
  ├─ backendUrl       → Calls `openhuman.config_resolve_api_url`; falls back to VITE_BACKEND_URL only outside Tauri
  ├─ socketService    → Socket.io to the local core (base URL derived from the RPC URL); MCP-style envelopes
  ├─ coreRpcClient    → JSON-RPC over HTTP to the local openhuman core; `relay_http_rpc` fallback for non-loopback http
  └─ transport/       → ConnectionProfile transports for iOS/remote (LanHttp, Tunnel, CloudHttp)
```

#### Runtime config precedence

The desktop app does not bake the core RPC URL or the API host into the bundle as a hard requirement. At runtime the app resolves them in this order (highest first):

1. Welcome-screen RPC URL field, saved via `utils/configPersistence` and restored on next launch. End users configure a self-hosted core address here, not by hand-editing `config.toml` or `.env` files.
2. Tauri `core_rpc_url` command, the port the embedded core is listening on for this process.
3. `VITE_OPENHUMAN_CORE_RPC_URL`, build-time fallback for development.
4. The hardcoded `http://127.0.0.1:7788/rpc` default.

Once the RPC handshake succeeds, `services/backendUrl` calls `openhuman.config_resolve_api_url` to pull `api_url` (and other safe client fields) from the loaded core `Config`. `VITE_BACKEND_URL` is only used as a web fallback when the app runs outside Tauri.

Components that need the backend URL should call `useBackendUrl()` (or `getBackendUrl()` from non-React code), they must not import the static `BACKEND_URL` constant from `utils/config`, which represents the build-time value only.

### Related docs

- Rust architecture: [Architecture](../architecture.md)
- Tauri shell: [Tauri shell](tauri-shell.md)

## State management

The application uses Redux Toolkit with Redux-Persist. There is no single root persist config: each slice that persists wraps its own reducer with `persistReducer` in `store/index.ts`, whitelisting exactly the fields that should survive a restart.

### Storage backends

- `userScopedStorage` (`store/userScopedStorage.ts`) is the default storage for persisted slices. Blobs are keyed `${userId}:persist:<key>` so state never leaks across users on logout or login.
- Plain `localStorage` is used only for pre-login, device-wide slices (`coreMode`, `locale`, `theme`) that must survive user switches.

### Slices

Authoritative list = the `reducer` map in `store/index.ts`. One-line purposes:

| Slice                 | Purpose                                                                          | Persisted?                                                      |
| --------------------- | --------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| `accounts`            | Connected web-app accounts + rail ordering                                        | `accounts`, `order`, `lastActiveAccountId` (not the active id)  |
| `announcement`        | Harness-init announcement banner, seen ids                                        | `shownIds`                                                      |
| `channelConnections`  | Messaging channel connections (WhatsApp, Slack, …)                                | connections + migration/default-channel fields                  |
| `chatRuntime`         | Streaming buffers, tool timelines, inference status, artifacts                    | only `artifactsByThread` (ready snapshots)                       |
| `connectivity`        | navigator.onLine, core health, renderer to core socket, core to hosted link       | no                                                               |
| `coreMode`            | Pre-login core mode selection (embedded / self-hosted / cloud)                    | `mode` (plain localStorage)                                      |
| `followupSuggestions` | Follow-up chips for each thread's latest settled turn                             | no (in-memory only)                                              |
| `githubStar`          | Whether the user dismissed the in-app "Star us on GitHub" CTA                     | `dismissed`                                                      |
| `layout`              | Two-pane layout geometry (sidebar visibility, dragged widths)                     | `panels`                                                         |
| `locale`              | UI language                                                                        | `current` (plain localStorage)                                  |
| `mascot`              | Mascot appearance / voice selection                                               | `color`, `voiceId`, `customMascotGifUrl`, `selectedMascotId`    |
| `notifications`       | Notification items + preferences                                                  | `items`, `preferences`                                          |
| `persona`             | Cosmetic persona display name + description (SOUL.md lives in the core)           | `displayName`, `description`                                    |
| `ptt`                 | Push-to-talk hotkey + session prefs (`isHeld` deliberately excluded)              | `shortcut`, `speakReplies`, `showOverlay`                        |
| `queue`               | The core's per-thread run queue plus the composer's pending follow-up messages    | no (in-memory only)                                              |
| `runMode`             | Per-thread plan/build run mode                                                    | no (in-memory only)                                              |
| `socket`              | Per-user socket connection status / socket ids                                    | no (reconnects on boot)                                          |
| `theme`               | Theme mode, font size, message view mode, custom themes                           | plain localStorage                                               |
| `thread`              | Chat thread list + per-thread message caches                                      | only `selectedThreadId`                                          |
| `threadGoal`          | Durable per-thread goal state                                                     | no (in-memory only)                                              |
| `threadTodos`         | Live per-thread todo list                                                         | no (in-memory only)                                              |
| `userErrors`          | User-actionable runtime errors                                            | no (in-memory only)                                              |
| `walletPreferences`   | Hidden-token preferences for the wallet view                                      | `hiddenTokenKeys`                                                |

Ephemeral chat state (streaming buffers, tool timelines) must not survive a restart: the UI would try to resume a turn whose live driver is gone. The one exception, agent-generated artifacts, goes through the `artifactsReadyOnlyTransform` in `store/index.ts` (pure logic in `store/artifactsPersistFilter.ts`).

### Typed hooks

File: `store/hooks.ts`

```typescript
// Use these instead of plain useDispatch/useSelector
export const useAppDispatch: () => AppDispatch = useDispatch;
export const useAppSelector: TypedUseSelectorHook<RootState> = useSelector;
```

### Best practices

1. Always use the typed hooks, `useAppDispatch` and `useAppSelector`.
2. Use selectors for derived state: see `store/socketSelectors.ts`, `store/connectivitySelectors.ts`, `store/userErrorsSelectors.ts`.
3. Whitelist persistence per slice. Never persist transient or loading state; add a per-slice `persistReducer` in `store/index.ts`.
4. Prefer Redux over ad hoc `localStorage`. Plain localStorage is reserved for the pre-login slices noted above.
5. In dev and E2E builds the store is exposed as `window.__OPENHUMAN_STORE__` so WDIO specs can assert backing state; production bundles do not expose it.

---

## Services layer

The application uses singleton services for external communication. This prevents connection leaks and provides consistent API access.

### Service architecture

```text
app/src/services/
  ├─ apiClient (HTTP REST)
  │   └─ backend URL resolved at runtime (services/backendUrl)
  ├─ socketService (Socket.io)
  │   └─ connects to the local core's socket endpoint (base derived from the RPC URL)
  ├─ coreRpcClient.ts
  │   ├─ direct webview fetch → local openhuman core (JSON-RPC over HTTP)
  │   └─ invoke('relay_http_rpc', …) fallback for non-loopback plain-http runtimes
  ├─ coreCommandClient.ts - typed wrappers over core RPC methods
  ├─ transport/ - ConnectionProfile transports (LanHttp, Tunnel, CloudHttp) for iOS/remote
  └─ services/api/* - domain API modules (~50 files, see below)
```

### API client (`services/apiClient.ts`)

Fetch-based HTTP REST client for backend communication with typed request/response handling and error handling. The backend URL is resolved at runtime (`services/backendUrl`), not baked in.

```typescript
import apiClient from "../services/apiClient";

const user = await apiClient.get<User>("/users/me");
const result = await apiClient.post<LoginResponse>("/auth/login", {
  email,
  password,
});
```

### Domain API modules (`services/api/`)

\~55 domain-scoped modules, one per feature surface, each wrapping either backend REST endpoints or core RPC methods. Representative examples:

- `authApi` / `userApi`: auth + user profile
- `threadApi`, `threadUsageApi`: chat threads
- `agentTeamApi`, `agentWorkApi`, `subagentApi`: agents
- `skillsApi`, `skillRegistryApi`, `flowsApi`, `workflowRunsApi`: skills & automation
- `channelConnectionsApi`, `mcpClientsApi`, `mcpSetupApi`, `tunnelsApi`: connections
- `memoryApi`: Memory v2 RPC wrappers (`openhuman.memory_*`)
- `billingApi`, `creditsApi`, `referralApi`, `inviteApi`: commerce
- `voiceSettingsApi`, `aiSettingsApi`, `modelCouncilApi`: AI/voice config

For the full list, `ls app/src/services/api/`. New feature surfaces get their own module here rather than growing `apiClient`.

### Socket service (`services/socketService.ts`)

Socket.io client singleton connected to the local core's socket endpoint (base URL derived from the resolved RPC URL via `coreSocket.ts`; authenticated with the core RPC token). It ingests realtime core events (connection status, channel updates) and dispatches them into Redux (`socketSlice`, `connectivitySlice`, `channelConnectionsSlice`). It also hosts the MCP-style transport (`SocketIOMCPTransportImpl` from `lib/mcp`).

Keep `socketService` and the core socket behavior aligned (the "dual socket sync" rule in AGENTS.md). Connection lifecycle is owned by `providers/SocketProvider.tsx`; on mobile the provider is not mounted at all: events arrive through the `TunnelTransport` relay instead.

### Core RPC (`services/coreRpcClient.ts`)

The Rust core runs in-process inside the Tauri host (no sidecar). The UI calls JSON-RPC methods on it over local HTTP:

```typescript
import { callCoreRpc } from "../services/coreRpcClient";

const result = await callCoreRpc<MyType>({
  method: "openhuman.some_method",
  params: {
    /* … */
  },
  timeoutMs: 60_000, // optional per-call override (default 30s)
  suppressAuthExpiredEvent: false, // narrow reads can opt out of global sign-out on 401
});
```

How a call flows:

1. URL + token resolution: the RPC URL follows the precedence in [Runtime config precedence](#runtime-config-precedence); the per-launch bearer token comes from the Tauri `core_rpc_token` command (or the stored token for self-hosted cores).
2. Direct fetch: the webview `fetch()`es the JSON-RPC envelope straight to the core (loopback http or any https URL).
3. Shell relay fallback: plain `http://` to a non-loopback host is active mixed content and Chromium blocks it. `rpcUrlNeedsShellRelay()` detects this and routes the call through `invoke('relay_http_rpc', { url, token, body })`, implemented in `crates/openhuman-app/src/core_rpc.rs` (a thin wrapper over `openhuman_rpc::post_json_rpc` from `crates/openhuman-rpc/`), which returns `{ status, body }` re-wrapped as a `Response`.
4. Transport override: iOS/remote connection profiles install a `CoreTransport` (`setActiveCoreTransport`) so the same `callCoreRpc` surface rides LAN/tunnel/cloud transports.

Errors are classified into a stable `CoreRpcError.kind` (`auth_expired`, `transport`, `timeout`, `rate_limited`, …): callers branch on `kind`, never on message regexes. An `auth_expired` classification broadcasts `core-rpc-auth-expired`, which `CoreStateProvider` turns into a session clear.

### Best practices

1. Use singletons. Never create multiple service instances.
2. Keep Tauri IPC and RPC calls in services. Do not scatter `invoke()` or raw fetches through components.
3. Clean up on unmount by disconnecting in the `useEffect` cleanup.
4. Handle errors through `CoreRpcError.kind` and retry only transient failures.

---

## Providers

React context providers (`app/src/providers/`) manage service lifecycle and expose core-owned state. The full nesting (including gates that live in `components/`) is the generated [provider chain](#provider-chain) above. There is no `UserProvider`, `AIProvider` or `SkillProvider`. Auth and user state live in `CoreStateProvider`, AI configuration lives in the Rust core, and skills run in the core.

### ThemeProvider (`providers/ThemeProvider.tsx`)

Applies theme tokens and dark-mode handling from the persisted `theme` slice (mode, font size, custom themes).

### CoreStateProvider (`providers/CoreStateProvider.tsx`)

The authoritative auth/session/onboarding context. Fetches the core app snapshot (`fetchCoreAppSnapshot()` RPC), exposes it via `useCoreState()` (`{ snapshot, isBootstrapping, refresh }`), and clears the session on the global `core-rpc-auth-expired` event. It follows a turn-boundary refetch contract: after every agent reply completes (`chat_done` in `ChatRuntimeProvider`) it refetches the user state (debounced 750ms) and merges it into the snapshot via `patchSnapshot`: see `providers/README.md`.

### SocketProvider (`providers/SocketProvider.tsx`)

Owns the socket.io connection to the local core: connects once core state is ready, updates the `socket` slice, and tears down on unmount. Desktop only: `App.tsx` skips it on mobile, where events arrive through the `TunnelTransport` relay.

### ChatRuntimeProvider (`providers/ChatRuntimeProvider.tsx`)

Subscribes to chat runtime socket events (message streaming, tool calls, subagent lifecycle, approval requests) and reduces them into the `chatRuntime` slice: per-thread tool timelines, streaming buffers, artifacts, and approval state consumed by the chat surface and the mascot.

### Gates and shell-level contexts (in `components/`)

- `BootCheckGate` (`components/BootCheckGate/`) blocks render until the core boot snapshot resolves.
- `CommandProvider` (`components/commands/`) holds the command palette context.
- `ServiceBlockingGate` (`components/daemon/`) blocks the shell until required services are configured.

### Context vs Redux

| Use Context For                    | Use Redux For                      |
| ---------------------------------- | ---------------------------------- |
| Service instances (socket, client) | Serializable state (status, data)  |
| Methods (emit, on, off)            | Persisted state (sessions, tokens) |
| Derived values                     | Complex state logic                |

Example: `SocketProvider` owns the socket instance; Redux stores connection status in `socketSlice`.

---

## Human mascot surface

The mascot appears on two surfaces, deliberately. `/human`
(`app/src/features/human/HumanPage.tsx`) is the dedicated full-bleed stage with a
right-rail chat. `/chat` carries the same mascot docked on its composer, where it
expands into a voice stage in place. Both read one set of mascot preferences from
`mascotSlice` (colour, voice, speak-replies, dismissal) so the two can never
disagree about the same setting.

`app/src/features/human/chatMascot/` owns the chat-side surface:

| Module                  | Role                                                                                                      |
| ----------------------- | --------------------------------------------------------------------------------------------------------- |
| `ChatMascotContext.tsx` | Shared dock/stage refs and the send binding. Every value is stable: see the re-render note below.        |
| `ChatMascotDock.tsx`    | The small mascot standing on the composer's input box. An anchor + hit area; it draws nothing.            |
| `ChatMascotStage.tsx`   | The scaled-up voice surface: `MicComposer`, input-device selector, speak-replies switch, collapse button. |
| `ChatMascotOverlay.tsx` | The single Rive instance, moved between dock and stage with a `transform`.                                |
| `geometry.ts`           | Pure dock ⇄ stage transform maths (`inscribedSquare`, `lerpBox`, `boxTransform`).                         |

Clicking the dock expands the mascot into a right-hand stage column while the
transcript and the text composer stay live in the left column, so voice and text
are the same conversation. `pages/Accounts.tsx` animates the column width;
`ChatMascotOverlay` re-measures both anchors per frame so the mascot stays glued
to a destination that is still moving. Expanded/collapsed and the speak-replies
preference are persisted in `mascotSlice`.

Two invariants matter here. The mascot re-renders at ~60fps during TTS
lipsync, so (a) it is rendered as a leaf with nothing beneath it, and (b) the
mascot context value is deliberately non-reactive: reactive state lives in
Redux or in the send-binding external store instead. A reactive context value
would reconcile the whole chat tree every frame, which stalls the UI. And the overlay only mounts while the agent account is selected, so a fixed
overlay is never left alive as an invisible canvas still burning frames under
another account.

The mascot face comes from `useHumanMascot`, which subscribes to chat lifecycle
events for thinking, speaking, acknowledgement, and error states, plus a
`listening` pose driven by `MicComposer`'s `onRecordingChange`.

Sub-agent delegation is visualized by `SubMascotLayer`. It does not introduce a
new socket protocol. Instead, it reads the selected or active thread's
`chatRuntime.toolTimelineByThread` entries that `ChatRuntimeProvider` already
builds from `subagent_spawned`, `subagent_completed`, `subagent_failed`,
`subagent_iteration_start`, `subagent_tool_call`, and `subagent_tool_result`.

Lifecycle mapping:

| Runtime timeline state | Sub-mascot state                                                     |
| ---------------------- | -------------------------------------------------------------------- |
| `running`              | Small colored mascot in a thinking face with a short activity bubble |
| `success`              | Same mascot resolves to a happy face and completion bubble           |
| `error`                | Same mascot resolves to a concerned face and failure bubble          |

Activity bubble text is intentionally compact: current child tool call, child
iteration, the delegation prompt excerpt, or final status. The thread timeline
remains the authoritative detailed view; sub-mascots are only the glanceable
orchestration layer around the main mascot.

### Tool-call presentation

Every surface that names a tool call (chat cards, the processing panel, the
status line, the mascot) resolves it through one registry,
`app/src/features/conversations/tools/toolPresentation.ts`
(`describeToolCall`). It returns the icon, a translated phrase in two tenses
("Reading file" while running, "Read file" once settled), the target chip, and
which rich body the call expands into. The data lives in `toolSpecs.ts` (exact
names, collapsed tools that switch on an argument, prefix families, named
agents) and `toolPhrases.ts` (phrases, served as
`conversations.tools.<id>.active|done`). Composio action slugs
(`GMAIL_SEND_EMAIL`) resolve through the toolkit catalog in
`components/composio/toolkitMeta.tsx` to "Used Gmail · Send email" with the
app's logo. The server's `tool_display_label` is used only for tools the
registry cannot describe.

### assistant-ui composition

The conversation UI uses `@assistant-ui/react` for interaction and runtime
state, `@assistant-ui/react-lexical` for the rich composer, and
`@assistant-ui/react-markdown` for message rendering. The visual components
under `components/assistant-ui/` are assistant-ui registry source installed
through the app's shadcn configuration (`base-nova`), with OpenHuman tokens,
translations and product slots. Registry components are editable source; they
are not separately exported components from the npm runtime package.

| Surface                                        | assistant-ui owner                                                                   | OpenHuman adapter                                                                                                    |
| ---------------------------------------------- | ------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------- |
| Message list, scrolling and scroll-anchor pill | `ThreadPrimitive.Viewport`, `Messages`, `ScrollToBottom`                             | Real thread identity through `adapters.threadList.threadId`; run-start jumps disabled to preserve readers in history |
| Main composer, send and cancel                 | `ComposerPrimitive`, `LexicalComposerInput`                                          | Draft persistence, model selection, product voice mode and core-backed file ingestion                                |
| Message editing                                | Message-scoped `ComposerPrimitive.Root`, `Input`, `Send`, `Cancel`                   | Edit RPC and translated warning about subsequent turns                                                               |
| Message actions and branches                   | `ActionBarPrimitive`, `BranchPickerPrimitive`                                        | Adapter capability gates and core-backed regeneration, speech and feedback                                           |
| Queued follow-ups                              | `ComposerPrimitive.Queue`, `QueueItemPrimitive.Text`, `Remove`                       | Core run-queue adapter and translated registry row captions                                                          |
| Markdown, code and citations                   | `MarkdownTextPrimitive` and registry elements                                        | Source projection, app links, math and artifact access                                                               |
| Reasoning and tool groups                      | `MessagePrimitive.GroupedParts`, registry tool-group/reasoning elements              | Core timing and grouping interleaved reasoning with calls                                                            |
| Tasks and subagents                            | Registry task-card/subagent-list, `ReadonlyThreadProvider`                           | Delegation state, awaiting-user replies and worktree actions                                                         |
| Approvals, questions and plans                 | Registry approval-card, elicitation-form and agent-plan                              | Core decision RPCs, security policy and plan/workflow lifecycle                                                      |
| Todos and goals                                | Registry todo-list and agent-status                                                  | Durable harness progress and remembered disclosure state                                                             |
| Images, documents and tool results             | Registry image, artifact-card, web-search, terminal-block, code-diff and web-preview | Core artifact and tool-result projection                                                                             |
| Sources, conversation map and context          | Registry sources, conversation-map and context elements                              | Core sources, search, token usage and context breakdown RPC                                                          |

Keep runtime behavior in the library. New product functionality should use a
runtime adapter or component slot rather than a second scroll controller,
message store, editor or queue implementation. Presentation-only registry
components can remain local; OpenHuman owns the meaning and execution of its
RPC-backed controls.

Rendering uses assistant-ui's elements, vendored under
`app/src/components/assistant-ui/elements/` (tool-call, tool-timeline,
web-search, terminal-block, code-diff, web-preview) with the `tw-shimmer`
utility. `ToolGroupRoot` / `ToolGroupTrigger` / `ToolGroupContent` wrap a run of calls in the tool timeline;
`AssistantUiToolCallCard` renders each call. The adapters in
`tools/ToolBodies.tsx` only map tool data onto those elements.

The core's `tool_result` socket event carries `args`, `elapsed_ms`, the
recomputed `tool_display_label` / `tool_display_detail`, and `structured`
(the tool's `ToolResult.metadata`; web searches send
`{ kind: "web_search", query, provider, results: [...] }`).
`parseWebSearchResult.ts` prefers that payload and falls back to parsing the
text rendering for older turns.

`tools/__fixtures__/coreToolNames.json` lists every tool the core registers.
The Rust test `tools/ops_tests_catalog_fixture_tests.rs` keeps it in sync
(`UPDATE_TOOL_CATALOG=1` regenerates it) and
`toolPresentation.catalog.test.ts` fails if any listed tool falls through to
the generic fallback, so a new core tool cannot reach the chat unlabelled.
`/dev/tools` (dev builds only) renders every state and the whole catalog.

---

## Pages and routing

The application uses HashRouter with protected and public route guards. Desktop routes live in `app/src/AppRoutes.tsx`; on mobile (iOS/Android) `AppRoutesIOS.tsx` renders a reduced Human/Chat/Settings set instead.

### Route map

`app/src/AppRoutes.tsx` is the authoritative table and is heavily commented with the rationale for each entry. The desktop routes it declares:

| Route | Renders | Guard |
| --- | --- | --- |
| `/` | `Welcome` | `PublicRoute` (signed in: forwards to `/home`) |
| `/auth` | `WebCallbackPage`, the auth callback | none |
| `/callback/:kind`, `/callback/:kind/:status` | `WebCallbackPage`, generic OAuth and provider callbacks | none |
| `/onboarding/*` | `Onboarding` stepper | `ProtectedRoute` |
| `/human` | `HumanPage`, the dedicated mascot stage | `ProtectedRoute` |
| `/chat/:threadId?` | `Accounts`, the unified chat (agent plus connected web apps) | `ProtectedRoute` |
| `/flows` | `FlowsPage` | `ProtectedRoute` |
| `/flows/draft` | `FlowCanvasDraftPage`, an unsaved proposed graph passed in `location.state` | `ProtectedRoute` |
| `/flows/:id` | `FlowCanvasPage` | `ProtectedRoute` |
| `/workflows` | `Activity`, the `SKILL.md` workflow hub | `ProtectedRoute` |
| `/workflows/run` | `WorkflowsRun`, the single-purpose Skill runner | `ProtectedRoute` |
| `/connections` | `Skills`, the connections hub | `ProtectedRoute` |
| `/invites` | `Invites` | `ProtectedRoute` |
| `/notifications` | `Notifications` | `ProtectedRoute` |
| `/settings/*` | `Settings` | `ProtectedRoute` |
| `/ptt-overlay` | `PttOverlayPage`, the push-to-talk overlay window | none |
| `/dev/agent-insights` | `AgentInsightsPreview` | dev only |
| `/dev/ui` | `UiGallery`, every shared UI primitive in the active theme | dev only |
| `/dev/tools` | `ToolCallGallery`, every tool-call state and the whole core catalog | dev only |
| `/dev/assistant-ui` | The upstream assistant-ui demo on a mock runtime | dev only |
| `*` | `DefaultRedirect` | none |

The four `/dev/*` routes are registered inside an `IS_DEV` branch (`utils/config`), so `import.meta.env.DEV` is substituted at build time, the branch folds away, and their component trees leave a production bundle entirely. They are previews, never part of the shipped product.

Memory is a surface of `/connections`, not a route of its own: `?tab=brain&brain=<chip>` selects one of its eight chips, `engine`, `ask`, `explorer`, `learnings`, `conversations`, `brain`, `background` and `settings`. The list is `app/src/components/memory/memoryChips.ts` and the page is `app/src/pages/Memory.tsx`. `memoryChips.ts` also remaps the retired names (`graph`, `goals` and `context` to `ask`; `documents`, `sources`, `sync` and `history` to `brain`), and `/settings/memory-engine` redirects to the `engine` chip.

Back-compat redirects, all `Navigate replace`. The `ForwardSearch` ones copy the query string to the destination so old deep links still land on the right sub-tab:

| From | To |
| --- | --- |
| `/home` | `/chat` |
| `/accounts` | `/chat` |
| `/brain` | `/connections?tab=brain` (`BrainRedirect` remaps the old `?tab=` to `?brain=`) |
| `/skills` | `/connections` (`ForwardSearch`) |
| `/channels` | `/connections?tab=messaging` |
| `/activity` | `/settings/account` |
| `/intelligence` | `/settings/account` |
| `/feedback` | `/settings/feedback` |
| `/routines` | `/flows` |
| `/webhooks` | `/settings/integrations` (`ForwardSearch`) |

There is no `/login` route: authentication flows through the Welcome page, the `/auth` callback, and deep links. `/agents` does not exist either, and Settings is an ordinary route rather than an overlay (see [Settings](#settings)).

### Route guards

All three guards read `useCoreState()` (not Redux auth state) and render `RouteLoadingScreen` while bootstrapping:

- `ProtectedRoute` (`components/ProtectedRoute.tsx`, `({ children, requireAuth = true, redirectTo })`): without a session token, navigates to `redirectTo || '/'`. Onboarding gating is _not_ done here; an effect in `AppShellDesktop` (App.tsx) forces non-onboarding routes back to `/onboarding` while `onboarding_completed` is false, and bounces off it once complete.
- `PublicRoute` (`components/PublicRoute.tsx`): redirects signed-in users to `/home` (which forwards to `/chat`).
- `DefaultRedirect` (`components/DefaultRedirect.tsx`): signed out → `/`; signed in but onboarding incomplete → `/onboarding`; otherwise → `/chat`. Waits for `snapshot.currentUser` to avoid the post-login race.

### Onboarding flow (`pages/onboarding/`)

A routed stepper (`Onboarding.tsx` mounts nested routes inside `OnboardingLayout`):

```text
/onboarding/welcome         → WelcomePage
/onboarding/runtime-choice  → RuntimeChoicePage
  ├── cloud  → /chat
  └── custom → /onboarding/custom/inference → voice → oauth → search
               → embeddings → (activity) → vault → /chat
```

Each custom step offers Default (let OpenHuman manage it) or Configure (inline controls, or a deep-link callout to Settings for domains not yet embedded). Pages live in `pages/onboarding/pages/`; the older Composio, skills and context-gathering steps (`pages/onboarding/steps/`) are not in the default flow. Completion is tracked by the core's `onboarding_completed` flag, enforced by the AppShell onboarding gate. After onboarding, `AppWalkthrough` (Joyride) runs the post-onboarding tour.

### Settings

Settings is a routed `/settings/*` page like every other surface, on desktop and on iOS alike. It is not a modal overlay. Older names such as `SettingsPanelLayout`, `useSettingsAnimation` and `ProfilePanel` are covered in [Removed names](#removed-names) below.

- `components/settings/settingsRouteRegistry.ts`: single declarative source of truth for every settings destination (id and route slug, i18n keys, section, sidebar `navGroup`, `devOnly`, `searchKeywords`). Navigation menus and breadcrumbs derive from it.
- `components/settings/settingsRouteElements.tsx`: maps registry entries to panel `<Route>` elements, including the redirects (`/settings/memory-engine` to the Memory `engine` chip).
- `components/settings/layout/`: the two-pane chrome. `SettingsLayout` projects the settings nav into the app sidebar's dynamic region; `SettingsSidebar` groups entries by `SettingsNavGroup` (`general`, `appearance`, `agentsAutonomy`, `security`, `data`, `knowledgeMemory`, `automationIntegrations`, `diagnosticsLogs`, in `NAV_GROUP_ORDER`); `SettingsSubNav`, `SettingsIndexRedirect`, `SettingsTabbedPage` and `SettingsPanel`, the one panel template, sit beside them.
- `components/settings/panels/`: leaf panels such as `AccountPanel`, `AppearancePanel`, `ThemeStudioPanel`, `AgentAccessPanel`, `AutonomyPanel`, `McpServerPanel`, `PrivacyPanel` and `DeveloperOptionsPanel`. Adding a panel means adding the component and a registry entry; navigation and breadcrumbs pick it up automatically.
- `components/settings/controls/`: the shared form primitives every panel composes (`SettingsSwitch`, `SettingsRow`, `SettingsSection`, `SettingsSelect`, and the rest), so a boolean is a switch everywhere.

### HashRouter vs BrowserRouter

The app uses HashRouter for desktop compatibility:

```typescript
// App.tsx
import { HashRouter } from "react-router-dom";

// URLs look like: app://localhost/#/home
// Instead of: app://localhost/home
```

Why HashRouter:

1. Tauri deep links work with hash-based URLs
2. No server configuration needed
3. Works with file:// protocol
4. Prevents 404 on direct URL access

### Deep link handling

Deep links are handled before routing:

```typescript
// main.tsx
import("./utils/desktopDeepLinkListener").then((m) => {
  m.setupDesktopDeepLinkListener().catch(console.error);
});
```

The listener intercepts `openhuman://` URLs (e.g. auth handoff), exchanges tokens through the Rust side (bypassing CORS), stores the session, and navigates to the right route. See `utils/desktopDeepLinkListener.ts`.

---

## Components

Shared UI lives in `app/src/components/`; feature-specific UI lives in `app/src/features/<vertical>/`. Highlights:

```text
components/
├── ProtectedRoute / PublicRoute / DefaultRedirect   # Route guards
├── layout/shell/            # RootShellLayout, AppSidebar, SidebarSlot (two-pane app chrome)
├── settings/                # Settings registry, layout, panels, controls (see above)
├── accounts/                # Connected-app provider icons 
├── BootCheckGate/, daemon/  # Boot + service gates in the provider chain
├── commands/                # CommandProvider (command palette)
├── Announcement/, upsell/, notices/, walkthrough/    # Shell-level overlays
├── keyring/, InitProgressScreen/                     # Consent + init overlays
├── memory/                  # Memory v2 tabs (Engine, Ask, Explorer, Learnings, Conversations, Brain, Background, Settings) and import banner
└── intelligence/            # Shared intelligence UI (WorkflowsTab, Toast, ConfirmationModal)
```

Conventions:

- Modals render through a portal, above routed content. Settings is not one of them any more: it is a route.
- Modals are controlled: parents own `isOpen` state and pass `onClose`.
- All user-facing text goes through `useT()` (`lib/i18n/I18nContext`); CI enforces locale parity.
- Production `app/src` code uses only static `import` / `import type`, never dynamic imports.

---

## Hooks and utilities

### Custom hooks (`hooks/`)

`app/src/hooks/` holds the app-level hooks (the [Scale](#scale) table names the command that counts them). Representative examples:

- `useUser` is a thin wrapper over `useCoreState()`; it returns `{ user: snapshot.currentUser, isLoading, error, refetch }`. There is no standalone user store.
- `useBackendUrl` resolves the backend URL at runtime (see [Runtime config precedence](#runtime-config-precedence)).
- `useThreadQueries` fetches chat threads.
- `useDaemonHealth` / `useDaemonLifecycle` track core service health.
- `useDictationHotkey` / `usePttHotkey` manage global hotkeys.
- `useDeveloperMode`, `useMediaQuery`, `useEscapeKey` are general UI utilities.
- Feature hooks: `useFlowRunProgress`, `useWorkflowBuilderChat`, `useConsciousItems`, `useIntelligenceStats`, `useCostDashboard`, ….

Feature-local hooks live next to their feature under `features/*/`.

### Utilities

#### Configuration (`utils/config.ts`)

Centralized build-time environment variable access: never read `import.meta.env` directly elsewhere. These constants only carry the value baked into the bundle; for the runtime URL the app actually talks to, see `services/backendUrl` and `hooks/useBackendUrl`.

```typescript
// Build-time fallback only (used outside Tauri).
export const BACKEND_URL = /* VITE_BACKEND_URL || default */;
// Core RPC build-time fallback.
export const CORE_RPC_URL = /* VITE_OPENHUMAN_CORE_RPC_URL || 'http://127.0.0.1:7788/rpc' */;
// Dev flags, e.g.
export const DEV_FORCE_ONBOARDING = /* dev-only VITE_DEV_FORCE_ONBOARDING */;
```

> Do not import `BACKEND_URL` directly to make API calls. Resolve the URL at runtime so the core's `api_url` (via `openhuman.config_resolve_api_url`) takes effect:
>
> ```typescript
> // React components
> import { useBackendUrl } from "../hooks/useBackendUrl";
> const backendUrl = useBackendUrl();
>
> // Non-React code
> import { getBackendUrl } from "../services/backendUrl";
> const backendUrl = await getBackendUrl();
> ```

#### Desktop deep link listener (`utils/desktopDeepLinkListener.ts`)

Handles incoming `openhuman://` deep links via the Tauri deep-link plugin: parses the URL, performs the Rust-side token exchange (bypasses CORS), stores the session, and navigates. Set up lazily from `main.tsx` so the Tauri IPC bridge is ready first.

#### URL opener (`utils/openUrl.ts`)

Cross-platform URL opening: tries the Tauri opener plugin, falls back to `window.open`. Always use this instead of raw `window.open` so links open in the system browser.

#### Tauri command wrappers (`utils/tauriCommands/`)

Typed wrappers around `invoke(...)`, including the bridge-gap-aware `isTauri()` guard (checks `__TAURI_INTERNALS__.invoke` is actually wired, not merely that the app runs under Tauri). Use it; never check `window.__TAURI__` directly.

### Polyfills (`polyfills.ts`)

Node.js globals (`Buffer`, `process`, `util`) are polyfilled for the browser. Several browser-side modules use Node APIs, including voice/PTT audio encoding (`features/voice/pttAudio.ts`, `wavEncoder.ts`), mascot Rive asset caching (`features/human/Mascot/`), and tool-timeline formatting.

Two layers provide them:

1. `vite-plugin-node-polyfills` in `app/vite.config.ts` (`buffer`, `process`, `util`, `os`, `crypto`, `stream`, plus `Buffer`/`process`/`global` globals).
2. `polyfills.ts`, imported first in `main.tsx`, which synchronously assigns `Buffer`/`process`/`util` onto `globalThis`/`window`/`global`/`self` before any dependent module executes.

### Best practices

#### Hook dependencies and cleanup

```typescript
useEffect(() => {
  on("event", handler);
  return () => off("event", handler);
}, [on, off, handler]);
```

Always include dependencies and always clean up subscriptions.

#### Error handling

Wrap Tauri/utility calls in try-catch with a fallback:

```typescript
try {
  await openUrl(url);
} catch (error) {
  console.error("Failed to open URL:", error);
}
```

#### Type safety

Use TypeScript generics for API and RPC calls:

```typescript
const user = await apiClient.get<User>("/users/me");
const result = await callCoreRpc<Snapshot>({
  method: "openhuman.app_state_snapshot",
});
```

## Removed names

These names still turn up in older code and notes. Use the replacement:

| Removed | Replaced by |
| --- | --- |
| `SettingsModal`, `SettingsModalFrame`, `SettingsModalLayout`, `settingsOverlay.ts` | `/settings/*` as an ordinary route rendered by `Settings` |
| `SettingsPanelLayout`, `useSettingsAnimation`, `ProfilePanel` | The `settingsRouteRegistry` plus `components/settings/layout/` |
| `components/settings/search/` and `settingsSearchRegistry` | Nothing: the sidebar search field was removed. `searchKeywords` stays on the registry entries. |
| `WebviewHost` overlay in `components/accounts/` | Nothing: it went with the CEF provider webviews |
| The frontend QuickJS skills engine | Skill execution in the Rust core |
| `UserProvider`, `AIProvider`, `SkillProvider` | `CoreStateProvider` for auth and user state; AI configuration and skills in the core |
| `/conversations`, `/accounts` as standalone pages | `/chat/:threadId?` (`Accounts`), the unified chat surface |

## See also

- [Architecture](../architecture.md): the Rust side this UI presents.
- [Tauri shell](tauri-shell.md): the host that serves this bundle and owns the IPC surface.
- [Agent harness](agent-harness.md): what the chat surface's tool timeline is rendering.
- [Memory](../../features/memory.md): the user-facing shape of the `/connections?tab=brain` chips.
- [Theming](../theming.md): the token layer behind `ThemeProvider` and Theme Studio.
- [iOS companion](../../features/ios-companion.md): the experimental mobile client the `AppRoutesIOS` shell and the `transport/` profiles belong to.
