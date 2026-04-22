// OpenHuman webview-accounts recipe runtime.
// Injected via WebviewBuilder.initialization_script BEFORE page JS runs.
// Exposes a small `window.__openhumanRecipe` API per-provider recipes use
// to scrape the DOM and pipe state back to Rust.
//
// Runs in the loaded service's origin (e.g. https://mail.google.com).
// IPC back to Rust uses Tauri's `window.__TAURI_INTERNALS__.invoke`,
// which Tauri auto-injects into every webview it controls (including
// child webviews on external origins).
//
// Event kinds emitted to Rust via `webview_recipe_event`:
//   log        { level, msg }
//   ingest     { messages, unread?, snapshotKey? }      (recipe-driven)
//   <custom>   arbitrary — recipes push via api.emit(kind, payload)
//
// NOTE: only injected for providers that still need a JS bridge (gmail,
// linkedin, google-meet). The migrated providers (whatsapp, telegram,
// slack, discord, browserscan) load with ZERO injected JS under cef —
// their scraping runs natively via CDP in the per-provider scanner
// modules. WebSocket interception lives in the Rust-side CDP Network
// listener (see `discord_scanner/mod.rs`), not here.
//
// Browser push notifications are intercepted natively in the CEF render
// process by `cef-helper`'s NotifyV8Handler, which replaces
// window.Notification + ServiceWorkerRegistration.prototype.showNotification
// with V8 native bindings (see the tauri-cef fork).
(function () {
  if (window.__openhumanRecipe) return;

  const ctx = window.__OPENHUMAN_RECIPE_CTX__ || { accountId: 'unknown', provider: 'unknown' };
  const POLL_MS = 2000;

  function rawInvoke(cmd, payload) {
    try {
      const inv = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
      if (typeof inv !== 'function') return Promise.resolve();
      return inv(cmd, payload || {});
    } catch (e) {
      // swallow — never let a bad invoke break the host page
      return Promise.resolve();
    }
  }

  function send(kind, payload) {
    return rawInvoke('webview_recipe_event', {
      args: {
        account_id: ctx.accountId,
        provider: ctx.provider,
        kind: kind,
        payload: payload || {},
        ts: Date.now(),
      },
    });
  }

  let loopFn = null;
  let pollTimer = null;

  function safeRunLoop() {
    if (!loopFn) return;
    try {
      loopFn(api);
    } catch (e) {
      send('log', { level: 'warn', msg: '[recipe] loop threw: ' + (e && e.message ? e.message : String(e)) });
    }
  }

  const api = {
    loop(fn) {
      loopFn = fn;
      if (pollTimer) clearInterval(pollTimer);
      pollTimer = setInterval(safeRunLoop, POLL_MS);
      // also kick once on next tick so we don't wait POLL_MS for the first call
      setTimeout(safeRunLoop, 250);
      send('log', { level: 'info', msg: '[recipe] loop registered, polling every ' + POLL_MS + 'ms' });
    },
    ingest(payload) {
      // payload: { messages: Array<{id?, from?, body, ts?}>, unread?, snapshotKey? }
      send('ingest', payload || {});
    },
    log(level, msg) {
      send('log', { level: level || 'info', msg: String(msg) });
    },
    /** Push an arbitrary event kind up to Rust. Recipe-specific events
     *  (e.g. `meet_call_started`) go through here — the host side just
     *  sees another `webview:event` envelope with the given `kind`. */
    emit(kind, payload) {
      if (!kind) return;
      send(String(kind), payload || {});
    },
    context() {
      return Object.assign({}, ctx);
    },
  };

  window.__openhumanRecipe = api;
  send('log', { level: 'info', msg: '[recipe-runtime] ready provider=' + ctx.provider + ' accountId=' + ctx.accountId });
})();
