// OpenHuman webview-accounts recipe runtime.
// Injected via WebviewBuilder.initialization_script BEFORE page JS runs.
// Exposes a small `window.__openhumanRecipe` API per-provider recipes
// use to scrape DOM state and intercept WebSocket traffic.
//
// Runs in the loaded service's origin (e.g. https://web.whatsapp.com).
// IPC back to Rust uses Tauri's `window.__TAURI_INTERNALS__.invoke`,
// which Tauri auto-injects into every webview it controls (including
// child webviews on external origins).
//
// Event kinds emitted to Rust via `webview_recipe_event`:
//   log              { level, msg }
//   ingest           { messages, unread?, snapshotKey? }      (recipe-driven)
//   ws_message       { direction:'in'|'out', kind, data, url, size, ts }
//   ws_open          { url, ts }
//   ws_close         { url, code, reason, ts }
//   <custom>         arbitrary — recipes can push any kind via api.emit(kind, payload)
//
// Browser push notifications are NOT handled here — they're intercepted
// natively in the CEF render process by `cef-helper`'s NotifyV8Handler,
// which replaces window.Notification + ServiceWorkerRegistration.prototype
// .showNotification with V8 native bindings (see the tauri-cef fork).
// Composer autocomplete has been removed; any ghost-text overlay is now
// the UI host's responsibility.
(function () {
  if (window.__openhumanRecipe) return;

  const ctx = window.__OPENHUMAN_RECIPE_CTX__ || { accountId: 'unknown', provider: 'unknown' };
  const POLL_MS = 2000;

  // Cap the size of WS payloads we forward — Telegram / WhatsApp ship
  // large encrypted blobs we can't decode anyway, and dragging huge buffers
  // through Tauri IPC will stall the UI thread.
  const WS_MAX_FORWARD_BYTES = 16 * 1024;

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

  // ─── WebSocket interception ──────────────────────────────────────────
  // We patch `window.WebSocket` early (before the page boots) so we capture
  // every socket the provider opens. Emission to Rust is gated behind
  // `api.observeWebSocket()` so recipes can opt in only after they're sure
  // the chat UI is loaded — keeps noise (auth/handshake frames) down.
  let wsObserve = false;
  let wsFilter = null;
  const wsRegistry = new WeakSet();

  function classify(data) {
    if (typeof data === 'string') return 'text';
    if (data instanceof ArrayBuffer) return 'arraybuffer';
    if (typeof Blob !== 'undefined' && data instanceof Blob) return 'blob';
    if (data && typeof data.byteLength === 'number') return 'arraybufferview';
    return 'unknown';
  }

  function sizeOf(data) {
    if (typeof data === 'string') return data.length;
    if (data instanceof ArrayBuffer) return data.byteLength;
    if (typeof Blob !== 'undefined' && data instanceof Blob) return data.size;
    if (data && typeof data.byteLength === 'number') return data.byteLength;
    return 0;
  }

  function serializeForForward(data, kind) {
    if (kind === 'text') {
      const s = String(data);
      return s.length > WS_MAX_FORWARD_BYTES ? s.slice(0, WS_MAX_FORWARD_BYTES) : s;
    }
    // Binary frames — return null; recipes that care can decode in JS first
    // (provider-specific protobuf) and re-emit text via api.emitWebSocket().
    return null;
  }

  function shouldForward(frame) {
    if (!wsObserve) return false;
    if (typeof wsFilter !== 'function') return true;
    try { return !!wsFilter(frame); } catch (_) { return false; }
  }

  function forwardFrame(frame) {
    if (!shouldForward(frame)) return;
    send('ws_message', frame);
  }

  try {
    const NativeWS = window.WebSocket;
    if (NativeWS && !NativeWS.__openhumanPatched) {
      function PatchedWS(url, protocols) {
        const sock = protocols === undefined
          ? new NativeWS(url)
          : new NativeWS(url, protocols);
        wsRegistry.add(sock);
        try { send('ws_open', { url: String(url) }); } catch (_) {}

        const nativeSend = sock.send.bind(sock);
        sock.send = function (data) {
          try {
            const kind = classify(data);
            const size = sizeOf(data);
            forwardFrame({
              direction: 'out',
              kind: kind,
              data: serializeForForward(data, kind),
              url: String(url),
              size: size,
              ts: Date.now(),
            });
          } catch (_) {}
          return nativeSend(data);
        };

        sock.addEventListener('message', function (ev) {
          try {
            const kind = classify(ev.data);
            const size = sizeOf(ev.data);
            forwardFrame({
              direction: 'in',
              kind: kind,
              data: serializeForForward(ev.data, kind),
              url: String(url),
              size: size,
              ts: Date.now(),
            });
          } catch (_) {}
        });
        sock.addEventListener('close', function (ev) {
          try { send('ws_close', { url: String(url), code: ev.code, reason: ev.reason }); } catch (_) {}
        });
        return sock;
      }
      PatchedWS.prototype = NativeWS.prototype;
      PatchedWS.CONNECTING = NativeWS.CONNECTING;
      PatchedWS.OPEN = NativeWS.OPEN;
      PatchedWS.CLOSING = NativeWS.CLOSING;
      PatchedWS.CLOSED = NativeWS.CLOSED;
      PatchedWS.__openhumanPatched = true;
      window.WebSocket = PatchedWS;
    }
  } catch (_) {
    // WebSocket missing — fine, nothing to patch.
  }

  // ─── Public API ───────────────────────────────────────────────────────
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
     *  (e.g. `meet_call_started`, `slack_thread_open`) go through here —
     *  the host side just sees another `webview:event` envelope with
     *  the given `kind`. No-ops if `kind` is falsy. */
    emit(kind, payload) {
      if (!kind) return;
      send(String(kind), payload || {});
    },
    context() {
      return Object.assign({}, ctx);
    },

    // WebSocket
    observeWebSocket(opts) {
      opts = opts || {};
      wsObserve = true;
      wsFilter = typeof opts.filter === 'function' ? opts.filter : null;
      send('log', { level: 'info', msg: '[recipe] websocket observation enabled' });
    },
    stopObservingWebSocket() {
      wsObserve = false;
      wsFilter = null;
    },
    /** Manually emit a normalized ws frame after recipe-side decoding. */
    emitWebSocket(frame) {
      if (!frame) return;
      send('ws_message', Object.assign({ direction: 'in', kind: 'text', ts: Date.now() }, frame));
    },

    // Escape hatch — used by Rust when it wants to run arbitrary recipe
    // helpers without round-tripping through a typed command.
    runScript(js) {
      try { return (new Function(js))(); } catch (e) {
        send('log', { level: 'error', msg: '[recipe] runScript threw: ' + (e && e.message ? e.message : String(e)) });
        return null;
      }
    },
  };

  window.__openhumanRecipe = api;
  send('log', { level: 'info', msg: '[recipe-runtime] ready provider=' + ctx.provider + ' accountId=' + ctx.accountId });
})();
