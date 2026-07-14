---
description: >-
  The React + Vite frontend (`app/src/`) - architecture, state, services,
  providers, routing, components, hooks.
icon: browsers
---

# Frontend (app/src/)

The OpenHuman desktop UI: a Vite + React 19 tree under `app/src/` (pnpm workspace `openhuman-app`). It uses Redux Toolkit with persistence for session state, talks to the in-process Rust core over JSON-RPC (`coreRpcClient` → local HTTP, with the Tauri `relay_http_rpc` command as a fallback relay) and socket.io (`socketService`), and reaches the cloud backend via REST (`apiClient`). Heavy logic lives in the core, not here.

This is one consolidated reference. Use the table of contents above (or your reader's outline) to jump between sections.

## Quick reference

| Section                                           | Covers                                                          |
| ------------------------------------------------- | --------------------------------------------------------------- |
| [Architecture](frontend.md#architecture-overview) | Provider chain, build, layout, conventions                      |
| [State Management](frontend.md#state-management)  | Redux Toolkit slices, selectors, persistence                    |
| [Services Layer](frontend.md#services-layer)      | `apiClient`, `socketService`, `coreRpcClient`                   |
| [Providers](frontend.md#providers)                | `ThemeProvider`, `CoreState`, `Socket`, `ChatRuntime` providers |
| [Pages & Routing](frontend.md#pages-routing)      | `HashRouter`, route guards, main routes                         |
| [Components](frontend.md#components)              | UI / settings component patterns                                |
| [Hooks & Utilities](frontend.md#hooks-utilities)  | Shared hooks, helpers, config                                   |

## Scale

| Metric                                  | Value                                                                     |
| --------------------------------------- | ------------------------------------------------------------------------- |
| TypeScript / TSX files under `app/src/` | \~1700 (`find app/src -name '*.ts' -o -name '*.tsx' \| wc -l` to refresh) |
| Test runner                             | Vitest (`app/test/vitest.config.ts`)                                      |

## Directory layout

```
app/src/
├── App.tsx                 # Provider chain + HashRouter shell (desktop + mobile shells)
├── AppRoutes.tsx           # Desktop route table (AppRoutesIOS.tsx for mobile)
├── main.tsx                # Entry (polyfills, Sentry, store, styles)
├── store/                  # Redux slices, selectors, userScopedStorage persistence
├── providers/              # ThemeProvider, CoreStateProvider, SocketProvider, ChatRuntimeProvider
├── services/               # apiClient, socketService, coreRpcClient, transport/, api/* (~50 modules)
├── lib/                    # AI prompt loaders, i18n, MCP helpers, platform, tunnel crypto
├── pages/                  # Route-level screens (incl. onboarding/, ios/, dev/)
├── features/               # Feature verticals (human/, conversations/, meet/, voice/)
├── components/             # Shared UI (incl. settings/, layout/shell/, accounts/)
├── agentworld/             # tiny.place Agent World surface (/agent-world/*)
├── hooks/                  # App hooks
├── utils/                  # Config, Tauri command wrappers, routing utilities
└── assets/                 # Icons and static assets
```

## Architecture overview

### System architecture

OpenHuman’s desktop UI is a **React 19** app (`app/src/`) that:

- Uses **Redux Toolkit** with persistence for session-related state
- Connects to the backend with **REST** (`apiClient`) and to the local core with **Socket.io** (`socketService` → core socket endpoint)
- Calls the **Rust core** (embedded in the Tauri host as a tokio task) over HTTP via **`coreRpcClient`** (JSON-RPC methods implemented in repo root `src/openhuman/`); non-loopback plain-http runtimes are relayed through the Tauri **`relay_http_rpc`** command
- Leaves **AI prompts** to the core: bundled `src/openhuman/agent/prompts` (repo root) ship as Tauri resources and are read core-side, not by the frontend
- Uses a **minimal MCP-style** helper layer under `lib/mcp/` (transport, validation)

### Entry points

| File                    | Purpose                                                                          |
| ----------------------- | -------------------------------------------------------------------------------- |
| `app/src/main.tsx`      | React root, polyfills, Sentry boundary, store, global styles                     |
| `app/src/App.tsx`       | Provider chain (see below) + desktop/mobile shells, Settings modal overlay       |
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

**Why this order**

1. Redux `Provider` is outermost so `useAppSelector` / dispatch work everywhere.
2. `PersistGate` rehydrates persisted slices before children assume stable auth/session.
3. `BootCheckGate` / `CoreStateProvider` resolve the core boot snapshot (auth, onboarding) before feature providers mount.
4. `SocketProvider` (desktop only) and `ChatRuntimeProvider` depend on that core state for realtime events and approvals.
5. `Router` supplies navigation to all routes.

### Module relationships (simplified)

```
App.tsx
  ├─ Redux store + persistor
  ├─ ThemeProvider / I18nProvider - theme tokens, useT() localization
  ├─ BootCheckGate - waits for the core boot snapshot
  ├─ CoreStateProvider - auth/session/onboarding snapshot (fetchCoreAppSnapshot RPC)
  ├─ SocketProvider - socket.io connection to the local core (desktop only)
  ├─ ChatRuntimeProvider - chat streaming, tool timeline, approvals → Redux
  └─ AppShell (desktop or mobile)
       ├─ AppRoutes - PublicRoute / ProtectedRoute / DefaultRedirect
       ├─ SettingsModal - overlay mounted when the URL is /settings/*
       └─ WebviewHost - active connected-app CEF webview overlay
```

### Services layer (conceptual)

```
services/
  ├─ apiClient        → REST to a URL resolved at runtime via `services/backendUrl#getBackendUrl`
  ├─ backendUrl       → Calls `openhuman.config_resolve_api_url`; falls back to VITE_BACKEND_URL only outside Tauri
  ├─ socketService    → Socket.io to the local core (base URL derived from the RPC URL); MCP-style envelopes
  ├─ coreRpcClient    → JSON-RPC over HTTP to the local openhuman core; `relay_http_rpc` fallback for non-loopback http
  └─ transport/       → ConnectionProfile transports for iOS/remote (LanHttp, Tunnel, CloudHttp)
```

#### Runtime config precedence

The desktop app does not bake the core RPC URL or the API host into the bundle as a hard requirement. At runtime the app resolves them in this order (highest first):

1. **Welcome-screen RPC URL field**, saved via `utils/configPersistence` and restored on next launch. End users configure a self-hosted core address here, not by hand-editing `config.toml` or `.env` files.
2. **Tauri `core_rpc_url` command**, the port the embedded core is listening on for this process.
3. **`VITE_OPENHUMAN_CORE_RPC_URL`**, build-time fallback for development.
4. The hardcoded `http://127.0.0.1:7788/rpc` default.

Once the RPC handshake succeeds, `services/backendUrl` calls `openhuman.config_resolve_api_url` to pull `api_url` (and other safe client fields) from the loaded core `Config`. `VITE_BACKEND_URL` is only used as a web fallback when the app runs outside Tauri.

Components that need the backend URL should call `useBackendUrl()` (or `getBackendUrl()` from non-React code), they must not import the static `BACKEND_URL` constant from `utils/config`, which represents the build-time value only.

### Related docs

- Rust architecture: [Architecture](../architecture.md)
- Tauri shell: [Tauri Shell](tauri-shell.md)

## State Management

The application uses Redux Toolkit with Redux-Persist. There is no single root persist config: each slice that persists wraps its own reducer with `persistReducer` in **`store/index.ts`**, whitelisting exactly the fields that should survive a restart.

### Storage backends

- **`userScopedStorage`** (`store/userScopedStorage.ts`) — the default storage for persisted slices. Blobs are keyed `${userId}:persist:<key>` so state never leaks across users on logout/login (#900).
- **Plain `localStorage`** — used only for pre-login, device-wide slices (`coreMode`, `locale`, `theme`) that must survive user switches.

### Slices

Authoritative list = the `reducer` map in `store/index.ts`. One-line purposes:

| Slice                | Purpose                                                                 | Persisted?                                                     |
| -------------------- | ----------------------------------------------------------------------- | -------------------------------------------------------------- |
| `accounts`           | Connected web-app (CEF webview) accounts + rail ordering                | `accounts`, `order`, `lastActiveAccountId` (not the active id) |
| `agentProfiles`      | Agent profile data                                                      | no                                                             |
| `announcement`       | Harness-init announcement banner, seen ids                              | `shownIds`                                                     |
| `backendMeet`        | Backend-driven Google Meet call state (join/leave, transcript, replies) | no                                                             |
| `channelConnections` | Messaging channel connections (WhatsApp, Slack, …)                      | connections + migration/default-channel fields                 |
| `chatRuntime`        | Streaming buffers, tool timelines, inference status, artifacts          | only `artifactsByThread` (ready snapshots)                     |
| `companion`          | Companion overlay state                                                 | no                                                             |
| `connectivity`       | navigator.onLine + backend/core health status                           | no                                                             |
| `coreMode`           | Pre-login core mode selection (embedded / self-hosted / cloud)          | `mode` (plain localStorage)                                    |
| `layout`             | Two-pane layout geometry (sidebar visibility, dragged widths)           | `panels`                                                       |
| `locale`             | UI language                                                             | `current` (plain localStorage)                                 |
| `mascot`             | Mascot appearance / voice selection                                     | `color`, `voiceId`, `customMascotGifUrl`, `selectedMascotId`   |
| `notifications`      | Notification items + preferences                                        | `items`, `preferences`                                         |
| `persona`            | Cosmetic persona display name + description (SOUL.md lives in the core) | `displayName`, `description`                                   |
| `providerSurfaces`   | Provider webview surface state                                          | no                                                             |
| `ptt`                | Push-to-talk hotkey + session prefs (`isHeld` deliberately excluded)    | `shortcut`, `speakReplies`, `showOverlay`                      |
| `socket`             | Per-user socket connection status / socket ids                          | no (reconnects on boot)                                        |
| `theme`              | Theme mode, font size, message view mode, custom themes                 | plain localStorage                                             |
| `thread`             | Chat thread list + per-thread message caches                            | only `selectedThreadId`                                        |
| `userErrors`         | User-actionable runtime errors (#3931)                                  | no (in-memory only)                                            |

Ephemeral chat state (streaming buffers, tool timelines) must **not** survive a restart — the UI would try to resume a turn whose live driver is gone. The one exception, agent-generated artifacts, goes through the `artifactsReadyOnlyTransform` in `store/index.ts` (pure logic in `store/artifactsPersistFilter.ts`).

### Typed hooks

**File:** `store/hooks.ts`

```typescript
// Use these instead of plain useDispatch/useSelector
export const useAppDispatch: () => AppDispatch = useDispatch;
export const useAppSelector: TypedUseSelectorHook<RootState> = useSelector;
```

### Best practices

1. **Always use typed hooks** — `useAppDispatch` and `useAppSelector`.
2. **Use selectors for derived state** — see `store/socketSelectors.ts`, `store/connectivitySelectors.ts`, `store/userErrorsSelectors.ts`.
3. **Whitelist persistence per slice** — never persist transient/loading state; add a per-slice `persistReducer` in `store/index.ts`.
4. **Prefer Redux over ad-hoc `localStorage`** — plain localStorage is reserved for the pre-login slices noted above.
5. In dev / E2E builds the store is exposed as `window.__OPENHUMAN_STORE__` so WDIO specs can assert backing state; production bundles do not expose it.

---

## Services Layer

The application uses singleton services for external communication. This prevents connection leaks and provides consistent API access.

### Service architecture

```
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

### API Client (`services/apiClient.ts`)

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

\~50 domain-scoped modules, one per feature surface, each wrapping either backend REST endpoints or core RPC methods. Representative examples:

- `authApi` / `userApi` — auth + user profile
- `threadApi`, `threadGoalApi`, `threadUsageApi` — chat threads
- `agentProfilesApi`, `agentTeamApi`, `agentWorkApi`, `subagentApi` — agents
- `skillsApi`, `skillRegistryApi`, `flowsApi`, `workflowRunsApi`, `todosApi` — skills & automation
- `channelConnectionsApi`, `mcpClientsApi`, `mcpSetupApi`, `tunnelsApi` — connections
- `memoryTimelineApi`, `memoryFreshnessApi`, `graphCentralityApi`, `namespaceOverviewApi` — memory/graph
- `billingApi`, `creditsApi`, `referralApi`, `rewardsApi`, `inviteApi` — commerce
- `voiceSettingsApi`, `voiceInstallApi`, `aiSettingsApi`, `modelCouncilApi` — AI/voice config

For the full list, `ls app/src/services/api/`. New feature surfaces get their own module here rather than growing `apiClient`.

### Socket Service (`services/socketService.ts`)

Socket.io client singleton connected to the **local core's** socket endpoint (base URL derived from the resolved RPC URL via `coreSocket.ts`; authenticated with the core RPC token). It ingests realtime core events — chat/meet/channel/companion updates — and dispatches them into Redux (`socketSlice`, `backendMeetSlice`, `channelConnectionsSlice`, `companionSlice`, `connectivitySlice`). It also hosts the MCP-style transport (`SocketIOMCPTransportImpl` from `lib/mcp`).

Keep `socketService` and the core socket behavior aligned (the "dual socket sync" rule in AGENTS.md). Connection lifecycle is owned by `providers/SocketProvider.tsx`; on mobile the provider is not mounted at all — events arrive through the `TunnelTransport` relay instead.

### Core RPC (`services/coreRpcClient.ts`)

The Rust core runs **in-process** inside the Tauri host (no sidecar). The UI calls JSON-RPC methods on it over local HTTP:

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

1. **URL + token resolution** — the RPC URL follows the precedence in [Runtime config precedence](frontend.md#runtime-config-precedence); the per-launch bearer token comes from the Tauri `core_rpc_token` command (or the stored token for self-hosted cores).
2. **Direct fetch** — the webview `fetch()`es the JSON-RPC envelope straight to the core (loopback http or any https URL).
3. **Shell relay fallback** — plain `http://` to a **non-loopback** host is active mixed content and Chromium blocks it (#3865). `rpcUrlNeedsShellRelay()` detects this and routes the call through `invoke('relay_http_rpc', { url, token, body })`, implemented in **`app/src-tauri/src/core_rpc.rs`**, which returns `{ status, body }` re-wrapped as a `Response`.
4. **Transport override** — iOS/remote connection profiles install a `CoreTransport` (`setActiveCoreTransport`) so the same `callCoreRpc` surface rides LAN/tunnel/cloud transports.

Errors are classified into a stable `CoreRpcError.kind` (`auth_expired`, `transport`, `timeout`, `rate_limited`, …) — callers branch on `kind`, never on message regexes. An `auth_expired` classification broadcasts `core-rpc-auth-expired`, which `CoreStateProvider` turns into a session clear.

### Best Practices

1. **Use singletons** — never create multiple service instances.
2. **Keep Tauri IPC and RPC calls in services** — do not scatter `invoke()` or raw fetches through components.
3. **Clean up on unmount** — disconnect in `useEffect` cleanup.
4. **Handle errors via `CoreRpcError.kind`** — retry only transient failures.

---

## Providers

React context providers (`app/src/providers/`) manage service lifecycle and expose core-owned state. The full nesting (including gates that live in `components/`) is the generated [provider chain](frontend.md#provider-chain) above. There is **no** `UserProvider`, `AIProvider`, or `SkillProvider` — auth/user state lives in `CoreStateProvider`, AI configuration lives in the Rust core, and skills execute in the core (the frontend QuickJS skills engine was removed).

### ThemeProvider (`providers/ThemeProvider.tsx`)

Applies theme tokens and dark-mode handling from the persisted `theme` slice (mode, font size, custom themes).

### CoreStateProvider (`providers/CoreStateProvider.tsx`)

The authoritative auth/session/onboarding context. Fetches the core app snapshot (`fetchCoreAppSnapshot()` RPC), exposes it via `useCoreState()` (`{ snapshot, isBootstrapping, refresh }`), and clears the session on the global `core-rpc-auth-expired` event. It follows a **turn-boundary refetch contract**: after every agent reply completes (`chat_done` in `ChatRuntimeProvider`) it refetches the user state (debounced 750ms) and merges it into the snapshot via `patchSnapshot` — see `providers/README.md`.

### SocketProvider (`providers/SocketProvider.tsx`)

Owns the socket.io connection to the local core: connects once core state is ready, updates the `socket` slice, and tears down on unmount. Desktop only — `App.tsx` skips it on mobile, where events arrive through the `TunnelTransport` relay.

### ChatRuntimeProvider (`providers/ChatRuntimeProvider.tsx`)

Subscribes to chat runtime socket events (message streaming, tool calls, subagent lifecycle, approval requests) and reduces them into the `chatRuntime` slice — per-thread tool timelines, streaming buffers, artifacts, and approval state consumed by the chat surface and the mascot.

### Gates and shell-level contexts (in `components/`)

- **`BootCheckGate`** (`components/BootCheckGate/`) — blocks render until the core boot snapshot resolves.
- **`CommandProvider`** (`components/commands/`) — command palette context.
- **`ServiceBlockingGate`** (`components/daemon/`) — blocks the shell until required services are configured.

### Context vs Redux

| Use Context For                    | Use Redux For                      |
| ---------------------------------- | ---------------------------------- |
| Service instances (socket, client) | Serializable state (status, data)  |
| Methods (emit, on, off)            | Persisted state (sessions, tokens) |
| Derived values                     | Complex state logic                |

Example: `SocketProvider` owns the socket instance; Redux stores connection status in `socketSlice`.

---

## Human Mascot Surface

The Human page (`app/src/features/human/HumanPage.tsx`) renders the main
`YellowMascot` beside the conversation sidebar. The mascot face still comes
from `useHumanMascot`, which subscribes to chat lifecycle events for thinking,
speaking, acknowledgement, and error states.

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

---

## Pages & Routing

The application uses HashRouter with protected and public route guards. Desktop routes live in **`app/src/AppRoutes.tsx`**; on mobile (iOS/Android) `AppRoutesIOS.tsx` renders a reduced Human/Chat/Settings set instead.

### Route map

Current desktop routes (read `AppRoutes.tsx` for the authoritative table — the file is heavily commented with the rationale for each redirect):

```
/                      → Welcome (PublicRoute; redirects to /home if logged in)
/auth                  → WebCallbackPage (auth callback)
/callback/:kind[/:status] → WebCallbackPage (generic OAuth/provider callbacks)
/onboarding/*          → Onboarding stepper (ProtectedRoute)
/human                 → HumanPage (mascot surface)
/brain                 → Brain (memory knowledge-graph)
/flows                 → FlowsPage · /flows/draft → draft canvas · /flows/:id → FlowCanvasPage
/orchestration         → OrchestrationPage (TinyPlace multi-agent coordination)
/workflows/run         → WorkflowsRun (single-purpose Skill runner)
/connections           → Skills page (connections hub)
/chat/:threadId?       → Accounts (unified chat: agent + connected web apps)
/invites               → Invites
/feedback              → Feedback
/notifications         → Notifications
/rewards               → Rewards
/ptt-overlay           → PttOverlayPage (push-to-talk overlay window)
/dev/agent-insights    → dev-only preview
/agent-world/*         → AgentWorld (tiny.place A2A social network)
*                      → DefaultRedirect
```

Back-compat redirects (all `Navigate replace`, query params preserved):

```
/home        → /chat                     /skills      → /connections
/activity    → /settings/notifications   /channels    → /connections?tab=messaging
/intelligence→ /settings/notifications   /routines    → /settings/automations
/workflows   → /settings/automations     /webhooks    → /settings/integrations#webhooks
/brain/tinyplace-orchestration → /orchestration
```

There is **no** `/login` route — authentication flows through the Welcome page, the `/auth` callback, and deep links. Desktop **Settings is not an inline route**: when the URL is `/settings/*`, `AppShellDesktop` keeps rendering the _background_ location and mounts `SettingsModal` on top (see [Settings](frontend.md#settings)). Note that `/agents` does not exist; the agent-social surface is `/agent-world/*`.

### Route guards

All three guards read `useCoreState()` (not Redux auth state) and render `RouteLoadingScreen` while bootstrapping:

- **`ProtectedRoute`** (`components/ProtectedRoute.tsx`) — `({ children, requireAuth = true, redirectTo })`; without a session token, navigates to `redirectTo || '/'`. Onboarding gating is _not_ done here — an effect in `AppShellDesktop` (App.tsx) forces non-onboarding routes back to `/onboarding` while `onboarding_completed` is false, and bounces off it once complete.
- **`PublicRoute`** (`components/PublicRoute.tsx`) — redirects signed-in users to `/home` (which forwards to `/chat`).
- **`DefaultRedirect`** (`components/DefaultRedirect.tsx`) — signed out → `/`; signed in but onboarding incomplete → `/onboarding`; otherwise → `/chat`. Waits for `snapshot.currentUser` to avoid the post-login race.

### Onboarding Flow (`pages/onboarding/`)

A routed stepper (`Onboarding.tsx` mounts nested routes inside `OnboardingLayout`):

```
/onboarding/welcome         → WelcomePage
/onboarding/runtime-choice  → RuntimeChoicePage
  ├── cloud  → /chat
  └── custom → /onboarding/custom/inference → voice → oauth → search
               → embeddings → (activity) → vault → /chat
```

Each custom step offers **Default** (let OpenHuman manage it) vs **Configure** (inline controls, or a deep-link callout to Settings for domains not yet embedded). Pages live in `pages/onboarding/pages/`; the legacy Composio/skills/context-gathering steps (`pages/onboarding/steps/`) are retired from the default flow but remain on disk. Completion is tracked by the core's `onboarding_completed` flag, enforced by the AppShell onboarding gate. After onboarding, `AppWalkthrough` (Joyride) runs the post-onboarding tour.

### Settings

Settings is a full `/settings/*` URL surface, presented on desktop as a **modal overlay** and on iOS as a full page. The old `SettingsPanelLayout` / `useSettingsAnimation` / `ProfilePanel` modal system is gone.

- **`components/settings/settingsRouteRegistry.ts`** — single declarative source of truth for every settings destination (id/route slug, i18n keys, section, sidebar `navGroup`, `devOnly`, search keywords). Navigation menus, breadcrumbs, and settings search all derive from it.
- **`components/settings/settingsRouteElements.tsx`** — maps registry entries to panel `<Route>` elements.
- **`components/settings/modal/`** — `SettingsModal` (mounted by `AppShellDesktop` whenever the path is a settings path; `settingsOverlay.ts` computes `{ settingsOpen, baseLocation }` so the page behind stays rendered), `SettingsModalFrame` (backdrop / Esc / focus / close), `SettingsModalLayout` (routed two-column layout).
- **`components/settings/layout/`** — two-pane chrome: `SettingsLayout`, `SettingsSidebar` (grouped by `SettingsNavGroup`: general, assistant, data, connections, knowledge & memory, agents & autonomy, models & inference, automation & integrations, diagnostics & logs), `SettingsSubNav`, `SettingsIndexRedirect`.
- **`components/settings/panels/`** — \~50 leaf panels (`AccountPanel`, `AppearancePanel`, `AIPanel`, `AgentsPanel`, `AgentAccessPanel`, `AutonomyPanel`, `BillingPanel`, `CronJobsPanel`, `IntegrationsPanel`, `McpServerPanel`, `NotificationsTabbedPanel`, `PrivacyPanel`, `DeveloperOptionsPanel`, …). Adding a panel = add the component + a registry entry; nav, breadcrumbs, and search pick it up automatically.
- **`components/settings/search/`** — settings search bar + registry-derived index.

### HashRouter vs BrowserRouter

The app uses HashRouter for desktop compatibility:

```typescript
// App.tsx
import { HashRouter } from "react-router-dom";

// URLs look like: app://localhost/#/home
// Instead of: app://localhost/home
```

**Why HashRouter:**

1. Tauri deep links work with hash-based URLs
2. No server configuration needed
3. Works with file:// protocol
4. Prevents 404 on direct URL access

### Deep Link Handling

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

```
components/
├── ProtectedRoute / PublicRoute / DefaultRedirect   # Route guards
├── layout/shell/            # RootShellLayout, AppSidebar, SidebarSlot (two-pane app chrome)
├── settings/                # Settings registry, modal, layout, panels, search (see above)
├── accounts/                # WebviewHost + connected-app (CEF webview) surfaces
├── BootCheckGate/, daemon/  # Boot + service gates in the provider chain
├── commands/                # CommandProvider (command palette)
├── Announcement/, upsell/, userErrors/, walkthrough/  # Shell-level overlays
├── keyring/, mcp-setup/, InitProgressScreen/          # Consent + init overlays
└── intelligence/            # Memory/vault surfaces (ObsidianVaultSection, VaultHealthChecklist, WorkflowsTab, …)
```

Conventions:

- **Modal via portal** — shell modals (Settings, link modal) render above routed content; the Settings modal uses the backgroundLocation pattern rather than unmounting the page underneath.
- **Controlled modals** — parents own `isOpen` state and pass `onClose`.
- **i18n everywhere** — all user-facing text goes through `useT()` (`lib/i18n/I18nContext`); CI enforces locale parity.
- **No dynamic imports** in production `app/src` code — static `import` / `import type` only.

---

## Hooks & Utilities

### Custom Hooks (`hooks/`)

\~40 app-level hooks. Representative examples:

- **`useUser`** — thin wrapper over `useCoreState()`; returns `{ user: snapshot.currentUser, isLoading, error, refetch }`. There is no standalone user store.
- **`useBackendUrl`** — runtime backend URL resolution (see [Runtime config precedence](frontend.md#runtime-config-precedence)).
- **`useThreadQueries`** — chat thread fetching.
- **`useDaemonHealth` / `useDaemonLifecycle`** — core service health.
- **`useDictationHotkey` / `usePttHotkey`** — global hotkey managers.
- **`useDeveloperMode`**, **`useMediaQuery`**, **`useEscapeKey`**, **`useStickToBottom`** — UI utilities.
- Feature hooks: `useFlowRunProgress`, `useWorkflowBuilderChat`, `useConsciousItems`, `useSubconscious`, `useIntelligenceStats`, `useCostDashboard`, ….

Feature-local hooks live next to their feature under `features/*/`.

### Utilities

#### Configuration (`utils/config.ts`)

Centralized build-time environment variable access — **never read `import.meta.env` directly elsewhere**. These constants only carry the value baked into the bundle; for the **runtime** URL the app actually talks to, see `services/backendUrl` and `hooks/useBackendUrl`.

```typescript
// Build-time fallback only (used outside Tauri).
export const BACKEND_URL = /* VITE_BACKEND_URL || default */;
// Core RPC build-time fallback.
export const CORE_RPC_URL = /* VITE_OPENHUMAN_CORE_RPC_URL || 'http://127.0.0.1:7788/rpc' */;
// Dev flags, e.g.
export const DEV_FORCE_ONBOARDING = /* dev-only VITE_DEV_FORCE_ONBOARDING */;
```

> **Do not** import `BACKEND_URL` directly to make API calls. Resolve the URL at runtime so the core's `api_url` (via `openhuman.config_resolve_api_url`) takes effect:
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

#### Desktop Deep Link Listener (`utils/desktopDeepLinkListener.ts`)

Handles incoming `openhuman://` deep links via the Tauri deep-link plugin: parses the URL, performs the Rust-side token exchange (bypasses CORS), stores the session, and navigates. Set up lazily from `main.tsx` so the Tauri IPC bridge is ready first.

#### URL Opener (`utils/openUrl.ts`)

Cross-platform URL opening — tries the Tauri opener plugin, falls back to `window.open`. Always use this instead of raw `window.open` so links open in the system browser.

#### Tauri command wrappers (`utils/tauriCommands/`)

Typed wrappers around `invoke(...)`, including the bridge-gap-aware `isTauri()` guard (checks `__TAURI_INTERNALS__.invoke` is actually wired, not merely that the app runs under Tauri). Use it — never check `window.__TAURI__` directly.

### Polyfills (`polyfills.ts`)

Node.js globals (`Buffer`, `process`, `util`) polyfilled for the browser. Several browser-side modules use Node APIs — e.g. voice/PTT audio encoding (`features/voice/pttAudio.ts`, `wavEncoder.ts`), mascot Rive asset caching (`features/human/Mascot/`), the Meet mascot frame producer, and tool-timeline formatting.

Two layers provide them:

1. **`vite-plugin-node-polyfills`** in `app/vite.config.ts` (`buffer`, `process`, `util`, `os`, `crypto`, `stream`, plus `Buffer`/`process`/`global` globals).
2. **`polyfills.ts`**, imported **first** in `main.tsx`, which synchronously assigns `Buffer`/`process`/`util` onto `globalThis`/`window`/`global`/`self` before any dependent module executes.

### Best Practices

#### Hook dependencies & cleanup

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

---
