## Project Summary

Cross-platform crypto community communication platform built with **Tauri v2** (React 19 + Rust). Targets desktop (Windows, macOS) and mobile (Android, iOS). Features deep Telegram integration via MTProto, real-time Socket.io communication, V8-based skill execution engine, and an MCP (Model Context Protocol) tool system for AI-driven Telegram interactions.

## App Theme & Design System

**Design Philosophy**: Premium, sophisticated crypto platform with calm, trustworthy aesthetic.

### Color Palette

- **Primary**: Ocean blue (`#4A83DD`) optimized for dark backgrounds
- **Sage**: Success green (`#4DC46F`) for growth indicators
- **Amber**: Warning (`#E8A838`) for attention states
- **Coral**: Error (`#F56565`) soft professional red
- **Canvas**: Background layers (`#FAFAF9` to `#D4D4D1`) with subtle warmth
- **Market Colors**: Bullish green, bearish red, Bitcoin orange, Ethereum purple

### Typography

- **Primary**: Inter (premium font stack)
- **Display**: Cabinet Grotesk for headings
- **Mono**: JetBrains Mono for code
- **Scale**: Sophisticated sizing with negative letter spacing for elegance

### Component System

- **Shadows**: Glow effects, subtle to float depth levels
- **Animations**: Fade-in, slide-in, scale-in with cubic-bezier easing
- **Border Radius**: Smooth system from `xs` (0.25rem) to `5xl` (2rem)
- **Spacing**: Extended scale including custom values (4.5, 13, 15, etc.)

### Current UI State

- Uses HashRouter (not BrowserRouter) as seen in `App.tsx:1`
- 153 TypeScript files total in src/
- Sophisticated Tailwind config with custom color system and animations

## Commands

```bash
# Frontend dev server only (port 1420)
yarn dev

# Desktop dev with hot-reload (starts Vite + Tauri)
yarn tauri dev

# Desktop dev with enhanced debugging (RUST_BACKTRACE and RUST_LOG enabled)
yarn dev:app

# Production build (TypeScript compile + Vite build + Tauri bundle)
yarn tauri build

# Debug build with .app bundle (required for deep link testing on macOS)
# On macOS, openhuman:// only works when running the .app, not `tauri dev`
yarn tauri build --debug --bundles app
yarn macos:dev

# Android
yarn tauri android dev
yarn tauri android build

# iOS
yarn tauri ios dev
yarn tauri ios build

# Skills development
yarn skills:build   # Build skills in development mode
yarn skills:watch   # Watch skills for changes

# AI Configuration
yarn tools:generate # Discover tools from V8 runtime and generate TOOLS.md

# Rust checks
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
```

No test framework is currently configured. **ESLint and Prettier are configured** with Husky pre-commit/pre-push hooks for code quality enforcement.

## Architecture

### Provider Chain (App.tsx)

The app wraps in this order: `Redux Provider` → `PersistGate` → `SocketProvider` → `TelegramProvider` → `HashRouter` → `AppRoutes`. **Note**: Now uses HashRouter instead of BrowserRouter. This ordering matters because Socket.io and Telegram providers depend on Redux auth state.

### State Management (Redux Toolkit + Persist)

State lives in `src/store/` using Redux Toolkit slices:

- **authSlice** — JWT token, onboarding completion flag (persisted)
- **userSlice** — user profile
- **socketSlice** — connection status, socket ID
- **telegramSlice** — connection/auth status, chats, messages, threads (selectively persisted; loading/error states excluded)
- **aiSlice** — AI system state, memory management, session tracking
- **skillsSlice** — skills catalog, setup status, management state, V8 runtime integration
- **teamSlice** — team management, member invites, permissions

Redux Persist stores auth and telegram state (storage backend is configurable; default uses localStorage). The telegram slice has a complex nested structure in `src/store/telegram/` with separate files for types, reducers, extraReducers, and thunks.

### LocalStorage

- **Do not use `localStorage` (or `sessionStorage`) for app state or feature logic.** Use Redux (and Redux Persist where needed) instead.
- **Remove any existing `localStorage` usage** when touching related code. User-scoped data (auth, onboarding, Telegram session, socket state) lives in Redux, keyed by user id where applicable. Telegram session is in `telegram.byUser[userId].sessionString`, not localStorage.
- **Exceptions**: Redux-persist may use a localStorage-backed storage adapter by default; that is the persistence layer, not app logic. Any other remaining usage (e.g. deep-link `deepLinkHandled` flag) should be migrated to Redux or similar when that code is modified.
- **General rule**: Avoid adding new `localStorage` or `sessionStorage` usage; prefer Redux and remove existing usage when you work on affected areas.

### Service Layer (Singletons)

- **mtprotoService** (`src/services/mtprotoService.ts`) — Telegram MTProto client via `telegram` npm package. Session stored in Redux (`telegram.byUser[userId].sessionString`), not localStorage. Auto-retries FLOOD_WAIT up to 60s.
- **socketService** (`src/services/socketService.ts`) — Socket.io client. Auth token passed in socket `auth` object (not query string). Transports: polling first, then WebSocket. Enhanced with Rust-native Socket.io client for persistent connections.
- **apiClient** (`src/services/apiClient.ts`) — HTTP client for REST backend.

### MCP System (`src/lib/mcp/`)

Model Context Protocol implementation for AI tool execution over Socket.io:

- `transport.ts` — Socket.io JSON-RPC 2.0 transport with 30s timeout
- `telegram/server.ts` — TelegramMCPServer manages 99 tool definitions
- `telegram/tools/` — Individual tool files (one per Telegram API operation)
- Tools use `big-integer` library for Telegram's large integer IDs

### Routing (`src/AppRoutes.tsx`)

```
/           → Welcome (public)
/login      → Login (public)
/onboarding → Onboarding (protected, requires auth, not yet onboarded)
/home       → Home (protected, requires auth + onboarded)
*           → DefaultRedirect (routes based on auth state)
```

`PublicRoute` redirects authenticated users away. `ProtectedRoute` enforces auth and optionally onboarding status.

### Deep Link Auth Flow

Web-to-desktop handoff using `openhuman://` URL scheme:

1. User authenticates in browser
2. Browser redirects to `openhuman://auth?token=<loginToken>`
3. Tauri catches the deep link, Rust `exchange_token` command calls backend via `reqwest` (bypasses CORS)
4. Backend returns `sessionToken` + user object
5. App stores session in Redux, navigates to onboarding/home

Key file: `src/utils/desktopDeepLinkListener.ts` (lazy-loaded in `main.tsx`). Uses a `deepLinkHandled` flag to prevent infinite reload loops. Deep links do NOT work in `tauri dev` on macOS — must use built `.app` bundle.

### Rust Backend (`src-tauri/src/lib.rs`)

Enhanced Rust backend with comprehensive skill execution and runtime management:

**Core Commands:**

- `greet` — demo command
- `exchange_token` — CORS-free HTTP POST to backend for token exchange (desktop only)

**Runtime Management:**

- `discover_skills` — V8 skill discovery and manifest parsing
- `enable_skill` / `disable_skill` — skill lifecycle management
- `get_skill_preferences` / `set_skill_preferences` — skill configuration
- `connect_to_socket` — Rust-native Socket.io connection
- `get_socket_status` — connection status monitoring

**Android Support:**

- `RuntimeService` — background service for skill execution
- Notification permissions and foreground service management
- Android logging integration with logcat

Deep link plugin registered at setup. `register_all()` called only on Windows/Linux (panics on macOS).

### V8 Runtime System (`src-tauri/src/runtime/`)

Advanced JavaScript execution engine for skills using V8 (via deno_core):

**Core Components:**

- `v8_engine.rs` — V8 JavaScript runtime initialization and management
- `v8_skill_instance.rs` — Individual skill execution contexts and lifecycle
- `skill_registry.rs` — Skill discovery, registration, and state management
- `manifest.rs` — Skill manifest parsing with platform compatibility checks
- `socket_manager.rs` — Persistent Socket.io connections with reconnection logic
- `cron_scheduler.rs` — Scheduled task execution for time-based skills
- `preferences.rs` — Skill configuration and settings persistence

**Bridge System (`src-tauri/src/runtime/bridge/`):**

- `skills_bridge.rs` — Skill-to-skill communication and state sharing
- `tauri_bridge.rs` — Frontend-backend IPC and environment access
- `net.rs` — HTTP/fetch operations for skills
- `db.rs` — Database operations and storage management
- `store.rs` — Key-value storage for skill data
- `log_bridge.rs` — Structured logging from skills
- `cron_bridge.rs` — Cron job scheduling and management

**Quickjs Integration (`src-tauri/src/services/quickjs/`):**

- `service.rs` — High-level client management with V8 integration
- `bootstrap.js` — V8 JavaScript bootstrap environment
- `ops/mod.rs` — Native operations for WebSocket, timers, and async handling
- `storage.rs` — Persistent storage for sessions and data

**Platform Support:**

- Desktop platforms: Full V8 runtime with all features
- Mobile platforms: Error handling with feature availability checks
- Platform-specific skill filtering based on manifest declarations

## Environment Variables

Set in `.env` (Vite exposes `VITE_*` prefixed vars):

| Variable                     | Purpose                                                             |
| ---------------------------- | ------------------------------------------------------------------- |
| `VITE_BACKEND_URL`           | Backend API URL (default: `http://localhost:5005`)                  |
| `VITE_SENTRY_DSN`            | Sentry DSN for error reporting (optional)                           |
| `VITE_DEBUG`                 | Debug mode flag                                                     |
| `ALPHAHUMAN_DAEMON_INTERNAL` | Force internal daemon mode (default: false, uses external services) |

Production defaults are in `src/utils/config.ts`.

## AI Configuration System

OpenHumanuses an OpenClaw-compliant AI configuration system that automatically injects persona and tool context into every user message for consistent AI behavior.

### Configuration Files

All AI configuration lives in the `/ai/` directory:

- **`/ai/SOUL.md`** - AI personality, voice, tone, and behavior patterns
- **`/ai/TOOLS.md`** - Auto-generated documentation of all available tools (generated via `yarn tools:generate`)
- **`/ai/IDENTITY.md`** - Core identity and values (TODO)
- **`/ai/AGENTS.md`** - Agent roles and specializations (TODO)
- **`/ai/USER.md`** - User adaptation strategies (TODO)
- **`/ai/BOOTSTRAP.md`** - Initialization procedures (TODO)
- **`/ai/MEMORY.md`** - Long-term knowledge and patterns (TODO)

### Modular Loader System

```typescript
// Individual loaders with multi-layer caching
loadSoul() → SoulConfig     // Personality, voice, behavior
loadTools() → ToolsConfig   // Available tools and capabilities

// Unified loader
loadAIConfig() → AIConfig   // Combined SOUL + TOOLS configuration
```

**Caching Strategy:**

- Memory cache (immediate)
- localStorage cache (30min TTL)
- GitHub remote (latest)
- Bundled fallback (reliable)

**TODO**: Set up public AI configuration repository to eliminate 404 fallback errors

- Current: AI config loaders try GitHub URLs first (fail with 404), then fallback to bundled files
- Console shows: "Failed to load resource: the server responded with a status of 404"
- Affected: Settings → AI Configuration "Refresh Soul/Tools" buttons
- Files: `src/lib/ai/soul/loader.ts`, `src/lib/ai/tools/loader.ts`

### Unified Injection System

Every user message automatically gets AI context injected:

```typescript
// Unified injection (recommended)
import { injectAll } from '../lib/ai/injector';
// Individual injections (for specific needs)
import { injectSoul, injectTools } from '../lib/ai/injector';

const injectedMessage = await injectAll(userMessage);

const soulMessage = await injectSoul(userMessage);
const toolsMessage = await injectTools(userMessage);
```

**Message Format:**

```
[PERSONA_CONTEXT]
I am OpenHuman that incredibly smart, funny friend who loves helping people get stuff done
Personality: Curious & Enthusiastic, Witty & Engaging, Empathetic
Voice: Conversational, Use humor naturally but don't force it
[/PERSONA_CONTEXT]

[TOOLS_CONTEXT]
4 tools across 3 skills
Categories: Communication (2), Productivity (1), Email (1)
Key skills: telegram, notion, gmail
[/TOOLS_CONTEXT]

User message: Hello!
```

### Dynamic TOOLS.md Generation

TOOLS.md is automatically generated from the V8 skills runtime:

```bash
# Discover tools and generate documentation
yarn tools:generate

# Integration in build pipeline
yarn skills:build && yarn tools:generate && tsc && vite build
```

**Process:**

1. **Discovery**: Spawns Tauri runtime to call `runtime_all_tools()`
2. **Parsing**: Extracts tool definitions with JSON Schema
3. **Formatting**: Generates OpenClaw-compliant markdown
4. **Bundling**: Includes in app for AI context injection

**Generated Output:**

- Professional documentation with usage examples
- Environment-specific configurations
- Tool categorization by skill
- Statistics and metadata

### Integration Points

AI context injection happens in 4 places:

1. **`src/pages/Conversations.tsx`** - Main chat interface
2. **`src/store/threadSlice.ts`** - Redux sendMessage thunk
3. **`src/services/api/threadApi.ts`** - API layer
4. **`src/utils/tauriCommands.ts`** - Tauri agent chat

All use the unified `injectAll()` function for consistency.

### Settings UI

View and manage AI configuration in **Settings → AI Configuration**:

- Live SOUL personality preview
- TOOLS statistics and categories
- Individual refresh buttons
- Source indicators (GitHub vs bundled)
- Combined "Refresh All" functionality

## Recent Changes

Key updates from recent commits (cd9ebcd to current):

### Major Runtime Transition

- **V8 Runtime Migration** (`99c20ea`, `0f6a092`): Complete transition from QuickJS to V8
  - Replaced QuickJS with V8 (via deno_core) for improved JavaScript execution and WASM support
  - Enhanced skill management with V8 runtime including improved performance and compatibility
  - New V8 skill instance handling with advanced execution contexts
  - Updated dependencies and Cargo.toml to reflect V8 integration
  - Platform compatibility checks and enhanced manifest handling

### Android Platform Support

- **Full Android Integration** (`ce06cfc`, `a2578b9`): Production-ready mobile platform support
  - Complete Android project generation with MainActivity and RuntimeService
  - Background service for persistent skill execution on Android
  - Notification permission handling and foreground service management
  - Android logging integration with logcat for better debugging
  - Deep link support configuration in AndroidManifest.xml

### Enhanced Socket & Runtime Management

- **Rust-Native Socket.io Client** (`68d397e`): Persistent connection infrastructure
  - Native Rust Socket.io implementation for improved reliability
  - Enhanced socket connection handling with reconnection logic
  - Dynamic backend URL configuration support
  - Improved error handling and connection status monitoring

### Skills System Improvements

- **Advanced Skill Management** (`e841c86`, `719e6e5`): Enhanced skill lifecycle and configuration
  - Skill setup pipeline with contextual Enable/Setup/Configure/Retry buttons
  - Platform filtering for skills with manifest-based compatibility checks
  - Enhanced skill status derivation and connection indicators
  - Environment variable exposure to skills (whitelisted values)
  - Improved skill discovery and manifest processing with logging

### Major Additions

- **ESLint & Prettier Integration** (`5896966`): Complete code quality toolchain
  - ES module syntax for ESLint configuration with enhanced TypeScript support
  - Husky pre-commit/pre-push hooks for automatic formatting and linting
  - Type-only imports standardization across codebase
  - Consolidated import statements and improved code organization
  - GitHub workflows updated with Prettier and ESLint checks
- **Advanced Skills System** (`10ec1b3`): Comprehensive skill management platform
  - Dynamic skills loading from local directory via Rust integration
  - SkillSetupModal with conditional rendering (wizard vs management panel)
  - Background GitHub sync for skills catalog updates
  - Skills table with setup status indicators and management controls
  - Enhanced skill metadata with setup hooks and descriptions
- **Team Management Features** (`10ec1b3`): Multi-user collaboration system
  - TeamPanel, TeamMembersPanel, and TeamInvitesPanel components
  - Redux state management for teams, members, and invites
  - Team API integration with CRUD operations
  - Settings modal routing for team management paths
  - Role-based permissions and invitation system
- **AI System Enhancements**: Advanced memory and session management
  - Hybrid search with encryption for AI memory
  - Constitution-based AI behavior with GitHub integration
  - Entity graph migration to Neo4j backend
  - Session capture and transcript management
  - Memory chunking and context formatting
- **Enhanced CI/CD Pipeline** (`b1d7bce`): Production-ready deployment
  - XGH_TOKEN authentication for alphahumanxyz/openhuman releases
  - Python sidecar setup and caching for cross-platform builds
  - Tauri configuration updates (com.openhuman.app identifier)
  - GitHub Pages deployment with optimized workflows
  - Version tagging and environment variable management
- **Device Detection & Download System** (`9d74721`, `b5bccd2`): Enhanced multi-architecture download support
  - Optimized asset parsing using Maps for unique architecture links per platform
  - Enhanced DownloadScreen.tsx with architecture-specific download options
  - Improved device detection for Windows, macOS, Linux, and Android platforms
  - Added preference logic for more specific filenames in asset parsing
  - Support for multiple architectures (x64, aarch64) with intelligent sorting
- **Version Bump**: Project updated to v0.20.0 (`891517c`)

### Design System Updates

- **Settings Modal UI**: Clean 520px white modal contrasting with glass morphism theme
- **Animations**: 200ms entry animations, 250ms panel transitions, chevron hover effects
- **Lottie Animations**: Integrated into onboarding flow (`334673e`)
- **Connection Components**: Added Telegram and Gmail connection indicators
- **Routing**: Switched to HashRouter for better desktop app compatibility
- **Theme**: Implemented sophisticated color system with premium crypto aesthetic

### Component Structure

- **200+ TypeScript files** across `src/` directory with comprehensive tooling
- **AI System Architecture** (`src/lib/ai/`): Advanced artificial intelligence platform
  - Memory management with encryption, chunking, and hybrid search
  - Constitution-based behavior with GitHub integration
  - Entity graph with Neo4j backend integration
  - Session capture, transcript management, and tool compression
  - Provider system with OpenAI integration and custom providers
- **Skills Management System**: Dynamic skill platform with Rust integration
  - SkillsGrid.tsx - Skills catalog with setup status and management
  - SkillSetupModal.tsx - Conditional wizard/management panel rendering
  - SkillProvider.tsx - GitHub sync and local directory integration
  - Skills submodule integration with background updates
- **Team Collaboration Features**: Multi-user workspace management
  - TeamPanel.tsx - Team overview with member management
  - TeamMembersPanel.tsx - Member roles and permissions
  - TeamInvitesPanel.tsx - Invitation system with role assignment
  - Team API integration with Redux state management
- **Settings Modal System**: Comprehensive configuration interface
  - SettingsModal.tsx - Main container with URL routing
  - SettingsLayout.tsx - Modal wrapper with createPortal
  - Enhanced panels: Billing, Team, Connections, Privacy, Profile
  - Hooks: useSettingsNavigation.ts, useSettingsAnimation.ts
- **Download System**: Enhanced multi-platform distribution
  - DownloadScreen.tsx - Platform detection with architecture support
  - deviceDetection.ts - Comprehensive device/architecture utilities
  - GitHub API integration for real-time release assets
- **Code Quality Infrastructure**: ESLint, Prettier, and Husky integration
  - Pre-commit/pre-push hooks with TypeScript compilation checks
  - Standardized type-only imports and consolidated statements
  - GitHub workflow integration with automated quality checks

## Git Workflow

- **Push target**: All pushes go to the **user's private repo** (your fork). Do not push directly to the org repository.
- **PR target**: All pull requests are opened **from your fork** against the **org's private repo**, targeting the **`develop`** branch (not `main`).
- **No direct pushes to org**: The org repo does not allow direct pushes. All changes reach the org repo via PRs from your fork.

## Key Patterns

- **Code Quality**: ESLint and Prettier enforce code standards with Husky hooks. Use type-only imports (`import type`) and consolidate imports from same modules.
- **No dynamic imports**: All imports must be static `import` statements at the top of the file. Do not use `await import()` or `import().then()` inside functions or code blocks. Use try/catch around Tauri API calls for non-Tauri environments instead.
- **No localStorage**: Avoid `localStorage` and `sessionStorage`; use Redux (and persist) for app state. Remove any direct usage when working on affected code.
- **AI System Integration**: Use `src/lib/ai/` for memory management, constitution loading, entity queries, and session capture. AI providers abstracted through interface pattern.
- **AI Configuration System**: OpenClaw-compliant AI configuration with dynamic TOOLS.md generation. Use `loadSoul()`, `loadTools()`, `loadAIConfig()` for configuration loading, and `injectAll()` for unified SOUL + TOOLS injection into user messages.
- **V8 Skills Runtime**: Skills execute in V8 JavaScript engine on desktop platforms. Use `SkillProvider` for GitHub sync, `SkillsGrid` for management interface, and Rust runtime commands for lifecycle management. Platform filtering ensures skills only run on supported platforms.
- **Team Collaboration**: Team features in `src/components/settings/panels/Team*`. Use Redux `teamSlice` for state management and `teamApi` for backend operations.
- **Device Detection**: Use `deviceDetection.ts` utilities for platform/architecture detection. Support multiple architectures per platform (x64, aarch64) with intelligent preference logic.
- **GitHub Integration**: Fetch release assets via GitHub API (`fetchLatestRelease()`) and parse by architecture (`parseReleaseAssetsByArchitecture()`). Use Maps for efficient unique architecture tracking.
- **Download System**: Platform-specific file type support (.exe/.msi for Windows, .dmg for macOS, .AppImage/.deb/.rpm for Linux, .apk for Android) with fallback links.
- **Modal System**: Settings modal uses `createPortal` pattern with URL-based routing. Clean white design (not glass morphism) for system settings. Navigate with `/settings` paths for different panels.
- **Component Reuse**: Connection management reuses `connectOptions` array and components from onboarding flow. Maintains consistent UX patterns across features.
- **Redux Integration**: Multiple slices (auth, user, telegram, ai, skills, team) with Redux Persist. Use typed hooks and selectors. State functions accept optional `userId` param.
- **Node polyfills**: Vite config (`vite.config.ts`) polyfills `buffer`, `process`, `util`, `os`, `crypto`, `stream` for the `telegram` package which requires Node APIs.
- **Telegram IDs**: Use `big-integer` library, not native JS numbers (Telegram IDs exceed `Number.MAX_SAFE_INTEGER`).
- **MCP tool files**: Each tool in `src/lib/mcp/telegram/tools/` exports a handler conforming to `TelegramMCPToolHandler` interface. Tool names are typed in `src/lib/mcp/telegram/types.ts`.
- **Tauri IPC**: Frontend calls Rust via `invoke()` from `@tauri-apps/api/core`. Rust commands are registered in `generate_handler![]` macro. Enhanced with runtime management commands for V8 skill execution and Socket.io integration.
- **CORS workaround**: External HTTP requests from the WebView hit CORS. Use Rust `reqwest` via Tauri commands instead of browser `fetch()`.
- **Hash Routing**: Uses HashRouter for desktop app compatibility and deep link handling.
- **Integration Libraries**: Each integration (Telegram, future Gmail, etc.) lives under `src/lib/<integration>/` with its own `state/`, `services/`, `api/` subdirectories. Domain-specific services belong in the integration folder, not in `src/services/` (which holds only cross-cutting services like socketService, apiClient).
- **Unit Tests**: All unit tests live in `__tests__/` folders co-located with the code they test. Use Jest with TypeScript support.
- **Runtime Platform Differences**: V8 runtime is desktop-only. Mobile platforms use feature detection and graceful degradation. Skills with platform restrictions are filtered during discovery.
- **Socket Management**: Rust-native Socket.io client provides persistent connections with automatic reconnection. Use `connect_to_socket` command instead of frontend-only socket connections for reliability.
- **Dual Socket Codebase**: Socket event handling exists in **both** the TypeScript frontend (`src/services/socketService.ts`, `src/utils/tauriSocket.ts`) and the Rust backend (`src-tauri/src/runtime/socket_manager.rs`). **Any new socket event or protocol change must be implemented in both codebases.** The web frontend handles events directly via Socket.io; the Rust backend handles them over raw WebSocket with Engine.IO/Socket.IO framing. Example: `tool:sync` is emitted from both `src/lib/skills/sync.ts` (web mode) and `socket_manager.rs` (Rust mode, on connect + skill lifecycle changes).

## Platform Gotchas

- **macOS deep links**: Require `.app` bundle (not `tauri dev`). Clear WebKit caches when debugging stale content: `rm -rf ~/Library/WebKit/com.openhuman.app ~/Library/Caches/com.openhuman.app`
- **Cargo caching**: May serve stale frontend assets on incremental builds. Run `cargo clean --manifest-path src-tauri/Cargo.toml` if the app shows outdated UI.
- **`window.__TAURI__`**: Not available at module load time. Use static imports and try/catch around Tauri API calls (not around imports).
- **Android background services**: RuntimeService requires notification permissions (API 33+) and foreground service type specification (API 34+). Use Android logging (`android_logger`) for debug output in logcat.
- **V8 runtime limitations**: V8 engine is desktop-only. Android skills should use lightweight alternatives or server-side execution patterns.
- **Socket connections**: Persistent Socket.io connections via Rust backend work better than WebView-based connections on mobile platforms.
