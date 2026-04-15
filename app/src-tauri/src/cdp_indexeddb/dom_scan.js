// Fast-tick DOM scrape — extracts rendered WhatsApp message bodies from
// the page without touching IndexedDB. Designed to run every ~2s via
// `Runtime.evaluate` so the UI picks up new messages nearly live.
//
// Output (`{ ok, domMessages, hash }`) is fed back into the same ingest
// pipeline as the full scan. `hash` is a cheap stable digest of the row
// set so the Rust side can skip emission when nothing has changed.
(() => {
  try {
    const rows = document.querySelectorAll('[data-id]');
    const out = [];
    const seen = new Set();
    rows.forEach((row) => {
      const dataId = row.getAttribute('data-id');
      if (!dataId || seen.has(dataId)) return;
      const parts = dataId.split('_');
      if (parts.length < 3) return;
      const fromMeTok = parts[0];
      if (fromMeTok !== 'true' && fromMeTok !== 'false') return;
      seen.add(dataId);
      const fromMe = fromMeTok === 'true';
      const chatId = parts[1];
      const msgId = parts.slice(2).join('_');
      let authorLabel = null;
      let preTimestamp = null;
      const preEl = row.querySelector('[data-pre-plain-text]');
      const preAttr = preEl && preEl.getAttribute('data-pre-plain-text');
      if (preAttr) {
        const m = /^\[([^\]]+)\]\s*([^:]+):/.exec(preAttr);
        if (m) {
          preTimestamp = m[1];
          authorLabel = m[2].trim();
        }
      }
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
      if (!body && !preAttr) return;
      out.push({
        dataId,
        fromMe,
        chatId,
        msgId,
        author: authorLabel,
        preTimestamp,
        body: body.slice(0, 4000),
      });
    });
    // Tiny rolling hash over (dataId, body) so the Rust side can skip
    // emission when nothing changed. FNV-1a style, 32-bit.
    let h = 2166136261 >>> 0;
    for (const r of out) {
      const s = r.dataId + '\x01' + (r.body || '');
      for (let i = 0; i < s.length; i += 1) {
        h ^= s.charCodeAt(i);
        h = Math.imul(h, 16777619);
      }
    }
    return { ok: true, domMessages: out, hash: h >>> 0 };
  } catch (e) {
    return { ok: false, error: (e && e.message) || String(e) };
  }
})()
