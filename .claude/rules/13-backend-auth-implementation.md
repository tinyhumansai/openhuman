# Backend Authentication Implementation Guide

## Overview

The login web-to-desktop flow uses deep links to hand off authentication from a web browser to the Tauri desktop app. The frontend and Rust-side token exchange are implemented. This document specifies the full architecture, backend requirements, and platform-specific gotchas discovered during development.

## Architecture

```
Web Browser                    Backend Server                 Desktop App (Tauri)
    │                              │                              │
    │  1. User clicks login        │                              │
    │─────────────────────────────>│                              │
    │                              │                              │
    │  2. Auth flow (Telegram/     │                              │
    │     Phone OTP)               │                              │
    │<────────────────────────────>│                              │
    │                              │                              │
    │  3. POST /api/auth/          │                              │
    │     web-complete             │                              │
    │─────────────────────────────>│                              │
    │                              │                              │
    │  4. Returns loginToken       │                              │
    │<─────────────────────────────│                              │
    │                              │                              │
    │  5. Redirect to              │                              │
    │     outsourced://auth?token= │                              │
    │─────────────────────────────────────────────────────────────>│
    │                              │                              │
    │                              │  6. Rust invoke               │
    │                              │     exchange_token             │
    │                              │  (POST /auth/desktop-exchange) │
    │                              │<─────────────────────────────│
    │                              │                              │
    │                              │  7. Returns sessionToken     │
    │                              │     + user object             │
    │                              │─────────────────────────────>│
    │                              │                              │
    │                              │     8. App stores session,   │
    │                              │        navigates to onboarding│
```

**Key**: Step 6 uses a **Rust Tauri command** (`exchange_token`) via `invoke()` instead of browser `fetch()`. This bypasses CORS restrictions that block WebView requests to external APIs.

## Required Endpoints

### 1. `GET /auth/telegram`

Initiates Telegram OAuth. The frontend opens this URL in the system browser.

**Query params:**
- `platform=desktop` — indicates the callback should produce a deep link handoff

**Behavior:**
1. Redirect user to Telegram OAuth authorization URL
2. On callback, validate Telegram user data
3. Create or find user in database
4. Generate a short-lived `loginToken` (single-use, 5-minute TTL)
5. Redirect to `outsourced://auth?token=<loginToken>`

### 2. `POST /api/auth/web-complete`

Called by the web frontend after phone-based authentication completes.

**Request body:**
```json
{
  "method": "phone",
  "phoneNumber": "+1234567890",
  "countryCode": "+1"
}
```
or
```json
{
  "method": "telegram",
  "telegramUser": { /* Telegram user object */ }
}
```

**Response (200):**
```json
{
  "loginToken": "short-lived-opaque-token"
}
```

**Behavior:**
1. Validate the authentication data (verify OTP for phone, verify Telegram hash)
2. Create or find user in database
3. Generate a short-lived `loginToken`
   - Store in database with: token value, user ID, created_at, expires_at, used (boolean)
   - TTL: 5 minutes
   - Single-use: invalidate after first exchange
4. Return the token

### 3. `POST /auth/desktop-exchange`

Called by the Tauri Rust command `exchange_token` (NOT browser fetch). Exchanges the short-lived handoff token for a long-lived session.

**Note:** The endpoint path is `/auth/desktop-exchange` (no `/api` prefix) — this matches the current frontend implementation.

**Request body:**
```json
{
  "token": "loginToken-from-web"
}
```

**Response (200):**
```json
{
  "sessionToken": "long-lived-session-token",
  "user": {
    "id": "uuid",
    "username": "string",
    "firstName": "string"
  }
}
```

**Error response (401):**
```json
{
  "success": false,
  "error": "Token expired or invalid"
}
```

**Behavior:**
1. Look up the `loginToken` in the database
2. Validate: not expired, not already used
3. Mark token as used (single-use enforcement)
4. Generate a long-lived `sessionToken` (e.g., 30-day TTL)
5. Return session token and user profile

## Database Schema

### `users` table
| Column | Type | Description |
|--------|------|-------------|
| id | UUID (PK) | User ID |
| username | TEXT | Display name |
| first_name | TEXT (nullable) | First name |
| phone_number | TEXT (nullable) | Phone number |
| telegram_id | TEXT (nullable) | Telegram user ID |
| created_at | TIMESTAMP | Account creation |
| updated_at | TIMESTAMP | Last update |

### `login_tokens` table (handoff tokens)
| Column | Type | Description |
|--------|------|-------------|
| id | UUID (PK) | Token record ID |
| token | TEXT (unique, indexed) | The opaque token string |
| user_id | UUID (FK -> users) | Associated user |
| created_at | TIMESTAMP | When issued |
| expires_at | TIMESTAMP | Expiration (created_at + 5min) |
| used | BOOLEAN | Whether already exchanged |

### `sessions` table
| Column | Type | Description |
|--------|------|-------------|
| id | UUID (PK) | Session ID |
| token | TEXT (unique, indexed) | Session token |
| user_id | UUID (FK -> users) | Associated user |
| created_at | TIMESTAMP | When issued |
| expires_at | TIMESTAMP | Expiration (created_at + 30 days) |
| revoked | BOOLEAN | Whether revoked |

## Token Generation

- Use cryptographically secure random bytes (32+ bytes, base64url-encoded)
- `loginToken`: short-lived (5 min), single-use, opaque
- `sessionToken`: long-lived (30 days), opaque, revocable

## Security Requirements

1. **Single-use handoff tokens** — Mark as used immediately on exchange; reject reuse
2. **Short TTL on handoff tokens** — 5 minutes maximum
3. **HTTPS only** in production — tokens travel as URL parameters and POST bodies
4. **Rate limiting** — on `/api/auth/web-complete` and `/auth/desktop-exchange`
5. **Token entropy** — minimum 256 bits of randomness
6. **Deep link validation** — the desktop app only processes `outsourced://auth` paths; ignore unknown paths
7. **Telegram data verification** — validate the `hash` field using your bot token per Telegram docs

## Implementation Details

### Rust Token Exchange Command

The desktop app calls the backend via a Rust Tauri command (`exchange_token` in `src-tauri/src/lib.rs`) using `reqwest`. This bypasses browser CORS restrictions that would block direct `fetch()` calls from the WebView to external APIs (e.g., ngrok tunnels).

```rust
#[tauri::command]
async fn exchange_token(backend_url: String, token: String) -> Result<serde_json::Value, String>
```

Frontend invocation:
```typescript
const data = await invoke<{ sessionToken?: string; user?: object }>(
  'exchange_token',
  { backendUrl: BACKEND_URL, token }
);
```

### Deep Link Listener

Located in `src/utils/desktopDeepLinkListener.ts`. Key behaviors:
- Uses **lazy dynamic import** in `main.tsx` to avoid loading before Tauri IPC is ready
- No `window.__TAURI__` guard — the try/catch handles non-Tauri environments
- Uses `localStorage.setItem('deepLinkHandled', 'true')` to prevent infinite reload loops (since `getCurrent()` returns the same URL after `window.location.replace()`)
- Clears the `deepLinkHandled` flag on next startup so future deep links work

### Backend URL Configuration

Configured in `src/utils/config.ts`:
```typescript
export const BACKEND_URL =
  import.meta.env.VITE_BACKEND_URL || 'https://2937933edf8a.ngrok-free.app';
```

Set `VITE_BACKEND_URL` environment variable for different environments.

## Frontend Integration Points

| File | Role |
|------|------|
| `src/main.tsx` | Lazy-imports and starts the deep link listener |
| `src/utils/desktopDeepLinkListener.ts` | Parses deep link -> invokes Rust `exchange_token` -> stores session -> navigates |
| `src/utils/deeplink.ts` | Web-side: calls `/api/auth/web-complete` and builds `outsourced://auth?token=...` URL |
| `src/utils/config.ts` | Backend URL configuration |
| `src/pages/Login.tsx` | Opens `GET /auth/telegram?platform=desktop` in browser |
| `src-tauri/src/lib.rs` | Rust `exchange_token` command using `reqwest` (CORS-free) |

## Phone OTP Flow (Future)

The frontend has a phone input UI but the OTP verification flow needs:

1. `POST /api/auth/send-otp` — sends SMS to phone number
2. `POST /api/auth/verify-otp` — verifies code, returns success
3. Then `POST /api/auth/web-complete` with `method: "phone"` to get the handoff token

## Platform-Specific Notes

See `14-deep-link-platform-guide.md` for detailed platform gotchas.

---

*Last updated: 2026-01-28*
