// IndexedDB scanner — runs inside a CEF page via CDP `Runtime.evaluate`.
// Returns a JSON object { ok, scannedAt, dbs[], chats{}, messages[],
// hadKey, error? } that the Rust side ingests. No api.* calls (we have no
// runtime here — this is a one-shot evaluation triggered by Rust).
//
// Decryption strategy:
//   WhatsApp Web persists most records as AES-GCM envelopes whose key is a
//   non-extractable CryptoKey ALSO stored in IndexedDB (round-trips via
//   structured clone). We deep-walk every record looking for a CryptoKey
//   instance, cache the first one, and call crypto.subtle.decrypt with it
//   directly (no importKey — it's already a CryptoKey).
//
// Returned shape per message:
//   { id, chatId, from, to, fromMe, body, type, timestamp }
//
// Caller controls re-scan cadence; we always do a full snapshot pass.
(async () => {
  // ─── crypto.subtle spy ────────────────────────────────────────────
  // Install once per page load. Captures deriveKey/deriveBits/decrypt
  // call args so we can see exactly what (info, salt) WhatsApp uses.
  // The spy stays put across our scans; we read the captured calls each
  // tick. We DON'T do this in production — it's a debug-only hook gated
  // on the existence of an explicit hook flag we set ourselves.
  if (!window.__ohSubtleSpy) {
    // `derivedKeys` keeps the actual resolved CryptoKey objects from
    // every successful deriveKey call — these are the keys WA actually
    // uses for AES-GCM, so we can re-use them for our own decryption
    // instead of trying to reproduce HKDF inputs.
    const cap = { derive: [], decrypt: [], decryptOk: [], derivedKeys: [] };
    window.__ohSubtleSpy = cap;
    const orig = {
      deriveKey: crypto.subtle.deriveKey.bind(crypto.subtle),
      deriveBits: crypto.subtle.deriveBits.bind(crypto.subtle),
      decrypt: crypto.subtle.decrypt.bind(crypto.subtle),
    };
    const summarizeBytes = (b) => {
      if (b == null) return null;
      let bytes;
      if (b instanceof Uint8Array) bytes = b;
      else if (b instanceof ArrayBuffer) bytes = new Uint8Array(b);
      else if (ArrayBuffer.isView(b)) bytes = new Uint8Array(b.buffer, b.byteOffset, b.byteLength);
      else return null;
      let hex = '';
      for (let i = 0; i < Math.min(bytes.length, 64); i += 1) hex += bytes[i].toString(16).padStart(2, '0');
      let utf8 = '';
      try { utf8 = new TextDecoder('utf-8', { fatal: false }).decode(bytes.subarray(0, Math.min(bytes.length, 64))); } catch (_) {}
      return { len: bytes.length, hex: hex + (bytes.length > 64 ? '…' : ''), utf8 };
    };
    const captureDerive = (label, params, baseKey, derived) => {
      try {
        cap.derive.push({
          fn: label,
          algName: params && params.name,
          algHash: params && params.hash,
          info: params ? summarizeBytes(params.info) : null,
          salt: params ? summarizeBytes(params.salt) : null,
          baseKeyAlg: baseKey && baseKey.algorithm && baseKey.algorithm.name,
          baseKeyUsages: baseKey && Array.from(baseKey.usages || []),
          derivedAlg: derived && (derived.name || (derived.algorithm && derived.algorithm.name)),
          derivedLen: derived && (derived.length || (typeof derived === 'number' ? derived : null)),
          ts: Date.now(),
        });
        if (cap.derive.length > 50) cap.derive.shift();
      } catch (_) {}
    };
    crypto.subtle.deriveKey = function (params, baseKey, derivedKeyType, extractable, usages) {
      captureDerive('deriveKey', params, baseKey, derivedKeyType);
      const p = orig.deriveKey(params, baseKey, derivedKeyType, extractable, usages);
      // Stash the resolved key — only AES-GCM derivations matter for us.
      const algName = derivedKeyType && (derivedKeyType.name || derivedKeyType);
      if (algName === 'AES-GCM') {
        const requestedUsages = Array.isArray(usages) ? usages.slice() : [];
        p.then((k) => {
          cap.derivedKeys.push({
            key: k,
            usages: Array.from(k.usages || requestedUsages),
            info: params && params.info ? new Uint8Array(
              params.info instanceof ArrayBuffer ? params.info :
              ArrayBuffer.isView(params.info) ? params.info.buffer.slice(params.info.byteOffset, params.info.byteOffset + params.info.byteLength) :
              params.info
            ) : null,
            salt: params && params.salt ? new Uint8Array(
              params.salt instanceof ArrayBuffer ? params.salt :
              ArrayBuffer.isView(params.salt) ? params.salt.buffer.slice(params.salt.byteOffset, params.salt.byteOffset + params.salt.byteLength) :
              params.salt
            ) : null,
          });
          if (cap.derivedKeys.length > 64) cap.derivedKeys.shift();
        }, () => {});
      }
      return p;
    };
    crypto.subtle.deriveBits = function (params, baseKey, length) {
      captureDerive('deriveBits', params, baseKey, length);
      return orig.deriveBits(params, baseKey, length);
    };
    crypto.subtle.decrypt = function (params, key, data) {
      const evt = {
        algName: params && params.name,
        ivLen: params && params.iv ? (params.iv.byteLength || params.iv.length) : null,
        aadLen: params && params.additionalData ? (params.additionalData.byteLength || params.additionalData.length) : null,
        keyAlg: key && key.algorithm && key.algorithm.name,
        keyUsages: key && Array.from(key.usages || []),
        dataLen: data && (data.byteLength || data.length),
        ts: Date.now(),
      };
      cap.decrypt.push(evt);
      if (cap.decrypt.length > 100) cap.decrypt.shift();
      const p = orig.decrypt(params, key, data);
      p.then((buf) => {
        cap.decryptOk.push({ ...evt, plainLen: buf && (buf.byteLength || buf.length) });
        if (cap.decryptOk.length > 100) cap.decryptOk.shift();
      }, () => {});
      return p;
    };
  }

  const out = {
    ok: false,
    scannedAt: Date.now(),
    dbs: [],
    chats: {},
    messages: [],
    hadKey: false,
    // Diagnostics so the Rust log can show what we actually got — first
    // decrypted message per scan + the names of every store we walked.
    sampleMessages: [], // up to N most recent decrypted messages, body preview included
    storeMap: {}, // dbName → [storeName, ...]
    keyCount: 0, // total CryptoKeys discovered (across all DBs/stores)
    keySources: [], // [dbName/storeName, ...] where keys were found
    // Union of all top-level keys observed across N sampled message records
    // — different message types (text/media/sticker/etc) use different
    // subsets of fields, so first-record-only shape was misleading.
    messageKeyUnion: null,
    messageTypeBreakdown: null, // { type: count }
    sampleByType: null, // { type: shapeOf(firstRecordOfThatType) }
    schemaDump: {}, // "db/store" → first row shape (for non-message stores of interest)
    opfs: null, // { ok, files: [...] } — origin-private filesystem listing
    keystoreSample: null, // { recordKey, shape, keys: [{path, alg, usages}] }
    cryptoSpy: null, // captured crypto.subtle.{deriveKey,deriveBits,decrypt} calls
    workerSpies: null, // { wrappedCount, replies: [...] } — per-worker spy dumps
    domMessages: [], // DOM-scraped rendered message bodies, joined with IndexedDB by data-id
  };
  const SAMPLE_COUNT = 5;
  const SAMPLE_BODY_PREVIEW = 120;
  const SCHEMA_DUMP_PATTERNS = ['comment', 'note', 'mutation', 'history', 'info', 'pinned'];

  // Exact (db, store) targets — discovered from a full store-map dump:
  //   model-storage/message  → message records
  //   model-storage/chat     → chat list
  //   model-storage/contact  → contact records (used for chat display names)
  //   model-storage/group-metadata → group display names
  // Substring matching grabbed `active-message-ranges` first (just metadata),
  // so we go fully exact now.
  const MESSAGE_STORES = new Set(['message']);
  const CHAT_STORES = new Set(['chat']);
  const CONTACT_STORES = new Set(['contact']);
  const GROUP_META_STORES = new Set(['group-metadata']);
  const MAX_RECORDS_PER_STORE = 20000;

  // ─── helpers ──────────────────────────────────────────────────────
  const isCryptoKey = (v) => typeof CryptoKey !== 'undefined' && v instanceof CryptoKey;

  const coerceBytes = (v) => {
    if (!v) return null;
    if (v instanceof Uint8Array) return v;
    if (v instanceof ArrayBuffer) return new Uint8Array(v);
    if (ArrayBuffer.isView(v)) return new Uint8Array(v.buffer, v.byteOffset, v.byteLength);
    return null;
  };

  const findCryptoKey = (v, depth) => {
    if (v == null || depth > 6) return null;
    if (isCryptoKey(v)) return v;
    if (typeof v !== 'object') return null;
    if (ArrayBuffer.isView(v) || v instanceof ArrayBuffer) return null;
    if (Array.isArray(v)) {
      for (const item of v) { const h = findCryptoKey(item, depth + 1); if (h) return h; }
      return null;
    }
    for (const k in v) {
      if (!Object.prototype.hasOwnProperty.call(v, k)) continue;
      const h = findCryptoKey(v[k], depth + 1);
      if (h) return h;
    }
    return null;
  };

  // Collect every reachable CryptoKey with the dotted path it was found
  // at, plus its algorithm + usages. Used to disambiguate the AES-GCM
  // body key from sibling HMAC / signing keys on the same record.
  const findAllCryptoKeys = (v, path, out) => {
    if (v == null) return;
    if (isCryptoKey(v)) {
      out.push({
        path: path || '<root>',
        key: v,
        alg: v.algorithm && v.algorithm.name,
        usages: Array.from(v.usages || []),
      });
      return;
    }
    if (typeof v !== 'object') return;
    if (ArrayBuffer.isView(v) || v instanceof ArrayBuffer) return;
    if (Array.isArray(v)) {
      v.forEach((item, i) => findAllCryptoKeys(item, path + '[' + i + ']', out));
      return;
    }
    for (const k in v) {
      if (!Object.prototype.hasOwnProperty.call(v, k)) continue;
      findAllCryptoKeys(v[k], path ? path + '.' + k : k, out);
    }
  };

  const coerceEnvelope = (v) => {
    if (!v) return null;
    if (typeof v === 'object' && !ArrayBuffer.isView(v) && !(v instanceof ArrayBuffer)) {
      const iv = coerceBytes(v.iv) || coerceBytes(v._iv);
      const ct = coerceBytes(v.ciphertext) || coerceBytes(v.data) || coerceBytes(v.payload) || coerceBytes(v.ct) || coerceBytes(v.encrypted);
      if (iv && ct && iv.length >= 12 && iv.length <= 16) return { iv, ciphertext: ct };
    }
    const bytes = coerceBytes(v);
    if (bytes && bytes.length > 28) return { iv: bytes.slice(0, 12), ciphertext: bytes.slice(12) };
    return null;
  };

  const tryDecrypt = async (value, key) => {
    if (!value || !key) return null;
    const env = coerceEnvelope(value);
    if (!env) return null;
    try {
      const buf = await crypto.subtle.decrypt({ name: 'AES-GCM', iv: env.iv }, key, env.ciphertext);
      const txt = new TextDecoder('utf-8', { fatal: false }).decode(buf);
      try { return JSON.parse(txt); } catch (_) { return txt; }
    } catch (_) { return null; }
  };

  const decryptDeep = async (v, key, depth) => {
    if (v == null || depth > 4) return v;
    if (typeof v !== 'object' || ArrayBuffer.isView(v) || v instanceof ArrayBuffer) return v;
    const asEnv = await tryDecrypt(v, key);
    if (asEnv !== null) return asEnv;
    if (Array.isArray(v)) {
      const arr = new Array(v.length);
      for (let i = 0; i < v.length; i += 1) arr[i] = await decryptDeep(v[i], key, depth + 1);
      return arr;
    }
    const obj = {};
    for (const k in v) {
      if (!Object.prototype.hasOwnProperty.call(v, k)) continue;
      obj[k] = await decryptDeep(v[k], key, depth + 1);
    }
    return obj;
  };

  const normalizeId = (v) => {
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
  };

  const normalizeMessage = (raw) => {
    if (!raw || typeof raw !== 'object') return null;
    const id = normalizeId(raw.id) || normalizeId(raw._id) || normalizeId(raw.key) || (typeof raw.id === 'string' ? raw.id : null);
    const from = normalizeId(raw.from) || normalizeId(raw.remoteJid);
    const to = normalizeId(raw.to);
    const author = normalizeId(raw.author) || normalizeId(raw.participant);
    const chatId = normalizeId(raw.chatId) || normalizeId(raw.remote) || from || to || null;
    const fromMe = !!(raw.fromMe || raw.isSentByMe || raw.isFromMe);
    const tsRaw = raw.t || raw.timestamp || raw.messageTimestamp || null;
    const ts = typeof tsRaw === 'number' ? tsRaw : (tsRaw ? Number(tsRaw) : null);
    let body = null;
    if (typeof raw.body === 'string') body = raw.body;
    else if (typeof raw.caption === 'string') body = raw.caption;
    else if (raw.body && typeof raw.body === 'object' && typeof raw.body.text === 'string') body = raw.body.text;
    else if (raw.message && typeof raw.message === 'object' && typeof raw.message.conversation === 'string') body = raw.message.conversation;
    return {
      id, chatId,
      from: fromMe ? 'me' : (author || from),
      to, fromMe, body,
      type: raw.type || (raw.message && Object.keys(raw.message)[0]) || null,
      timestamp: ts,
    };
  };

  const normalizeChat = (raw) => {
    if (!raw || typeof raw !== 'object') return null;
    const id = normalizeId(raw.id) || normalizeId(raw._id);
    if (!id) return null;
    const name = raw.name || raw.subject || (raw.contact && (raw.contact.name || raw.contact.pushname)) || raw.formattedTitle || null;
    return { id, name };
  };

  const openDb = (name) => new Promise((resolve, reject) => {
    const req = indexedDB.open(name);
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error || new Error('open ' + name));
    req.onblocked = () => reject(new Error('blocked ' + name));
  });

  const readAll = (db, storeName, limit) => new Promise((resolve, reject) => {
    const rows = [];
    let tx;
    try { tx = db.transaction(storeName, 'readonly'); }
    catch (e) { reject(e); return; }
    const req = tx.objectStore(storeName).openCursor();
    req.onsuccess = (ev) => {
      const c = ev.target.result;
      if (!c) { resolve(rows); return; }
      rows.push({ key: c.key, value: c.value });
      if (limit && rows.length >= limit) { resolve(rows); return; }
      c.continue();
    };
    req.onerror = () => reject(req.error || new Error('cursor'));
  });

  // ─── scan ─────────────────────────────────────────────────────────
  try {
    if (typeof indexedDB === 'undefined' || typeof indexedDB.databases !== 'function') {
      throw new Error('indexedDB.databases() unavailable');
    }
    const dbs = (await indexedDB.databases()).filter((d) => d && d.name);
    out.dbs = dbs.map((d) => d.name);

    // Quick shape summarizer for diagnostics — captures the top-level
    // keys + the type of each so we can see what an encrypted vs plain
    // record looks like without leaking content.
    const shapeOf = (v, depth) => {
      if (v == null || typeof v !== 'object') return typeof v;
      depth = depth || 0;
      const out = {};
      const keys = Object.keys(v); // no slice — show everything
      for (const k of keys) {
        const x = v[k];
        if (x == null) out[k] = String(x);
        else if (x instanceof Uint8Array) out[k] = 'Uint8Array(' + x.length + ')';
        else if (x instanceof ArrayBuffer) out[k] = 'ArrayBuffer(' + x.byteLength + ')';
        else if (ArrayBuffer.isView(x)) out[k] = x.constructor.name + '(' + x.byteLength + ')';
        else if (Array.isArray(x)) out[k] = 'Array(' + x.length + ')';
        else if (typeof CryptoKey !== 'undefined' && x instanceof CryptoKey) out[k] = 'CryptoKey';
        else if (typeof x === 'object') {
          // Recurse one level for the highly-suspicious envelope-shaped
          // fields so we can see their internal structure.
          if (depth < 1 && (k === 'msgRowOpaqueData' || k === 'opaqueData' || k === 'quotedMsg')) {
            out[k] = shapeOf(x, depth + 1);
          } else {
            out[k] = 'Object{' + Object.keys(x).slice(0, 8).join(',') + '}';
          }
        }
        else if (typeof x === 'string') out[k] = 'String(' + x.length + ')';
        else out[k] = typeof x;
      }
      return out;
    };

    // Collect ALL CryptoKeys we find — any one might be the local-storage
    // AES-GCM key. We try them in priority order (wawc_db_enc first).
    // [{ key, source: "dbName/storeName" }]
    const allKeys = [];
    // Map keyed by `_keyId` for `msgRowOpaqueData` decryption. WhatsApp's
    // `wawc_db_enc/keys` store holds rows whose record-key is the numeric
    // _keyId and whose value is `{ key: CryptoKey, ... }` (or similar).
    // We fill this in Pass 1.
    const keysById = new Map();

    // Pass 1: walk every store of every db, build a store map for the
    // log, harvest every CryptoKey, and dump shapes from message-related
    // stores so we can find whichever one carries the body.
    for (const info of dbs) {
      let db;
      try { db = await openDb(info.name); } catch (_) { continue; }
      const stores = Array.from(db.objectStoreNames || []);
      out.storeMap[info.name] = stores;
      for (const storeName of stores) {
        let rows;
        try { rows = await readAll(db, storeName, 500); } catch (_) { continue; }
        for (const r of rows) {
          const k = findCryptoKey(r.value, 0);
          if (k) {
            allKeys.push({ key: k, source: info.name + '/' + storeName });
            // Stop scanning this store after first key — any one record is
            // representative for our priority ordering.
            break;
          }
        }
        // Schema dump for body-hunt: any store whose name hints at
        // message content. Capture the first row's shape only.
        const lc = storeName.toLowerCase();
        if (rows.length && SCHEMA_DUMP_PATTERNS.some((p) => lc.indexOf(p) !== -1)) {
          out.schemaDump[info.name + '/' + storeName] = shapeOf(rows[0].value);
        }
        // The keystore: pull out every CryptoKey indexed by record key,
        // since msgRowOpaqueData._keyId points into here. We collect ALL
        // keys per record (not just first) so we can disambiguate
        // AES-GCM body key from sibling HMAC / signing keys.
        if (info.name === 'wawc_db_enc' && storeName === 'keys') {
          for (const r of rows) {
            const found = [];
            findAllCryptoKeys(r.value, '', found);
            // Stash the full shape of the record + every CryptoKey path/alg
            // so we can see what's actually in there.
            if (!out.keystoreSample) {
              out.keystoreSample = {
                recordKey: String(r.key),
                shape: shapeOf(r.value),
                keys: found.map((e) => ({ path: e.path, alg: e.alg, usages: e.usages })),
              };
            }
            // Prefer AES-GCM keys with `decrypt` usage.
            const ranked = found.slice().sort((a, b) => {
              const score = (e) => {
                let s = 0;
                if (e.alg === 'AES-GCM') s += 100;
                else if (e.alg && e.alg.startsWith('AES')) s += 50;
                if ((e.usages || []).indexOf('decrypt') !== -1) s += 50;
                if ((e.usages || []).indexOf('encrypt') !== -1) s += 10;
                return -s; // sort ascending → best first
              };
              return score(a) - score(b);
            });
            keysById.set(r.key, ranked.map((e) => e.key));
          }
        }
      }
      try { db.close(); } catch (_) {}
    }
    out.keyByIdCount = keysById.size;
    out.keyByIdSampleIds = Array.from(keysById.keys()).slice(0, 5).map(String);

    // Priority order: wawc_db_enc/keys first (this is WA's local-storage
    // encryption keystore), then anything else in wawc_db_enc, then
    // everything else. Dedupe by source so we don't try the same key twice.
    const priority = (src) => {
      if (src.startsWith('wawc_db_enc/keys')) return 0;
      if (src.startsWith('wawc_db_enc/')) return 1;
      if (src.startsWith('wawc/')) return 2;
      if (src.indexOf('signal-storage') !== -1) return 9; // probably wrong key
      return 5;
    };
    allKeys.sort((a, b) => priority(a.source) - priority(b.source));
    out.keyCount = allKeys.length;
    out.keySources = allKeys.map((e) => e.source);
    out.hadKey = allKeys.length > 0;

    const chatNames = new Map();
    const seen = new Set();
    // Union of every key seen across message records, with the type
    // signature observed for each. Lets us spot a `body`/`text`/`content`
    // field that only appears on text messages.
    const msgKeyUnion = new Map(); // fieldName -> Set<typeSignature>
    const msgTypeCounts = new Map(); // type -> count
    const msgSampleByType = new Map(); // type -> first record's shape
    // Decrypt-path counters so we can see where bodies are dropping off.
    let cntOpaqueSeen = 0;
    let cntOpaqueDecrypted = 0;
    let cntBodyExtracted = 0;
    const keyIdHistogram = new Map(); // _keyId -> count
    let sampleDecryptedHex = null;
    let sampleDecryptedHexKeyId = null;
    let workingDerivation = null; // { infoHex, infoUtf8, saltLen } once HKDF candidate works

    // Try each key in priority order until decryption yields a different
    // (richer) result. Returns the best-effort decrypted value.
    const decryptWithBestKey = async (value) => {
      let best = value;
      let bestScore = scoreValue(value);
      for (const { key } of allKeys) {
        try {
          const candidate = await decryptDeep(value, key, 0);
          const score = scoreValue(candidate);
          if (score > bestScore) {
            best = candidate;
            bestScore = score;
          }
        } catch (_) {}
      }
      return best;
    };

    // Heuristic: count plaintext-looking string fields. A successful
    // decrypt produces records with strings like `body`, `from`, `type`.
    // An encrypted record is mostly Uint8Arrays.
    function scoreValue(v) {
      if (v == null || typeof v !== 'object') return 0;
      let s = 0;
      for (const k of Object.keys(v)) {
        const x = v[k];
        if (typeof x === 'string') s += 1;
        else if (typeof x === 'number') s += 0.5;
        else if (x && typeof x === 'object' && !ArrayBuffer.isView(x)) s += 0.25;
      }
      return s;
    }

    // Cache of derived AES-GCM keys, keyed by `_keyId|infoHexHash` so we
    // don't re-derive for every message. Filled on first successful pair.
    const derivedKeyCache = new Map();
    // The candidate (info, salt) that worked the FIRST time we decrypted —
    // remember it and use it as the fast path for all subsequent messages.
    let workingDeriveParams = null;

    const enc = new TextEncoder();
    const concatBytes = (...parts) => {
      const total = parts.reduce((n, p) => n + (p ? p.length : 0), 0);
      const out = new Uint8Array(total);
      let o = 0;
      for (const p of parts) {
        if (!p) continue;
        out.set(p, o);
        o += p.length;
      }
      return out;
    };
    const u32le = (n) => {
      const b = new Uint8Array(4);
      new DataView(b.buffer).setUint32(0, (n >>> 0), true);
      return b;
    };

    // Build the catalog of (info, salt) candidates we'll try when
    // deriving an AES-GCM key from the HKDF master.
    const deriveCandidates = (msgKey, keyId, scheme) => {
      const cands = [];
      const labels = [
        '',
        'WA Local Encryption',
        'WhatsApp Local Encryption',
        'LocalStorageEncryption',
        'wa-local-encryption',
        'WAWC',
        'WAWC.LocalEncryption',
        'whatsapp-encryption',
        'localStorage',
        'WA Web Local Storage',
      ];
      const salts = [
        new Uint8Array(0),
        new Uint8Array(32), // 32 zero bytes (RFC 5869 "default" when not provided)
      ];
      if (typeof msgKey === 'string') {
        salts.push(enc.encode(msgKey));
      }
      const idBytes = u32le(keyId);
      const schemeBytes = u32le(scheme);
      // Bare: just the bytes
      for (const salt of salts) {
        cands.push({ info: new Uint8Array(0), salt });
        cands.push({ info: idBytes, salt });
        cands.push({ info: schemeBytes, salt });
        cands.push({ info: concatBytes(idBytes, schemeBytes), salt });
      }
      // Labelled
      for (const label of labels) {
        const lb = enc.encode(label);
        for (const salt of salts) {
          cands.push({ info: lb, salt });
          cands.push({ info: concatBytes(lb, idBytes), salt });
          cands.push({ info: concatBytes(lb, schemeBytes), salt });
          cands.push({ info: concatBytes(lb, idBytes, schemeBytes), salt });
        }
      }
      // msgKey-bound
      if (typeof msgKey === 'string') {
        const mb = enc.encode(msgKey);
        for (const label of ['', 'WA Local Encryption', 'WAWC.LocalEncryption']) {
          const lb = enc.encode(label);
          cands.push({ info: concatBytes(lb, mb), salt: new Uint8Array(0) });
        }
      }
      return cands;
    };

    // Derive an AES-GCM CryptoKey from an HKDF master, with caching.
    const deriveAesKey = async (hkdfKey, info, salt) => {
      const cacheTag = Array.from(info).map((b) => b.toString(16).padStart(2, '0')).join('') + '|' +
                       Array.from(salt).map((b) => b.toString(16).padStart(2, '0')).join('');
      if (derivedKeyCache.has(cacheTag)) return derivedKeyCache.get(cacheTag);
      try {
        const k = await crypto.subtle.deriveKey(
          { name: 'HKDF', hash: 'SHA-256', salt, info },
          hkdfKey,
          { name: 'AES-GCM', length: 256 },
          false,
          ['decrypt'],
        );
        derivedKeyCache.set(cacheTag, k);
        return k;
      } catch (_) {
        return null;
      }
    };

    // Decrypt msgRowOpaqueData → bytes. WhatsApp's scheme: master HKDF
    // key in `wawc_db_enc/keys/<keyId>.key`; per-row AES-GCM key derived
    // via HKDF with some (info, salt) tuple we have to discover.
    // Returns { plaintext, info, salt } or null.
    const decryptOpaque = async (opaque, msgKey) => {
      if (!opaque || typeof opaque !== 'object') return null;
      const iv = coerceBytes(opaque.iv);
      const ct = coerceBytes(opaque._data);
      const keyId = opaque._keyId;
      const scheme = opaque._scheme;
      if (!iv || !ct) return null;
      const candidates = keysById.get(keyId) || [];
      if (!candidates.length) return null;
      // Filter to HKDF master keys (the AES-GCM key is derived).
      const masters = candidates.filter(
        (k) => k && k.algorithm && k.algorithm.name === 'HKDF',
      );
      // Fallback: also try treating any AES-GCM key directly (in case
      // some accounts have non-HKDF schemes).
      const directs = candidates.filter(
        (k) => k && k.algorithm && k.algorithm.name === 'AES-GCM',
      );

      // Fast path: if a (master, info, salt) worked before, use it directly.
      if (workingDeriveParams) {
        const { master, info, salt } = workingDeriveParams;
        const aes = await deriveAesKey(master, info, salt);
        if (aes) {
          try {
            const buf = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, aes, ct);
            return { plaintext: new Uint8Array(buf) };
          } catch (_) { /* params stale; fall through to search */ }
        }
      }

      // *** Page-spy keys ***  Try WA's own derived AES-GCM CryptoKey
      // objects first (captured via our spy on `crypto.subtle.deriveKey`).
      const spy = window.__ohSubtleSpy;
      if (spy && spy.derivedKeys && spy.derivedKeys.length) {
        for (const dk of spy.derivedKeys) {
          // Skip keys without `decrypt` usage — Web Crypto enforces this.
          if ((dk.usages || []).indexOf('decrypt') === -1) continue;
          try {
            const buf = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, dk.key, ct);
            workingDeriveParams = { master: null, info: dk.info, salt: dk.salt };
            return { plaintext: new Uint8Array(buf), info: dk.info, salt: dk.salt };
          } catch (_) {}
        }
      }

      // *** Spy-driven re-derivation ***  Our captured derive-call args
      // tell us EXACTLY what (info, salt) WhatsApp uses. Re-run the
      // derivation against our master(s) via `deriveBits` so we control
      // the usages, then try AES-GCM decrypt.
      if (spy && spy.derive && spy.derive.length && masters.length) {
        // Dedupe by (info, salt) hex.
        const seenTuples = new Set();
        const tuples = [];
        for (const d of spy.derive) {
          if (!d || d.algName !== 'HKDF') continue;
          if (!d.info || !d.salt) continue;
          const tag = d.info.hex + '|' + d.salt.hex;
          if (seenTuples.has(tag)) continue;
          seenTuples.add(tag);
          // Reconstruct bytes from hex (we only have hex previews in the
          // captured data — not the original bytes — because info/salt
          // weren't pinned in the spy. So pull from the SAME captured
          // bytes the spy holds in derivedKeys when present, else hex).
          const fromHex = (h) => {
            if (!h) return new Uint8Array(0);
            const out = new Uint8Array(h.length / 2);
            for (let i = 0; i < out.length; i += 1) {
              out[i] = parseInt(h.substr(i * 2, 2), 16);
            }
            return out;
          };
          tuples.push({
            info: fromHex(d.info.hex),
            salt: fromHex(d.salt.hex),
          });
        }
        for (const master of masters) {
          for (const { info: tInfo, salt: tSalt } of tuples) {
            // Use deriveKey (not deriveBits) because WA's stored HKDF master
            // has usages=['deriveKey'] only — deriveBits would always throw.
            const aes = await deriveAesKey(master, tInfo, tSalt);
            if (!aes) continue;
            try {
              const buf = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, aes, ct);
              workingDeriveParams = { master, info: tInfo, salt: tSalt };
              return { plaintext: new Uint8Array(buf), info: tInfo, salt: tSalt };
            } catch (_) {}
          }
        }
      }

      // Try direct AES-GCM keys (no derivation).
      for (const k of directs) {
        try {
          const buf = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, k, ct);
          return { plaintext: new Uint8Array(buf) };
        } catch (_) {}
      }

      // Search: try each master × each (info, salt).
      const cands = deriveCandidates(msgKey, keyId, scheme);
      for (const master of masters) {
        for (const { info, salt } of cands) {
          const aes = await deriveAesKey(master, info, salt);
          if (!aes) continue;
          try {
            const buf = await crypto.subtle.decrypt({ name: 'AES-GCM', iv }, aes, ct);
            workingDeriveParams = { master, info, salt };
            return { plaintext: new Uint8Array(buf), info, salt };
          } catch (_) {}
        }
      }
      return null;
    };

    // Try to extract a human-readable text body from decrypted bytes.
    // WhatsApp uses protobuf for messages — proto field 1 (varint-len-prefixed
    // string) carries `conversation` text for plain text messages. We do a
    // minimal protobuf scan: look for a wire-tag 0x0A (field=1, type=length)
    // and read the following varint length + string.
    const extractTextFromProto = (bytes) => {
      if (!bytes || !bytes.length) return null;
      const decoder = new TextDecoder('utf-8', { fatal: false });
      // Try the literal-tag fast path first.
      let i = 0;
      while (i < bytes.length) {
        const tag = bytes[i];
        if (tag === 0x0A) {
          // length varint
          let len = 0, shift = 0, j = i + 1;
          while (j < bytes.length) {
            const b = bytes[j];
            len |= (b & 0x7f) << shift;
            j += 1;
            if (!(b & 0x80)) break;
            shift += 7;
            if (shift > 28) break;
          }
          if (len > 0 && j + len <= bytes.length) {
            const slice = bytes.subarray(j, j + len);
            const txt = decoder.decode(slice);
            // Reasonable text heuristic: at least one printable char,
            // mostly printable.
            const printable = txt.replace(/[\x00-\x08\x0B-\x1F]/g, '');
            if (printable.length >= Math.max(1, Math.floor(txt.length * 0.7))) {
              return txt;
            }
          }
        }
        i += 1;
      }
      // Fallback: best-effort full UTF-8 decode (may produce garbage).
      const txt = decoder.decode(bytes);
      const printable = txt.replace(/[\x00-\x08\x0B-\x1F]/g, '');
      if (printable.length >= Math.floor(txt.length * 0.5)) return txt;
      return null;
    };

    const normalizeContact = (raw) => {
      if (!raw || typeof raw !== 'object') return null;
      const id = normalizeId(raw.id) || normalizeId(raw._id);
      if (!id) return null;
      const name = raw.name || raw.notify || raw.shortName || raw.pushname || raw.verifiedName || null;
      return { id, name };
    };

    // Pass 2: chats + messages + contacts + group metadata, with
    // decryption applied to every record (active-message-ranges et al.
    // are skipped — we only touch exact store names now).
    for (const info of dbs) {
      let db;
      try { db = await openDb(info.name); } catch (_) { continue; }
      const stores = Array.from(db.objectStoreNames || []);

      for (const storeName of stores) {
        const isMsg = MESSAGE_STORES.has(storeName);
        const isChat = CHAT_STORES.has(storeName);
        const isContact = CONTACT_STORES.has(storeName);
        const isGroup = GROUP_META_STORES.has(storeName);
        if (!isMsg && !isChat && !isContact && !isGroup) continue;

        let rows;
        try { rows = await readAll(db, storeName, MAX_RECORDS_PER_STORE); } catch (_) { continue; }
        for (const r of rows) {
          const raw = r.value;
          let value = raw;
          if (allKeys.length) {
            value = await decryptWithBestKey(raw);
          }

          // Diagnostics: union the keys seen across every message record
          // and stash one full shape per `type`. With ~280 records spread
          // across text/media/sticker/system/etc, this surfaces fields
          // (like `body`) that only ever appear on text messages.
          if (isMsg && value && typeof value === 'object') {
            const t = (value.type && String(value.type)) || '<no-type>';
            msgTypeCounts.set(t, (msgTypeCounts.get(t) || 0) + 1);
            if (!msgSampleByType.has(t)) {
              msgSampleByType.set(t, shapeOf(value));
            }
            for (const k of Object.keys(value)) {
              const x = value[k];
              let sig;
              if (x == null) continue; // ignore undefined/null fields
              if (x instanceof Uint8Array) sig = 'Uint8Array';
              else if (x instanceof ArrayBuffer) sig = 'ArrayBuffer';
              else if (ArrayBuffer.isView(x)) sig = x.constructor.name;
              else if (Array.isArray(x)) sig = 'Array';
              else if (typeof CryptoKey !== 'undefined' && x instanceof CryptoKey) sig = 'CryptoKey';
              else if (typeof x === 'object') sig = 'Object';
              else sig = typeof x;
              if (!msgKeyUnion.has(k)) msgKeyUnion.set(k, new Set());
              msgKeyUnion.get(k).add(sig);
            }
          }

          if (isChat) {
            const c = normalizeChat(value);
            if (c && c.name) chatNames.set(c.id, c.name);
          }
          if (isContact) {
            const c = normalizeContact(value);
            if (c && c.name) chatNames.set(c.id, c.name);
          }
          if (isGroup) {
            const g = normalizeChat(value);
            if (g && g.name) chatNames.set(g.id, g.name);
          }
          if (isMsg) {
            const m = normalizeMessage(value);
            if (!m || !m.id || !m.chatId || !m.timestamp) continue;
            if (seen.has(m.id)) continue;
            seen.add(m.id);
            // Decrypt the per-message opaque body if present.
            if (!m.body && value && value.msgRowOpaqueData) {
              cntOpaqueSeen += 1;
              const opq = value.msgRowOpaqueData;
              const kid = opq._keyId;
              keyIdHistogram.set(kid, (keyIdHistogram.get(kid) || 0) + 1);
              const result = await decryptOpaque(opq, r.key);
              if (result && result.plaintext) {
                const plain = result.plaintext;
                cntOpaqueDecrypted += 1;
                if (!workingDerivation && result.info) {
                  let hex = '';
                  for (let h = 0; h < result.info.length; h += 1) {
                    hex += result.info[h].toString(16).padStart(2, '0');
                  }
                  let utf8 = '';
                  try { utf8 = new TextDecoder('utf-8', { fatal: false }).decode(result.info); }
                  catch (_) {}
                  workingDerivation = {
                    infoHex: hex,
                    infoUtf8: utf8,
                    infoLen: result.info.length,
                    saltLen: result.salt ? result.salt.length : 0,
                  };
                }
                if (!sampleDecryptedHex) {
                  const max = Math.min(plain.length, 64);
                  let hex = '';
                  for (let h = 0; h < max; h += 1) {
                    hex += plain[h].toString(16).padStart(2, '0');
                  }
                  sampleDecryptedHex = hex + (plain.length > max ? '...' : '');
                  sampleDecryptedHexKeyId = kid;
                }
                const text = extractTextFromProto(plain);
                if (text) {
                  m.body = text;
                  cntBodyExtracted += 1;
                }
                if (!m.bodyBytesLen) m.bodyBytesLen = plain.length;
              }
            }
            out.messages.push(m);
          }
        }
      }
      try { db.close(); } catch (_) {}
    }
    chatNames.forEach((v, k) => { out.chats[k] = v; });

    // Decrypt-path diagnostics.
    out.decryptStats = {
      opaqueSeen: cntOpaqueSeen,
      opaqueDecrypted: cntOpaqueDecrypted,
      bodyExtracted: cntBodyExtracted,
      keyIdHistogram: Array.from(keyIdHistogram.entries()).map(([k, v]) => [String(k), v]),
      sampleDecryptedHex,
      sampleDecryptedHexKeyId: sampleDecryptedHexKeyId == null ? null : String(sampleDecryptedHexKeyId),
      workingDerivation,
    };

    // Serialise message-key union + per-type sample for the log.
    {
      const union = {};
      msgKeyUnion.forEach((sigSet, name) => { union[name] = Array.from(sigSet).sort().join('|'); });
      out.messageKeyUnion = union;
      const types = {};
      msgTypeCounts.forEach((count, t) => { types[t] = count; });
      out.messageTypeBreakdown = types;
      const byType = {};
      msgSampleByType.forEach((shape, t) => { byType[t] = shape; });
      out.sampleByType = byType;
    }

    // Pick the N most-recent messages that actually have a body so the
    // Rust log can print real plaintext (proof that decryption worked).
    out.sampleMessages = out.messages
      .filter((m) => m.body && typeof m.body === 'string')
      .sort((a, b) => (b.timestamp || 0) - (a.timestamp || 0))
      .slice(0, SAMPLE_COUNT)
      .map((m) => ({
        chatId: m.chatId,
        chatName: out.chats[m.chatId] || null,
        fromMe: m.fromMe,
        from: m.from,
        timestamp: m.timestamp,
        bodyPreview: m.body.slice(0, SAMPLE_BODY_PREVIEW),
      }));

    // OPFS probe — WhatsApp Web has been migrating bodies into a
    // SQLite-via-WASM file living in the Origin Private File System.
    // List the root recursively so we can see what's there.
    try {
      if (navigator.storage && typeof navigator.storage.getDirectory === 'function') {
        const root = await navigator.storage.getDirectory();
        const files = [];
        const walk = async (dir, prefix) => {
          for await (const [name, handle] of dir.entries()) {
            const path = prefix ? prefix + '/' + name : name;
            if (handle.kind === 'file') {
              try {
                const f = await handle.getFile();
                files.push({ path, size: f.size, modified: f.lastModified });
              } catch (_) {
                files.push({ path, size: -1 });
              }
            } else if (handle.kind === 'directory') {
              if (files.length < 200) await walk(handle, path);
            }
          }
        };
        await walk(root, '');
        out.opfs = { ok: true, files: files.slice(0, 100) };
      } else {
        out.opfs = { ok: false, reason: 'navigator.storage.getDirectory unavailable' };
      }
    } catch (e) {
      out.opfs = { ok: false, reason: (e && e.message) || String(e) };
    }

    // Probe window globals for anything that looks like a local-encryption
    // helper — would let us call WA's own decryption directly.
    const probeGlobals = () => {
      const hits = [];
      const interesting = /local|encrypt|decrypt|opaque|wawc/i;
      for (const k of Object.keys(window).slice(0, 500)) {
        if (interesting.test(k)) hits.push(k);
      }
      return hits.slice(0, 30);
    };
    out.windowGlobals = probeGlobals();

    // Snapshot what each WRAPPED WORKER has captured — postMessage them
    // a {__ohSpy:'dump'} request and collect their replies. Wrapping
    // is set up by worker_hook.js (page-level Worker ctor override).
    out.workerSpies = await new Promise((resolve) => {
      const wrapped = window.__ohWrappedWorkers;
      const helloCount = window.__ohWorkerHelloCount || 0;
      const diag = (window.__ohWorkerDiag || []).slice(-12);
      if (!wrapped || wrapped.size === 0) {
        resolve({ wrappedCount: 0, helloCount, replies: [], diag });
        return;
      }
      const replies = [];
      const handlers = [];
      for (const w of wrapped) {
        const h = (ev) => {
          if (ev && ev.data && ev.data.__ohSpyReply) {
            replies.push(ev.data);
          }
        };
        try {
          w.addEventListener('message', h);
          handlers.push([w, h]);
          w.postMessage({ __ohSpy: 'dump' });
        } catch (_) {}
      }
      // Give workers up to 4s to reply.
      setTimeout(() => {
        for (const [w, h] of handlers) {
          try { w.removeEventListener('message', h); } catch (_) {}
        }
        resolve({ wrappedCount: wrapped.size, helloCount, replies, diag });
      }, 4000);
    });

    // Snapshot what the page's crypto.subtle has been doing.
    if (window.__ohSubtleSpy) {
      const s = window.__ohSubtleSpy;
      // Dedupe derives by (info.hex, salt.hex) to see ALL distinct
      // derivation patterns the page has used, not just last N.
      const distinct = new Map();
      for (const d of s.derive) {
        if (!d || d.algName !== 'HKDF') continue;
        const tag = (d.info && d.info.hex) + '|' + (d.salt && d.salt.hex);
        if (!distinct.has(tag)) distinct.set(tag, d);
      }
      // Histogram of decrypt dataLens — tells us if WA is decrypting
      // 48-byte (msg-row-opaque) blobs in the page or not.
      const decryptSizes = {};
      for (const e of s.decrypt) {
        if (e.algName !== 'AES-GCM') continue;
        decryptSizes[e.dataLen] = (decryptSizes[e.dataLen] || 0) + 1;
      }
      out.cryptoSpy = {
        deriveCount: s.derive.length,
        decryptCount: s.decrypt.length,
        decryptOkCount: s.decryptOk.length,
        derivedKeysCount: s.derivedKeys.length,
        derivedKeysWithDecrypt: s.derivedKeys.filter((dk) => (dk.usages || []).indexOf('decrypt') !== -1).length,
        distinctDeriveCount: distinct.size,
        distinctDerives: Array.from(distinct.values()),
        decryptSizes,
      };
    }

    // *** DOM scrape ***  WhatsApp randomizes CSS classes but stable hooks
    // remain: every rendered message row carries a `data-id` attribute with
    // format `"<fromMe>_<chatId>_<msgId>"` (e.g. `false_12345@c.us_3EB0A...`).
    // Message body text lives in a descendant `<span>` with selectable text;
    // the outer row also contains `[data-pre-plain-text="[HH:MM, D/M/YYYY]
    // Author Name: "]` which gives us author + timestamp metadata.
    try {
      const rows = document.querySelectorAll('[data-id]');
      const out_dom = [];
      const seen = new Set();
      rows.forEach((row) => {
        const dataId = row.getAttribute('data-id');
        if (!dataId || seen.has(dataId)) return;
        // Only message-shaped data-ids: must contain 2 underscores and end
        // with an uppercase hex run (msgId) — filters out chat-list rows etc.
        const parts = dataId.split('_');
        if (parts.length < 3) return;
        const fromMeTok = parts[0];
        if (fromMeTok !== 'true' && fromMeTok !== 'false') return;
        seen.add(dataId);
        const fromMe = fromMeTok === 'true';
        const chatId = parts[1];
        const msgId = parts.slice(2).join('_');
        // Author/timestamp from data-pre-plain-text when present.
        let authorLabel = null;
        let preTimestamp = null;
        const preEl = row.querySelector('[data-pre-plain-text]');
        const preAttr = preEl && preEl.getAttribute('data-pre-plain-text');
        if (preAttr) {
          // e.g. "[12:34, 3/15/2025] John Doe: " — pull time + name.
          const m = /^\[([^\]]+)\]\s*([^:]+):/.exec(preAttr);
          if (m) {
            preTimestamp = m[1];
            authorLabel = m[2].trim();
          }
        }
        // Body: prefer the copyable-text span (WhatsApp puts the full text
        // in a descendant span with contenteditable / selectable-text).
        // Fall back to row.innerText, filtering out the pre-plain prefix.
        let body = '';
        try {
          const spans = row.querySelectorAll('span.selectable-text, span[dir="ltr"], span[dir="rtl"]');
          for (const s of spans) {
            const t = (s.innerText || s.textContent || '').trim();
            if (t && t.length > body.length) body = t;
          }
          if (!body) {
            const full = (row.innerText || '').trim();
            body = full.replace(/^\[[^\]]+\][^:]*:\s*/, '');
          }
        } catch (_) {}
        if (!body && !preAttr) return; // nothing useful
        out_dom.push({
          dataId,
          fromMe,
          chatId,
          msgId,
          author: authorLabel,
          preTimestamp,
          body: body.slice(0, 4000),
        });
      });
      out.domMessages = out_dom;
    } catch (e) {
      out.domScrapeError = String(e && e.message || e);
    }

    out.ok = true;
    return out;
  } catch (err) {
    out.error = (err && err.message) || String(err);
    return out;
  }
})()
