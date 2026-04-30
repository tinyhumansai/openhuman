# Webview integration playbook

How to add a new third-party webview integration (WhatsApp-style: Instagram,
Messenger, LinkedIn messages, …) to the app — from zero to messages in memory.

The WhatsApp scanner (`app/src-tauri/src/whatsapp_scanner/`) is the reference
implementation. This doc captures the workflow we used to build it so future
integrations can follow the same pattern and debug the same way.

---

## The architecture in one picture

```
  CEF webview (third-party site)
     │
     │  ① Chrome DevTools Protocol (ws://127.0.0.1:9222)
     ▼
  Rust scanner (per-account poller)
     │
     │  ② `Runtime.evaluate` → scanner JS → returns JSON snapshot
     │
     ├─ ③ emit `webview:event` Tauri event (React UI consumes live)
     │
     └─ ④ POST `openhuman.memory_doc_ingest` JSON-RPC → core
                │
                ▼
           TinyHumans Neocortex memory (chunked + embedded + graph)
```

Step ① relies on CEF's remote debugging port (CEF is the only supported
runtime for this app).

---

## The workflow — step by step

### 0. Pre-work: find stable DOM / IDB hooks

Open the site in the CEF webview (any build with `pnpm dev:cef`), then open
DevTools at <http://localhost:9222>. Look for:

- **Stable selectors** — `data-*` attributes, `role`, `aria-label`. CSS
  classes are often randomised per build (WhatsApp, Facebook). Don't trust
  them.
- **Stable structure** — `[data-id]` on each message row, a container with
  `#main` / `[role="application"]`, etc.
- **IndexedDB stores** — `application → IndexedDB` in DevTools. Note the
  DB names and store names. Look for plaintext metadata (ids, timestamps,
  chat names) vs. encrypted blobs (bodies).

Write these down. You will reference them constantly.

### 1. Scaffold the Rust scanner

Copy `app/src-tauri/src/whatsapp_scanner/` to
`app/src-tauri/src/<provider>_scanner/`:

- `mod.rs` — dual-tick poller, CDP client, ingest emit, memory RPC
- `scanner.js` — full IDB walk (runs every 30s via `Runtime.evaluate`)
- `dom_scan.js` — cheap DOM scrape (runs every 2s, hashed for dedupe)

Wire it up in `lib.rs`:

```rust
mod instagram_scanner;
// …
let builder = builder.manage(instagram_scanner::ScannerRegistry::new());
```

And in `webview_accounts/mod.rs`, spawn the scanner when the account opens:

```rust
if args.provider == "instagram" {
    if let Some(prefix) = provider_url(&args.provider) {
        let registry = app
            .try_state::<std::sync::Arc<crate::instagram_scanner::ScannerRegistry>>()
            .map(|s| s.inner().clone());
        if let Some(registry) = registry {
            let app_clone = app.clone();
            let acct = args.account_id.clone();
            tokio::spawn(async move {
                registry.ensure_scanner(app_clone, acct, prefix.to_string()).await;
            });
        }
    }
}
```

### 2. Iterate fast with the dev-auto flag

Add a dev-only env-var-triggered auto-open so every rebuild lands you
immediately in the webview you're debugging, pre-authed with the profile
you've already logged into:

```rust
// app/src-tauri/src/lib.rs
if let Ok(account_id) = std::env::var("OPENHUMAN_DEV_AUTO_INSTAGRAM") {
    // schedule webview_account_open with provider="instagram" …
}
```

Then during development:

```bash
OPENHUMAN_DEV_AUTO_INSTAGRAM=<account-uuid> pnpm dev:cef >/tmp/oh-cef.log 2>&1 &
```

The account UUID is stable across runs so the webview profile persists
(cookies, QR sign-in, etc). No re-auth per iteration.

### 3. Monitor logs, not the UI

Every interesting scanner event goes through `log::info!`. Use `Monitor`
(or `tail -F … | grep -E …`) to watch only the events you care about:

```
tail -F /tmp/oh-cef.log | grep -E --line-buffered \
  "scan ok|fast dom-scan rows=[1-9]|memory upsert ok|memory write failed|error\[E"
```

Tag every log line consistently:

```rust
log::info!("[wa][{}] scan ok dbs={}", account_id, snap.dbs.len());
log::info!("[wa][{}] memory upsert ok key={} msgs={}", account_id, key, n);
```

Grep-friendly prefixes (`[wa]`, `[ig]`, …) keep cross-integration logs
readable when multiple scanners run at once.

### 4. Fast tick vs full tick

Two interleaved loops — do **not** run everything on one cadence:

| Tick | Interval | Job                                                          | Cost                         |
| ---- | -------- | ------------------------------------------------------------ | ---------------------------- |
| Fast | 2s       | `dom_scan.js` — `[data-id]` rows, body text from visible DOM | ~30 `querySelectorAll`, tiny |
| Full | 30s      | `scanner.js` — all IDB stores, chat names, contact names     | 12 DBs × ~500 rows, heavy    |

Cache a hash of the fast-tick output and emit only when it changes:

```rust
let changed = last_dom_hash != Some(dom.hash) && !dom.dom_messages.is_empty();
if changed {
    emit_dom_only(&app, &account_id, &dom.dom_messages);
    last_dom_hash = Some(dom.hash);
}
```

Idle = silent. Active = live updates every 2s.

### 5. Group by (chatId, day) → stable memory doc

Memory docs should upsert cleanly on re-scan. The key scheme that works:

- **namespace**: `<provider>-web:<account_id>`
- **key**: `<chatId>:<YYYY-MM-DD>`

So every day of every conversation is one document. Re-emit freely: same
day's scroll just upserts the existing doc.

Group messages in Rust:

```rust
let mut groups: HashMap<(String, String), Vec<Value>> = HashMap::new();
for m in messages {
    let chat_id = m.get("chatId")?.as_str()?.to_string();
    let day = seconds_to_ymd(m.get("timestamp")?.as_i64()?);
    groups.entry((chat_id, day)).or_default().push(normalize(m));
}
```

### 6. Direct core-RPC (don't rely on the React listener)

The React-side `webview:event` listener only runs when the main window is
open on `/accounts`. For background ingest to always work, POST directly
from the scanner:

```rust
let url = std::env::var("OPENHUMAN_CORE_RPC_URL")
    .unwrap_or_else(|_| "http://127.0.0.1:7788/rpc".into());
let body = json!({
    "jsonrpc": "2.0", "id": 1,
    "method": "openhuman.memory_doc_ingest",
    "params": { namespace, key, title, content, source_type, tags, metadata, category },
});
reqwest::Client::new().post(&url).json(&body).send().await?;
```

Fire-and-forget with `tokio::spawn` so the scanner tick doesn't block.

### 7. DOM + IDB merge

IDB gives you metadata for every message ever (ids, timestamps, chatIds)
but bodies may be encrypted at rest. DOM gives you rendered text but only
for the chat the user has open. Merge them by matching a stable identifier
(WhatsApp: the full `data-id` matches IDB's `_serialized` message id):

```rust
// by_msg_id keyed by BOTH full data-id and bare msgId → consumed-set
// deduped by source dataId so a row gets patched OR appended, never both.
```

Result: every IDB message row keeps its metadata, and its body appears
as soon as the user scrolls past it.

### 8. Resolve display names via a contact cache

Raw JIDs (`96550986580@c.us`) are useless in transcripts. The full IDB
scan already walks `model-storage/contact`, `/chat`, `/group-metadata`
and builds a `jid → name` map. Cache it per-account so the fast tick can
resolve names too:

```rust
fn contact_cache_put(account_id: &str, names: &serde_json::Map<String, Value>);
fn contact_cache_get(account_id: &str) -> serde_json::Map<String, Value>;
```

### 9. Minimise disruption to the user

The WhatsApp build started with a CSP bypass + `Page.reload` to install a
worker-constructor hook. It worked but **reloaded the page every startup**
— very disruptive. When we pivoted to pure DOM scraping we deleted:

- `Page.setBypassCSP`
- `Page.addScriptToEvaluateOnNewDocument`
- the forced `Page.reload`
- per-worker `Target.attachToTarget` probes (CEF workers don't answer
  Runtime commands anyway — each probe wasted ~10s)

Rule of thumb: **only reload the page if you genuinely can't avoid it**.
DOM scraping via `Runtime.evaluate` needs none of the above.

---

## Common pitfalls we hit

| Problem                                 | Cause                                                        | Fix                                                      |
| --------------------------------------- | ------------------------------------------------------------ | -------------------------------------------------------- |
| Scanner never runs                      | App not started via the CEF dev script                       | Use `pnpm dev:cef`                                       |
| `helloCount: 0` despite wrapper running | CSP blocks `blob:` workers                                   | Skip the workers entirely — use DOM scrape               |
| 10s-per-worker timeouts                 | CEF worker targets don't answer CDP Runtime calls            | Don't probe workers                                      |
| `patched=N, appended=N` duplicates      | DOM row indexed twice, only one key removed on patch         | Track consumed rows by a stable `dataId`, not by map key |
| Empty `dom-scrape count` after reload   | Chat not open yet / page still loading                       | Scanner polls — next tick picks it up                    |
| Memory writes never fire                | React listener not attached (user not on `/accounts`)        | POST to core RPC directly from Rust                      |
| Stale timestamps in transcript          | `preTimestamp` from DOM is string like `"4:53 AM, 7/5/2025"` | Parse it; fall back to `now_secs` if unparseable         |
| Raw JIDs in transcript                  | No contact lookup                                            | Populate `contact_cache` from full-scan chats map        |

---

## Reference files (WhatsApp)

- `app/src-tauri/src/whatsapp_scanner/mod.rs` — scanner entry, CDP client,
  ingest emit, memory RPC, dual-tick loop
- `app/src-tauri/src/whatsapp_scanner/scanner.js` — full IDB walk
- `app/src-tauri/src/whatsapp_scanner/dom_scan.js` — fast DOM scrape
- `app/src-tauri/src/webview_accounts/mod.rs` — scanner spawn on account open
- `app/src/services/webviewAccountService.ts` — React-side `webview:event`
  listener (optional; direct core-RPC also works)
- `src/openhuman/memory/schemas.rs` — `openhuman.memory_doc_ingest` RPC
  (the endpoint your scanner posts to)

---

## Checklist for a new integration

- [ ] DOM stable selectors identified (prefer `data-*`, `role`, `aria-*`)
- [ ] IDB stores mapped (db + store names, which hold what)
- [ ] Scanner module scaffolded (`<provider>_scanner/`)
- [ ] `lib.rs`: module declared, `ScannerRegistry` managed
- [ ] `webview_accounts/mod.rs`: scanner spawn on `provider == "<provider>"`
- [ ] Log prefix chosen (`[wa]`, `[ig]`, `[fb]`, …)
- [ ] Dev-auto env var wired (`OPENHUMAN_DEV_AUTO_<PROVIDER>`)
- [ ] Fast tick (2s) + full tick (30s) both wired
- [ ] Messages grouped by `(chatId, day)` → memory namespace + key
- [ ] Contact/chat name resolution (direct JID → display-name cache)
- [ ] Direct core-RPC `openhuman.memory_doc_ingest` POST (not React-only)
- [ ] Logs tagged + monitor-friendly
- [ ] No forced page reloads, no CSP bypass unless absolutely needed
