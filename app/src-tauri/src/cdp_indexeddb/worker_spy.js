// Worker-context spy: installs a hook on `crypto.subtle.deriveKey` /
// `decrypt` (idempotent) and returns whatever has been captured so far.
// The script runs each scan tick in every WhatsApp worker target via
// CDP `Runtime.evaluate`, so each tick we get an updated snapshot of the
// key derivations and decryptions happening inside that worker.
//
// We deliberately keep this *tiny* — workers don't need IndexedDB access
// here; their job is just to expose the (info, salt) tuples WA's worker
// code uses for HKDF, which we then bring back to the page scanner so
// IT can re-derive an AES-GCM key with `decrypt` usage and unwrap
// `msgRowOpaqueData` blobs.
(() => {
  const g = (typeof globalThis !== 'undefined' ? globalThis : self);
  if (!g.__ohWorkerSpy) {
    const cap = { derive: [], decrypt: [], decryptOk: [] };
    g.__ohWorkerSpy = cap;
    const orig = {
      deriveKey: g.crypto.subtle.deriveKey.bind(g.crypto.subtle),
      deriveBits: g.crypto.subtle.deriveBits.bind(g.crypto.subtle),
      decrypt: g.crypto.subtle.decrypt.bind(g.crypto.subtle),
    };
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
      return { len: bytes.length, hex: hex + (bytes.length > 128 ? '…' : ''), utf8 };
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
          derivedAlg: derived && (derived.name || (derived.algorithm && derived.algorithm.name)),
          ts: Date.now(),
        });
        if (cap.derive.length > 200) cap.derive.shift();
      } catch (_) {}
    };
    g.crypto.subtle.deriveKey = function (params, baseKey, derivedKeyType, extractable, usages) {
      captureDerive('deriveKey', params, baseKey, derivedKeyType);
      return orig.deriveKey(params, baseKey, derivedKeyType, extractable, usages);
    };
    g.crypto.subtle.deriveBits = function (params, baseKey, length) {
      captureDerive('deriveBits', params, baseKey, length);
      return orig.deriveBits(params, baseKey, length);
    };
    g.crypto.subtle.decrypt = function (params, key, data) {
      const evt = {
        algName: params && params.name,
        ivLen: params && params.iv ? (params.iv.byteLength || params.iv.length) : null,
        keyAlg: key && key.algorithm && key.algorithm.name,
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
  }

  const s = g.__ohWorkerSpy;
  // Dedupe HKDF derives by (info.hex, salt.hex).
  const distinct = new Map();
  for (const d of s.derive) {
    if (!d || d.algName !== 'HKDF') continue;
    const tag = (d.info && d.info.hex) + '|' + (d.salt && d.salt.hex);
    if (!distinct.has(tag)) distinct.set(tag, d);
  }
  const decryptSizes = {};
  for (const e of s.decrypt) {
    if (e.algName !== 'AES-GCM') continue;
    decryptSizes[e.dataLen] = (decryptSizes[e.dataLen] || 0) + 1;
  }
  const decryptOkSizes = {};
  for (const e of s.decryptOk) {
    if (e.algName !== 'AES-GCM') continue;
    const tag = e.dataLen + '->' + e.plainLen;
    decryptOkSizes[tag] = (decryptOkSizes[tag] || 0) + 1;
  }
  return {
    deriveCount: s.derive.length,
    decryptCount: s.decrypt.length,
    decryptOkCount: s.decryptOk.length,
    distinctDeriveCount: distinct.size,
    distinctDerives: Array.from(distinct.values()),
    decryptSizes,
    decryptOkSizes,
  };
})()
