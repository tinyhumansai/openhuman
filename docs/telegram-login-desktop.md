# Telegram Login (Web → Desktop Handoff)

This app implements **Telegram login for the desktop (Tauri) client** using a **system-browser auth flow** plus a **custom URL scheme deep link** (`openhuman://`) to return control back to the desktop app.

---

## High-level flow

1. **User clicks “Continue with Telegram”** inside the desktop app.
2. The app opens the system browser to the backend:
   - `GET ${BACKEND_URL}/auth/telegram-widget?redirect=openhuman://auth`
3. The backend performs Telegram authentication (bot-based login / OAuth-like flow).
4. On success, the backend generates a **short-lived single-use `loginToken`** and redirects the browser to:
   - `openhuman://auth?token=<loginToken>`
5. The OS routes that deep link to the installed desktop app.
6. The desktop app extracts the `token` from the deep link and exchanges it for a **long-lived `sessionToken`** by calling a Rust Tauri command (bypassing CORS):
   - `POST ${BACKEND_URL}/auth/desktop-exchange` with `{ token }`
7. The desktop app stores `sessionToken` (and optional `user`) and navigates into onboarding.

---

## Where this is implemented (current code)

### 1) Desktop UI entry point

The Telegram button in `src/components/TelegramLoginButton.tsx` opens the backend URL in the user’s system browser (via a Tauri command on desktop):

```ts
await startTelegramLoginWithUrl(BACKEND_URL);
```

The backend base URL is configured here (`src/utils/config.ts`):

```ts
export const BACKEND_URL =
  import.meta.env.VITE_BACKEND_URL || 'https://2937933edf8a.ngrok-free.app';
```

### 2) Deep link registration (Tauri config + Rust)

The URL scheme is declared in `src-tauri/tauri.conf.json`:

```json
{ "plugins": { "deep-link": { "desktop": { "schemes": ["openhuman"] } } } }
```

The deep-link plugin is initialized in `src-tauri/src/lib.rs`:

```rust
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            #[cfg(any(windows, target_os = "linux"))]
            {
                app.deep_link().register_all()?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![greet, exchange_token])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

### 3) Desktop deep link listener (frontend)

The listener is lazy-loaded in `src/main.tsx`:

```ts
import('./utils/desktopDeepLinkListener').then(m => {
  m.setupDesktopDeepLinkListener().catch(err => {
    console.error('[DeepLink] setup error:', err);
  });
});
```

The deep link handler:

- Accepts only the `openhuman:` scheme
- Requires `openhuman://auth?token=...`
- Calls the Rust command `exchange_token`
- Stores `sessionToken` in Redux auth state
- Redirects to `#/onboarding` (HashRouter)

```ts
const handleDeepLinkUrls = async (urls: string[] | null | undefined) => {
  if (!urls || urls.length === 0) return;
  const url = urls[0];

  try {
    const parsed = new URL(url);
    if (parsed.protocol !== 'openhuman:') return;
    if (parsed.hostname !== 'auth') return;

    const token = parsed.searchParams.get("token");
    if (!token) return;

    const data = await invoke("exchange_token", {
      backendUrl: BACKEND_URL,
      token,
    });
    // ... store sessionToken + user ...
    window.location.hash = "/onboarding";
  } catch (error) {
    console.error("[DeepLink] Failed to handle deep link URL:", url, error);
  }
};
```

### 4) Token exchange happens in Rust (CORS-safe)

The command `exchange_token` posts to the backend and returns the JSON body:

```rust
#[tauri::command]
async fn exchange_token(backend_url: String, token: String) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/auth/desktop-exchange", backend_url);
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({ "token": token }))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;
    // ... status handling ...
    Ok(body)
}
```

---

## Backend contract (required for Telegram login to work)

Your backend must implement **both**:

### A) `GET /auth/telegram-widget?redirect=openhuman://auth`

- **Purpose**: start Telegram auth in the user’s browser.
- **On success**:
  - create/find user
  - mint a **short-lived** `loginToken` (single-use, recommended TTL \(\le 5\) minutes)
  - redirect to: `openhuman://auth?token=<loginToken>`

### B) `POST /auth/desktop-exchange`

- **Purpose**: exchange `loginToken` for a long-lived desktop session token.
- **Request**:

```json
{ "token": "loginToken-from-deeplink" }
```

- **Response (200)**:

```json
{
  "sessionToken": "long-lived-session-token",
  "user": { "id": "uuid", "username": "string", "firstName": "string" }
}
```

---

## Platform notes (important for “it works on my machine” issues)

- **macOS**: deep links do **not** work reliably in `tauri dev` because there’s no `.app` bundle/`Info.plist`. You generally need a built `.app` bundle (debug build is fine).
- **Windows/Linux**: deep link scheme registration is done at runtime via `register_all()` and works in dev more easily.

---

## “Required code changes” checklist (to make Telegram login work properly)

### Backend (required)

- **Implement `GET /auth/telegram?platform=desktop`** and ensure it redirects to `openhuman://auth?token=...` on success.
- **Implement `POST /auth/desktop-exchange`** to exchange the login token for a session token.
- **Enforce security**:
  - `loginToken` is **single-use**
  - short TTL (recommended \(\le 5\) minutes)
  - reject expired/reused tokens

### Desktop app (required for reliable behavior)

- **Ensure the deep-link plugin is configured and permitted**:
  - `src-tauri/tauri.conf.json` includes `"schemes": ["openhuman"]`
  - `src-tauri/capabilities/default.json` includes `"deep-link:default"`
- **Use a real backend URL**:
  - set `VITE_BACKEND_URL` for dev/prod so `Login.tsx` opens the correct domain

### Desktop app (recommended hardening / cleanup)

- **Validate the deep link target more strictly** in `src/utils/desktopDeepLinkListener.ts`:
  - today it checks only `parsed.protocol === 'openhuman:'`
  - recommended: also require `parsed.hostname === 'auth'` (and optionally a known path)
- **Don’t skip Telegram auth in onboarding**:
  - `src/pages/onboarding/Step1Phone.tsx` currently has a “Continue with Telegram” button that _only navigates_ and does not authenticate.
- **Remove sensitive Telegram secrets from frontend env**:
  - any `VITE_*` variables are bundled into the frontend; don’t place bot tokens / api hashes there.
  - the desktop app typically only needs `VITE_BACKEND_URL`; Telegram verification secrets should live on the backend.
