// WhatsApp Web recipe — IndexedDB-backed.
//
// Reads chat + message data straight out of WhatsApp's IndexedDB stores.
// No DOM scrape, no wa-js, no WPP hooks.
//
// Storage layout (as of 2024-2025, subject to change):
//   DB `model-storage-v<N>-<jid>`  → app models (chat, message, contact, …)
//   DB `signal-storage-v<N>-<jid>` → Signal protocol state
//   DB `wawc`                      → misc, often holds the CryptoKey
//
// WhatsApp Web encrypts most IndexedDB values at rest with AES-GCM.
// The symmetric key is persisted as a **non-extractable `CryptoKey`
// object** (not raw bytes) — it round-trips through IndexedDB's structured
// clone so you get a real CryptoKey back, usable with `crypto.subtle.decrypt`
// without re-importing. We detect any CryptoKey found during scan, cache it,
// and use it directly.
//
// Envelope shape observed on encrypted records:
//   { _index: ..., iv: Uint8Array(12..16), data: Uint8Array(ct+tag) }
//   or a packed Uint8Array = [iv(12) || ct+tag]. We try both.
//
// Flow per scan:
//   1. indexedDB.databases() → list all origin DBs.
//   2. First pass: open every DB, walk every object store, hunt for
//      CryptoKey objects. Cache the first one found.
//   3. Second pass: for message-like stores, read records; decrypt each
//      record (wholesale or field-by-field) with the cached key; normalize;
//      bucket by (chatId, UTC day); debounce-flush to api.ingest.
//   4. Also emits schema-sample logs the first few scans so we can iterate
//      on the real shape without a DevTools session.
(function (api) {
  if (!api) return;
  api.log('info', '[whatsapp-recipe] starting (indexeddb-backed)');

  // ─── Tunables ───────────────────────────────────────────────────────
  const FLUSH_DEBOUNCE_MS = 2000;
  const SCAN_INTERVAL_MS = 15000;
  const MAX_RECORDS_PER_STORE = 20000;
  const SCHEMA_DUMP_SCANS = 2; // log 1-record samples for the first N scans

  // Heuristics — stores whose records look like messages.
  const MESSAGE_STORE_HINTS = ['message', 'messages', 'msg', 'msgs', 'message-main'];
  // Stores that look like chat/conversation lists.
  const CHAT_STORE_HINTS = ['chat', 'chats', 'conversation', 'conversations'];

  // ─── State ──────────────────────────────────────────────────────────
  // (chatId, day) → { chatName, messages: Map<id, msg>, timer, isSeed }
  const buffers = new Map();
  const seenMessageIds = new Set();
  const chatNames = new Map();
  let cryptoKey = null;
  let scanCount = 0;
  let scanInFlight = false;

  // ─── Day bucketing / flush ──────────────────────────────────────────
  function dayKey(tsSeconds) {
    const ms = (tsSeconds || Math.floor(Date.now() / 1000)) * 1000;
    const d = new Date(ms);
    return (
      d.getUTCFullYear() +
      '-' + String(d.getUTCMonth() + 1).padStart(2, '0') +
      '-' + String(d.getUTCDate()).padStart(2, '0')
    );
  }

  function bufferKey(chatId, day) { return chatId + '|' + day; }

  function scheduleFlush(chatId, day) {
    const key = bufferKey(chatId, day);
    const entry = buffers.get(key);
    if (!entry) return;
    if (entry.timer) clearTimeout(entry.timer);
    entry.timer = setTimeout(function () { flush(chatId, day); }, FLUSH_DEBOUNCE_MS);
  }

  function flush(chatId, day) {
    const key = bufferKey(chatId, day);
    const entry = buffers.get(key);
    if (!entry) return;
    entry.timer = null;
    if (!entry.messages.size) return;
    const messages = Array.from(entry.messages.values()).sort(function (a, b) {
      return (a.timestamp || 0) - (b.timestamp || 0);
    });
    api.ingest({
      provider: 'whatsapp',
      chatId: chatId,
      chatName: entry.chatName || chatNames.get(chatId) || null,
      day: day,
      messages: messages,
      isSeed: !!entry.isSeed,
    });
    entry.isSeed = false;
    entry.messages.clear();
  }

  function addToBuffer(msg, opts) {
    if (!msg || !msg.id || !msg.chatId || !msg.timestamp) return;
    if (seenMessageIds.has(msg.id)) return;
    seenMessageIds.add(msg.id);
    const day = dayKey(msg.timestamp);
    const key = bufferKey(msg.chatId, day);
    let entry = buffers.get(key);
    if (!entry) {
      entry = {
        chatName: chatNames.get(msg.chatId) || null,
        messages: new Map(),
        timer: null,
        isSeed: !!(opts && opts.seed),
      };
      buffers.set(key, entry);
    }
    entry.messages.set(msg.id, msg);
    scheduleFlush(msg.chatId, day);
  }

  // ─── IndexedDB helpers ──────────────────────────────────────────────
  async function listDatabases() {
    if (typeof indexedDB === 'undefined') return [];
    if (typeof indexedDB.databases !== 'function') {
      api.log('warn', '[whatsapp-recipe] indexedDB.databases() unavailable');
      return [];
    }
    try {
      const dbs = await indexedDB.databases();
      return (dbs || []).filter(function (d) { return d && d.name; });
    } catch (err) {
      api.log('warn', '[whatsapp-recipe] listDatabases failed: ' + (err && err.message));
      return [];
    }
  }

  function openDb(name) {
    return new Promise(function (resolve, reject) {
      const req = indexedDB.open(name);
      req.onsuccess = function () { resolve(req.result); };
      req.onerror = function () { reject(req.error || new Error('open failed: ' + name)); };
      req.onblocked = function () { reject(new Error('open blocked: ' + name)); };
    });
  }

  function storeNames(db) {
    try { return Array.from(db.objectStoreNames || []); }
    catch (_) { return []; }
  }

  function readAll(db, storeName, limit) {
    return new Promise(function (resolve, reject) {
      const out = [];
      let tx;
      try { tx = db.transaction(storeName, 'readonly'); }
      catch (e) { reject(e); return; }
      const store = tx.objectStore(storeName);
      const req = store.openCursor();
      req.onsuccess = function (ev) {
        const cursor = ev.target.result;
        if (!cursor) { resolve(out); return; }
        out.push({ key: cursor.key, value: cursor.value });
        if (limit && out.length >= limit) { resolve(out); return; }
        cursor.continue();
      };
      req.onerror = function () { reject(req.error || new Error('cursor failed')); };
    });
  }

  // ─── CryptoKey / byte coercion ──────────────────────────────────────
  function isCryptoKey(v) {
    return typeof CryptoKey !== 'undefined' && v instanceof CryptoKey;
  }

  function coerceBytes(v) {
    if (!v) return null;
    if (v instanceof Uint8Array) return v;
    if (v instanceof ArrayBuffer) return new Uint8Array(v);
    if (ArrayBuffer.isView(v)) return new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
    if (typeof v === 'string') {
      try {
        const bin = atob(v);
        const out = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i += 1) out[i] = bin.charCodeAt(i);
        return out;
      } catch (_) { return null; }
    }
    return null;
  }

  // Deep-walk `v` looking for a CryptoKey. Returns the first one found.
  function findCryptoKey(v, depth) {
    if (v == null || depth > 6) return null;
    if (isCryptoKey(v)) return v;
    if (typeof v !== 'object') return null;
    if (ArrayBuffer.isView(v) || v instanceof ArrayBuffer) return null;
    if (Array.isArray(v)) {
      for (let i = 0; i < v.length; i += 1) {
        const hit = findCryptoKey(v[i], depth + 1);
        if (hit) return hit;
      }
      return null;
    }
    for (const k in v) {
      if (!Object.prototype.hasOwnProperty.call(v, k)) continue;
      const hit = findCryptoKey(v[k], depth + 1);
      if (hit) return hit;
    }
    return null;
  }

  // ─── Envelope detection & decryption ────────────────────────────────
  function coerceEnvelope(v) {
    if (!v) return null;
    if (typeof v === 'object' && !ArrayBuffer.isView(v)) {
      // Object-style: { iv, ciphertext|data|payload }
      const iv = coerceBytes(v.iv) || coerceBytes(v._iv);
      const ct =
        coerceBytes(v.ciphertext) ||
        coerceBytes(v.data) ||
        coerceBytes(v.payload) ||
        coerceBytes(v.ct) ||
        coerceBytes(v.encrypted);
      if (iv && ct && iv.length >= 12 && iv.length <= 16) return { iv: iv, ciphertext: ct };
    }
    const bytes = coerceBytes(v);
    if (bytes && bytes.length > 28) {
      // Packed: first 12 bytes IV, remainder = ciphertext || tag (16 bytes).
      return { iv: bytes.slice(0, 12), ciphertext: bytes.slice(12) };
    }
    return null;
  }

  async function tryDecrypt(value, key) {
    if (!value || !key) return null;
    const env = coerceEnvelope(value);
    if (!env) return null;
    try {
      const buf = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: env.iv }, key, env.ciphertext);
      const txt = new TextDecoder('utf-8', { fatal: false }).decode(buf);
      try { return JSON.parse(txt); } catch (_) { return txt; }
    } catch (_) { return null; }
  }

  // Recursively walk an object, decrypting any envelope-shaped sub-values
  // using the given key. Returns a new object with decrypted replacements;
  // leaves non-envelope values untouched. Depth-bounded to avoid cycles.
  async function decryptDeep(v, key, depth) {
    if (v == null || depth > 4) return v;
    if (typeof v !== 'object' || ArrayBuffer.isView(v) || v instanceof ArrayBuffer) return v;

    // Is the object itself an envelope?
    const asEnv = await tryDecrypt(v, key);
    if (asEnv !== null) return asEnv;

    if (Array.isArray(v)) {
      const out = new Array(v.length);
      for (let i = 0; i < v.length; i += 1) out[i] = await decryptDeep(v[i], key, depth + 1);
      return out;
    }
    const out = {};
    for (const k in v) {
      if (!Object.prototype.hasOwnProperty.call(v, k)) continue;
      out[k] = await decryptDeep(v[k], key, depth + 1);
    }
    return out;
  }

  // ─── Record normalization ───────────────────────────────────────────
  function normalizeId(v) {
    if (!v) return null;
    if (typeof v === 'string') return v;
    if (v._serialized) return v._serialized;
    if (v.id && v.id._serialized) return v.id._serialized;
    if (v.id && typeof v.id === 'string') return v.id;
    if (v.remote) {
      const base = v.remote._serialized || v.remote;
      return typeof base === 'string' ? base : null;
    }
    return null;
  }

  function normalizeMessage(raw) {
    if (!raw || typeof raw !== 'object') return null;
    const id =
      normalizeId(raw.id) ||
      normalizeId(raw._id) ||
      normalizeId(raw.key) ||
      (typeof raw.id === 'string' ? raw.id : null);
    const from = normalizeId(raw.from) || normalizeId(raw.remoteJid);
    const to = normalizeId(raw.to);
    const author = normalizeId(raw.author) || normalizeId(raw.participant);
    const chatId =
      normalizeId(raw.chatId) ||
      normalizeId(raw.remote) ||
      from ||
      to ||
      null;
    const fromMe = !!(raw.fromMe || raw.isSentByMe || raw.isFromMe);
    const ts = raw.t || raw.timestamp || raw.messageTimestamp || null;

    let body = null;
    if (typeof raw.body === 'string') body = raw.body;
    else if (typeof raw.caption === 'string') body = raw.caption;
    else if (raw.body && typeof raw.body === 'object' && typeof raw.body.text === 'string') body = raw.body.text;
    else if (raw.message && typeof raw.message === 'object' && typeof raw.message.conversation === 'string') {
      body = raw.message.conversation;
    }

    return {
      id: id,
      chatId: chatId,
      from: fromMe ? 'me' : (author || from),
      to: to,
      fromMe: fromMe,
      body: body,
      type: raw.type || (raw.message && Object.keys(raw.message)[0]) || null,
      timestamp: typeof ts === 'number' ? ts : (ts ? Number(ts) : null),
    };
  }

  function normalizeChat(raw) {
    if (!raw || typeof raw !== 'object') return null;
    const id = normalizeId(raw.id) || normalizeId(raw._id);
    if (!id) return null;
    const name =
      raw.name ||
      raw.subject ||
      (raw.contact && (raw.contact.name || raw.contact.pushname)) ||
      raw.formattedTitle ||
      null;
    return { id: id, name: name };
  }

  // ─── Schema dump (first few scans) ──────────────────────────────────
  function summarize(v, depth) {
    if (v == null) return String(v);
    if (isCryptoKey(v)) {
      return 'CryptoKey{algorithm=' + (v.algorithm && v.algorithm.name) +
        ',type=' + v.type + ',usages=[' + (v.usages || []).join(',') + ']}';
    }
    if (typeof v === 'string') return 'String(' + v.length + ')';
    if (typeof v === 'number' || typeof v === 'boolean') return typeof v;
    if (v instanceof Uint8Array) return 'Uint8Array(' + v.length + ')';
    if (v instanceof ArrayBuffer) return 'ArrayBuffer(' + v.byteLength + ')';
    if (ArrayBuffer.isView(v)) return v.constructor.name + '(' + v.byteLength + ')';
    if (Array.isArray(v)) return 'Array(' + v.length + (v.length && depth < 2 ? '): [' + summarize(v[0], depth + 1) + ',…]' : ')');
    if (typeof v === 'object') {
      if (depth >= 2) return 'Object{…}';
      const keys = Object.keys(v).slice(0, 8);
      const parts = keys.map(function (k) { return k + ':' + summarize(v[k], depth + 1); });
      return '{' + parts.join(',') + (Object.keys(v).length > 8 ? ',…' : '') + '}';
    }
    return typeof v;
  }

  function logSchemaSample(dbName, storeName, row) {
    if (!row) return;
    const keyStr = typeof row.key === 'string' ? row.key : summarize(row.key, 0);
    api.log('info', '[whatsapp-recipe][schema] ' + dbName + '/' + storeName +
      ' key=' + keyStr + ' val=' + summarize(row.value, 0));
  }

  // ─── Main scan ──────────────────────────────────────────────────────
  async function scanDatabases() {
    if (scanInFlight) return;
    scanInFlight = true;
    scanCount += 1;
    const dumpSchema = scanCount <= SCHEMA_DUMP_SCANS;

    try {
      const dbs = await listDatabases();
      if (!dbs.length) {
        api.log('info', '[whatsapp-recipe] no IndexedDB databases yet (not logged in?)');
        return;
      }
      api.log('info', '[whatsapp-recipe] scan#' + scanCount + ' dbs=[' +
        dbs.map(function (d) { return d.name; }).join(', ') + ']');

      // Pass 1: hunt for CryptoKey + populate chatNames + schema dump.
      for (let i = 0; i < dbs.length; i += 1) {
        const dbName = dbs[i].name;
        let db;
        try { db = await openDb(dbName); }
        catch (err) {
          api.log('warn', '[whatsapp-recipe] open ' + dbName + ' failed: ' + (err && err.message));
          continue;
        }
        const stores = storeNames(db);
        if (dumpSchema) {
          api.log('info', '[whatsapp-recipe][schema] ' + dbName + ' stores=[' + stores.join(', ') + ']');
        }

        for (let s = 0; s < stores.length; s += 1) {
          const storeName = stores[s];
          let rows;
          try { rows = await readAll(db, storeName, cryptoKey ? 1 : 500); }
          catch (_) { continue; }

          if (dumpSchema && rows.length) logSchemaSample(dbName, storeName, rows[0]);

          // Look for CryptoKey in every record until found.
          if (!cryptoKey) {
            for (let r = 0; r < rows.length; r += 1) {
              const k = findCryptoKey(rows[r].value, 0);
              if (k) {
                cryptoKey = k;
                api.log('info', '[whatsapp-recipe] found CryptoKey in ' + dbName + '/' + storeName +
                  ' alg=' + (k.algorithm && k.algorithm.name) + ' usages=[' + (k.usages || []).join(',') + ']');
                break;
              }
            }
          }

          // Chat names (only useful from small chat stores).
          if (CHAT_STORE_HINTS.indexOf(storeName) !== -1) {
            rows.forEach(function (row) {
              const c = normalizeChat(row.value);
              if (c && c.name) chatNames.set(c.id, c.name);
            });
          }
        }
        try { db.close(); } catch (_) {}
      }

      if (!cryptoKey) {
        api.log('warn', '[whatsapp-recipe] no CryptoKey found yet — messages may still be readable if stored plain');
      }

      // Pass 2: read message stores with decryption.
      let totalIngested = 0;
      for (let i = 0; i < dbs.length; i += 1) {
        const dbName = dbs[i].name;
        let db;
        try { db = await openDb(dbName); }
        catch (_) { continue; }
        const stores = storeNames(db);

        for (let s = 0; s < stores.length; s += 1) {
          const storeName = stores[s];
          if (MESSAGE_STORE_HINTS.indexOf(storeName) === -1) continue;
          let rows;
          try { rows = await readAll(db, storeName, MAX_RECORDS_PER_STORE); }
          catch (e) {
            api.log('warn', '[whatsapp-recipe] read ' + dbName + '/' + storeName + ' failed: ' + (e && e.message));
            continue;
          }
          api.log('info', '[whatsapp-recipe] ' + dbName + '/' + storeName + ' → ' + rows.length + ' raw records');

          let ingested = 0;
          for (let r = 0; r < rows.length; r += 1) {
            let value = rows[r].value;
            if (cryptoKey) {
              try { value = await decryptDeep(value, cryptoKey, 0); }
              catch (_) { /* leave as-is */ }
            }
            const norm = normalizeMessage(value);
            if (!norm || !norm.id || !norm.chatId || !norm.timestamp) continue;
            addToBuffer(norm, { seed: true });
            ingested += 1;
          }
          api.log('info', '[whatsapp-recipe] ' + dbName + '/' + storeName + ' → ' + ingested + ' ingested');
          totalIngested += ingested;
        }
        try { db.close(); } catch (_) {}
      }

      api.log('info', '[whatsapp-recipe] scan#' + scanCount + ' done, ingested=' + totalIngested +
        ' buffers=' + buffers.size + ' chats=' + chatNames.size);

      // Flush any buckets still pending (seed-style immediate flush).
      const keys = Array.from(buffers.keys());
      keys.forEach(function (k) {
        const sep = k.lastIndexOf('|');
        if (sep === -1) return;
        flush(k.slice(0, sep), k.slice(sep + 1));
      });
    } catch (err) {
      api.log('error', '[whatsapp-recipe] scan failed: ' + (err && err.message));
    } finally {
      scanInFlight = false;
    }
  }

  // ─── Composer attach (send path still uses the DOM) ─────────────────
  let attachedComposerEl = null;
  let attachedHandle = null;

  function findComposer() {
    return (
      document.querySelector('div[contenteditable="true"][data-tab="10"]') ||
      document.querySelector('footer div[contenteditable="true"][role="textbox"]') ||
      document.querySelector('div[contenteditable="true"][data-lexical-editor="true"]')
    );
  }

  function ensureComposerAttached() {
    const el = findComposer();
    if (!el) return;
    if (el === attachedComposerEl) return;
    if (attachedHandle) { try { attachedHandle.detach(); } catch (_) {} }
    attachedComposerEl = el;
    attachedHandle = api.attachComposer(el, {
      id: 'whatsapp:composer',
      providerHint: 'whatsapp',
      debounceMs: 250,
      suggestionKey: 'Tab',
    });
    api.log('info', '[whatsapp-recipe] composer attached');
  }

  // ─── Main loop ──────────────────────────────────────────────────────
  let lastScanAt = 0;
  api.loop(function () {
    ensureComposerAttached();
    const now = Date.now();
    if (now - lastScanAt >= SCAN_INTERVAL_MS) {
      lastScanAt = now;
      void scanDatabases();
    }
  });

  // Kick once early so we don't wait a full interval.
  setTimeout(function () { void scanDatabases(); }, 2500);

  if (typeof api.onNotify === 'function') {
    api.onNotify(function (n) {
      api.log('info', '[whatsapp-recipe] notify: ' + (n && n.title ? n.title : ''));
    });
  }
})(window.__openhumanRecipe);
