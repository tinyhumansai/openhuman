---

```md
# Cross-Platform Tauri Agent Assistant — MVP Specification

## Overview

This MVP defines a **Telegram-based Agent Assistant** built with **Tauri (Rust + Web UI)** targeting:

- Windows
- macOS
- Android
- iOS

The assistant:
- Interacts with users via a **Telegram bot (DM-first)**
- Processes Telegram channel data **locally on device**
- Uses a **minimal backend** only for:
  - identity & login
  - payments & entitlements
  - push notifications (especially for iOS)
- Avoids storing Telegram message content on servers

---

## Core Platform Behavior Summary

| Platform | Listening Model                 | Trigger to Respond         |
| -------- | ------------------------------- | -------------------------- |
| Windows  | Continuous (background)         | Bot DM or channel activity |
| macOS    | Continuous (background)         | Bot DM or channel activity |
| Android  | Continuous (foreground service) | Bot DM or channel activity |
| iOS      | On-demand only                  | Bot DM → push → tap → sync |

---

## Architectural Pillars

- **UI-first development**
- **Single Rust agent runtime**
- **Telegram bot as the user interface**
- **Privacy-first local processing**
- **Backend as infrastructure, not intelligence**

---

# PHASED MVP PLAN

---

## Phase 0 — Project Skeleton & Tooling

### Goals

- Prepare repo structure
- Establish documentation and contribution rules
- No business logic yet

### Deliverables

- Monorepo structure
- Tauri project scaffold
- Mobile targets enabled (Tauri v2)
- CI hooks (optional)

### Repo Structure

```

/apps
/desktop
/mobile
/core
/agent-runtime (Rust)
/storage
/telegram
/backend
/docs

```

### Documentation Commands (Required)

```bash
/docs/architecture.md        # high-level system design
/docs/decisions/ADR-000.md   # initial architecture decision record
```

### Exit Criteria

- App builds and runs (blank UI)
- Docs folder initialized
- ADR process agreed upon

---

## Phase 1 — UI-First MVP (No Logic)

### Goals

Build the **entire UI flow** before implementing logic.

### UI Screens

- Login / Signup
- Telegram Connect (bot instructions)
- Channel Selection
- Sync Status Screen
- Settings (background, privacy, storage)
- Plan & Billing (stub)
- Logs / Activity View (local only)

### Platforms

- All platforms (desktop + mobile)

### Deliverables

- Responsive UI
- Navigation between screens
- Mock data only
- No Telegram, no backend, no Rust logic

### Documentation Commands

```bash
/docs/ui/flows.md            # user flows
/docs/ui/screens.md          # screen definitions
/docs/ui/states.md           # loading / error / empty states
```

### Exit Criteria

- Entire app is navigable
- No dead-end screens
- UI approved before logic begins

---

## Phase 2 — Local Agent Runtime (Rust Only)

### Goals

Implement the **local agent engine** without Telegram or backend.

### Components

- Rust agent runtime
- Intent router (question / sync / config)
- Processing pipeline (stubbed)
- Response composer (mock responses)

### Deliverables

- Tauri IPC commands:
  - `agent_init`
  - `agent_process_query`
  - `agent_status`

- In-memory only state

### Documentation Commands

```bash
/docs/core/agent.md          # agent architecture
/docs/core/events.md         # internal event types
/docs/core/state.md          # memory state model
```

### Exit Criteria

- UI can send a question
- Agent returns a mock response
- No persistence yet

---

## Phase 3 — Local Storage & Privacy Layer

### Goals

Add **efficient, privacy-first local storage**.

### Storage Rules

- No Telegram message bodies by default
- Store only:
  - channel IDs
  - last processed message IDs
  - dedupe hashes

- Encrypted at rest

### Deliverables

- Encrypted SQLite
- OS keychain integration
- Storage abstraction in Rust
- “Ephemeral mode” toggle

### Documentation Commands

```bash
/docs/storage/schema.md
/docs/storage/encryption.md
/docs/privacy/model.md
```

### Exit Criteria

- App restarts without losing cursors
- “Delete local data” wipes all state
- No plaintext sensitive data on disk

---

## Phase 4 — Telegram Bot Integration (Agent Assistant)

### Goals

Turn the app into a **real Telegram agent assistant**.

### Telegram Capabilities (MVP)

- Bot DM interaction
- Read user questions
- Fetch channel messages (where bot has access)
- Reply via DM

### Platform Behavior

- Windows/macOS/Android: continuous polling
- iOS: no polling (on-demand only)

### Deliverables

- Telegram Bot Gateway (Rust)
- Update polling / fetching
- Message dedupe + cursoring
- Agent replies sent via bot DM

### Documentation Commands

```bash
/docs/telegram/bot.md
/docs/telegram/flows.md
/docs/telegram/limits.md
```

### Exit Criteria

- User asks a question in Telegram
- App processes it
- Bot replies correctly
- No duplicate replies

---

## Phase 5 — Platform Background Execution

### Goals

Enable **platform-appropriate background behavior**.

### Platform Breakdown

#### Windows

- Tray app
- Autostart
- Background polling

#### macOS

- Menu bar app
- Launch at login

#### Android

- Foreground service (persistent notification)
- Background polling allowed

#### iOS

- ❌ No continuous background
- Only foreground execution

### Documentation Commands

```bash
/docs/platforms/windows.md
/docs/platforms/macos.md
/docs/platforms/android.md
/docs/platforms/ios.md
```

### Exit Criteria

- Desktop apps run without UI open
- Android foreground service stable
- iOS behaves strictly foreground-only

---

## Phase 6 — Minimal Backend Integration

### Goals

Introduce backend **without violating privacy goals**.

### Backend Responsibilities

- Authentication
- Device registration
- Entitlements
- Push notifications
- Payment verification

### Explicit Non-Responsibilities

- No Telegram message storage
- No agent logic
- No summaries

### Deliverables

- Auth flow wired into UI
- Entitlements fetched on startup
- Device registered for push

### Documentation Commands

```bash
/docs/backend/api.md
/docs/backend/data-model.md
/docs/backend/security.md
```

### Exit Criteria

- User login works
- Entitlements enforced locally
- Backend DB contains no message content

---

## Phase 7 — iOS Push → Tap → Sync Flow

### Goals

Implement the **iOS-specific agent interaction model**.

### Flow

1. User sends question to bot
2. Backend triggers visible push
3. User taps notification
4. App opens
5. App syncs Telegram
6. Agent processes
7. Bot replies

### Deliverables

- APNs integration
- Push payload handling
- Sync-on-open logic

### Documentation Commands

```bash
/docs/ios/push-flow.md
/docs/ios/limitations.md
```

### Exit Criteria

- Push reliably opens app
- Sync runs automatically
- Bot replies successfully

---

## Phase 8 — Payments & Plan Gating

### Goals

Monetize safely and correctly.

### Platforms

- Desktop: Stripe / Paddle
- Android: Play Billing
- iOS: StoreKit

### Deliverables

- Purchase flow per platform
- Receipt verification
- Feature gating in Rust

### Documentation Commands

```bash
/docs/billing/plans.md
/docs/billing/verification.md
/docs/billing/entitlements.md
```

### Exit Criteria

- Paid features unlock correctly
- Downgrades enforced
- Offline grace period handled

---

## Phase 9 — Hardening & Release Prep

### Goals

Stability, observability, and trust.

### Deliverables

- Error handling
- Rate limiting
- Abuse prevention
- Crash-safe storage
- UX polish

### Documentation Commands

```bash
/docs/release/checklist.md
/docs/known-issues.md
/docs/security/threat-model.md
```

### Exit Criteria

- No critical crashes
- No duplicate Telegram replies
- Clear user-facing error states

---

## Final Notes

- The **Telegram bot is the product interface**
- The **Tauri app is the execution engine**
- The **backend is infrastructure, not intelligence**
- iOS behavior is intentionally constrained for correctness

---

END OF MVP
