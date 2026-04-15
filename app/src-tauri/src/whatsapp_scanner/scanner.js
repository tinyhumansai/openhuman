// IndexedDB scanner — runs inside a CEF page via CDP `Runtime.evaluate`.
// Returns a JSON object { ok, scannedAt, dbs[], chats{}, messages[], error? }
// that the Rust side ingests. No api.* calls (we have no runtime here —
// this is a one-shot evaluation triggered by Rust).
//
// Read strategy:
//   Walk WhatsApp's IndexedDB stores (model-storage, signal-storage, etc.)
//   and pull plaintext metadata — message ids, chat ids, timestamps,
//   fromMe, sender JIDs, chat names, contact names. Message bodies are
//   encrypted at rest (`msgRowOpaqueData`) and we do NOT try to decrypt
//   them here; body text comes from the companion DOM scrape that runs
//   on the fast 2s tick and is merged into the messages by data-id.
//
// Returned shape per message:
//   { id, chatId, from, to, fromMe, body, type, timestamp }
(async () => {
  const out = {
    ok: false,
    scannedAt: Date.now(),
    dbs: [],
    chats: {},
    messages: [],
    // Diagnostics so the Rust log can show what we actually got.
    sampleMessages: [], // up to N most recent messages, body preview included
    storeMap: {}, // dbName → [storeName, ...]
    // Union of all top-level keys observed across N sampled message records
    // — different message types (text/media/sticker/etc) use different
    // subsets of fields, so first-record-only shape was misleading.
    messageKeyUnion: null,
    messageTypeBreakdown: null, // { type: count }
    sampleByType: null, // { type: shapeOf(firstRecordOfThatType) }
    schemaDump: {}, // "db/store" → first row shape (for non-message stores of interest)
    opfs: null, // { ok, files: [...] } — origin-private filesystem listing
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

    // Pass 1: walk every store of every db, build a store map for the
    // log, and dump shapes from message-related stores for diagnostics.
    for (const info of dbs) {
      let db;
      try { db = await openDb(info.name); } catch (_) { continue; }
      const stores = Array.from(db.objectStoreNames || []);
      out.storeMap[info.name] = stores;
      for (const storeName of stores) {
        let rows;
        try { rows = await readAll(db, storeName, 500); } catch (_) { continue; }
        const lc = storeName.toLowerCase();
        if (rows.length && SCHEMA_DUMP_PATTERNS.some((p) => lc.indexOf(p) !== -1)) {
          out.schemaDump[info.name + '/' + storeName] = shapeOf(rows[0].value);
        }
      }
      try { db.close(); } catch (_) {}
    }

    const chatNames = new Map();
    const seen = new Set();
    // Union of every key seen across message records, with the type
    // signature observed for each. Lets us spot a `body`/`text`/`content`
    // field that only appears on text messages.
    const msgKeyUnion = new Map(); // fieldName -> Set<typeSignature>
    const msgTypeCounts = new Map(); // type -> count
    const msgSampleByType = new Map(); // type -> first record's shape

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
          const value = r.value;

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
            // Body text comes from the DOM scrape path; IDB only gives us
            // metadata. No decrypt attempt here.
            out.messages.push(m);
          }
        }
      }
      try { db.close(); } catch (_) {}
    }
    chatNames.forEach((v, k) => { out.chats[k] = v; });

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
