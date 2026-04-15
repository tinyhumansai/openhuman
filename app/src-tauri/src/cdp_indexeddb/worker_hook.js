// Page-level Worker constructor wrapper. Installed via
// `Page.addScriptToEvaluateOnNewDocument` so it runs BEFORE WA's
// own scripts — that way every worker WA later spawns gets our spy
// transparently injected into its source.
//
// Mechanism:
//   * We override `window.Worker`. When WA does `new Worker(url, opts)`,
//     we synchronously fetch that script (workers must be same-origin,
//     so a fetch is allowed), prepend our spy code, wrap in a
//     `URL.createObjectURL(new Blob(...))`, and pass the blob URL to
//     the original constructor.
//   * Inside the worker, the spy hooks `crypto.subtle.{deriveKey,decrypt}`
//     and stashes captured calls on `globalThis.__ohWorkerSpy`. It also
//     proactively replies to a custom `postMessage({__ohSpy:'dump'})`
//     by posting back the captured tuples.
//   * Back in the page, we keep references to every wrapped Worker so
//     scanner.js (running in main page) can iterate them, ask each for
//     a spy dump, and aggregate the (info, salt) tuples.
(() => {
  if (typeof window === 'undefined') return;
  if (window.__ohWorkerHookInstalled) return;
  window.__ohWorkerHookInstalled = true;
  const Orig = window.Worker;
  if (!Orig) return;

  // Spy code that runs INSIDE the worker. Same instrumentation as
  // worker_spy.js but plus a postMessage reply path.
  // The `/*__FORCE_EXTRACTABLE__*/` sentinel is replaced at install
  // time (Rust side, in cdp_indexeddb/mod.rs) with the source of
  // force_extractable.js so every worker also gets the extractable
  // override before WA's own crypto calls run.
  const SPY_SRC = `
    /*__FORCE_EXTRACTABLE__*/
    (() => {
      const g = (typeof globalThis !== 'undefined' ? globalThis : self);
      if (g.__ohWorkerSpy) return;
      const cap = { derive: [], decrypt: [], decryptOk: [], importedScripts: [] };
      g.__ohWorkerSpy = cap;
      // Beacon hello FIRST — before any subtle binding which may throw.
      try { g.postMessage({ __ohSpyHello: true, ts: Date.now(), where: 'init', loc: (g.location && g.location.href) || null }); } catch (_) {}
      // Hook importScripts so any child scripts the worker loads get our spy
      // too (WhatsApp's init shim importScripts() the real WA worker bundle,
      // and that's where the actual crypto happens — wrapping new Worker()
      // alone misses it).
      try {
        const origImport = g.importScripts && g.importScripts.bind(g);
        if (origImport) {
          g.importScripts = function (...urls) {
            try { for (const u of urls) cap.importedScripts.push(String(u).slice(0, 200)); } catch (_) {}
            try { g.postMessage({ __ohSpyImport: true, urls: urls.map((u) => String(u).slice(0, 200)) }); } catch (_) {}
            return origImport(...urls);
          };
        }
      } catch (_) {}
      // crypto.subtle may not exist on every worker context (e.g. init
      // shims). Guard so a missing subtle doesn't kill the whole worker.
      let orig;
      try {
        orig = {
          deriveKey: g.crypto.subtle.deriveKey.bind(g.crypto.subtle),
          deriveBits: g.crypto.subtle.deriveBits.bind(g.crypto.subtle),
          decrypt: g.crypto.subtle.decrypt.bind(g.crypto.subtle),
        };
      } catch (e) {
        try { g.postMessage({ __ohSpyHello: true, where: 'no-subtle', err: String(e && e.message || e) }); } catch (_) {}
        // Still install reply handler below so we can be pinged for diag.
        orig = null;
      }
      if (orig) {
      const summarizeBytes = (b) => {
        if (b == null) return null;
        let bytes;
        if (b instanceof Uint8Array) bytes = b;
        else if (b instanceof ArrayBuffer) bytes = new Uint8Array(b);
        else if (ArrayBuffer.isView(b)) bytes = new Uint8Array(b.buffer, b.byteOffset, b.byteLength);
        else return null;
        let hex = '';
        for (let i = 0; i < Math.min(bytes.length, 128); i += 1) hex += bytes[i].toString(16).padStart(2, '0');
        let utf8 = '';
        try { utf8 = new TextDecoder('utf-8', { fatal: false }).decode(bytes.subarray(0, Math.min(bytes.length, 128))); } catch (_) {}
        return { len: bytes.length, hex, utf8 };
      };
      g.crypto.subtle.deriveKey = function (params, baseKey, derivedKeyType, extractable, usages) {
        try {
          cap.derive.push({
            fn: 'deriveKey',
            algName: params && params.name,
            algHash: params && params.hash,
            info: params ? summarizeBytes(params.info) : null,
            salt: params ? summarizeBytes(params.salt) : null,
            ts: Date.now(),
          });
          if (cap.derive.length > 200) cap.derive.shift();
        } catch (_) {}
        return orig.deriveKey(params, baseKey, derivedKeyType, extractable, usages);
      };
      g.crypto.subtle.deriveBits = function (params, baseKey, length) {
        try {
          cap.derive.push({
            fn: 'deriveBits',
            algName: params && params.name,
            algHash: params && params.hash,
            info: params ? summarizeBytes(params.info) : null,
            salt: params ? summarizeBytes(params.salt) : null,
            ts: Date.now(),
          });
          if (cap.derive.length > 200) cap.derive.shift();
        } catch (_) {}
        return orig.deriveBits(params, baseKey, length);
      };
      g.crypto.subtle.decrypt = function (params, key, data) {
        const evt = {
          algName: params && params.name,
          ivLen: params && params.iv ? (params.iv.byteLength || params.iv.length) : null,
          dataLen: data && (data.byteLength || data.length),
          ts: Date.now(),
        };
        cap.decrypt.push(evt);
        if (cap.decrypt.length > 200) cap.decrypt.shift();
        const p = orig.decrypt(params, key, data);
        p.then((buf) => {
          cap.decryptOk.push({ ...evt, plainLen: buf && (buf.byteLength || buf.length) });
          if (cap.decryptOk.length > 200) cap.decryptOk.shift();
        }, () => {});
        return p;
      };
      } // end if (orig)
      // Reply protocol — the page can ping us with {__ohSpy:'dump'} to
      // get the latest snapshot.
      g.addEventListener('message', (ev) => {
        if (!ev || !ev.data || ev.data.__ohSpy !== 'dump') return;
        const distinct = new Map();
        for (const d of cap.derive) {
          if (!d || d.algName !== 'HKDF') continue;
          const tag = (d.info && d.info.hex) + '|' + (d.salt && d.salt.hex);
          if (!distinct.has(tag)) distinct.set(tag, d);
        }
        const decryptSizes = {};
        for (const e of cap.decrypt) {
          if (e.algName !== 'AES-GCM') continue;
          decryptSizes[e.dataLen] = (decryptSizes[e.dataLen] || 0) + 1;
        }
        const decryptOkSizes = {};
        for (const e of cap.decryptOk) {
          if (e.algName !== 'AES-GCM') continue;
          const tag = e.dataLen + '->' + e.plainLen;
          decryptOkSizes[tag] = (decryptOkSizes[tag] || 0) + 1;
        }
        try {
          g.postMessage({
            __ohSpyReply: true,
            ts: Date.now(),
            loc: (g.location && g.location.href) || null,
            hasSubtle: !!orig,
            deriveCount: cap.derive.length,
            decryptCount: cap.decrypt.length,
            decryptOkCount: cap.decryptOk.length,
            distinctDeriveCount: distinct.size,
            distinctDerives: Array.from(distinct.values()),
            decryptSizes,
            decryptOkSizes,
            importedScripts: cap.importedScripts.slice(-8),
          });
        } catch (_) {}
      });
    })();
  `;

  const wrappedWorkers = new Set();
  window.__ohWrappedWorkers = wrappedWorkers;
  // Per-worker liveness: true if it ever sent us {__ohSpyHello:true}.
  window.__ohWorkerHello = new WeakSet();
  window.__ohWorkerHelloCount = 0;
  // Diagnostic ring buffer for every Worker construction.
  window.__ohWorkerDiag = [];
  function diag(rec) {
    try { window.__ohWorkerDiag.push(rec); if (window.__ohWorkerDiag.length > 64) window.__ohWorkerDiag.shift(); } catch (_) {}
  }

  function makeWrappedSrc(url, rec) {
    // Try sync-XHR to grab the original source so we can ship spy+original
    // as a single inline blob — avoids `importScripts` and `self.location`
    // breakage. CSP is bypassed via CDP Page.setBypassCSP, so blob: workers
    // are allowed.
    let originalSrc = '';
    try {
      const xhr = new XMLHttpRequest();
      xhr.open('GET', String(url), false);
      xhr.send();
      if (xhr.status >= 200 && xhr.status < 300) {
        originalSrc = xhr.responseText || '';
        rec.fetchOk = true;
        rec.fetchLen = originalSrc.length;
      } else {
        rec.fetchStatus = xhr.status;
      }
    } catch (e) {
      rec.fetchErr = String(e && e.message || e);
    }
    if (originalSrc) {
      return SPY_SRC + '\n' + originalSrc;
    }
    rec.fellBackToImportScripts = true;
    return SPY_SRC + '\n' +
      "try { importScripts(" + JSON.stringify(String(url)) + "); } catch (e) {}\n";
  }

  window.Worker = function (url, opts) {
    const rec = {
      ts: Date.now(),
      urlType: typeof url,
      urlIsURL: url instanceof URL,
      urlStr: (typeof url === 'string' || url instanceof URL) ? String(url).slice(0, 200) : null,
      optsType: opts && opts.type,
    };
    let workerUrl = url;
    try {
      if (typeof url === 'string' || url instanceof URL) {
        const src = makeWrappedSrc(url, rec);
        rec.srcLen = src.length;
        const blob = new Blob([src], { type: 'application/javascript' });
        workerUrl = URL.createObjectURL(blob);
        rec.blobUrl = String(workerUrl).slice(0, 80);
      }
    } catch (e) {
      rec.wrapErr = String(e && e.message || e);
      workerUrl = url;
    }
    let w;
    try {
      w = new Orig(workerUrl, opts);
      rec.constructed = true;
    } catch (e) {
      rec.constructErr = String(e && e.message || e);
      diag(rec);
      throw e;
    }
    try {
      wrappedWorkers.add(w);
      w.addEventListener('message', (ev) => {
        if (ev && ev.data && ev.data.__ohSpyHello) {
          if (!window.__ohWorkerHello.has(w)) {
            window.__ohWorkerHello.add(w);
            window.__ohWorkerHelloCount += 1;
            rec.helloAt = Date.now();
          }
        }
      });
      w.addEventListener('error', (ev) => {
        rec.errorAt = Date.now();
        rec.errorMsg = (ev && ev.message) ? String(ev.message).slice(0, 200) : '?';
        rec.errorFile = (ev && ev.filename) ? String(ev.filename).slice(0, 120) : null;
        rec.errorLine = ev && ev.lineno;
      });
      w.addEventListener('messageerror', () => { rec.messageError = true; });
    } catch (_) {}
    diag(rec);
    return w;
  };
  // Preserve prototype + statics where possible.
  try {
    window.Worker.prototype = Orig.prototype;
    Object.setPrototypeOf(window.Worker, Orig);
  } catch (_) {}
})();
