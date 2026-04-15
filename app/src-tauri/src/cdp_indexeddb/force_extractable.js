// Force every CryptoKey produced by `crypto.subtle` to be extractable.
//
// WhatsApp Web creates AES-GCM keys with `extractable: false`, which makes
// `crypto.subtle.exportKey(...)` throw and stops us from reading the raw
// key bytes. We wrap the four entry points that can mint a CryptoKey and
// flip the `extractable` flag to `true` before delegating to the real
// implementation. The resulting keys behave identically except that we
// can now `exportKey('raw' | 'jwk', key)` them.
//
// Runs as an IIFE so it can be dropped unchanged into:
//   * the page (via `Page.addScriptToEvaluateOnNewDocument`)
//   * every worker (prepended into the wrapped worker blob in
//     worker_hook.js via the `/*__FORCE_EXTRACTABLE__*/` sentinel)
//
// Idempotent: guarded by `__ohForceExtractableInstalled`. A small ring
// buffer at `globalThis.__ohForcedExtractable` records the (algorithm,
// usages, origin) tuple for each forced key so we can confirm coverage
// from the scanner.
(() => {
  const g = (typeof globalThis !== 'undefined' ? globalThis : self);
  if (!g || !g.crypto || !g.crypto.subtle) return;
  if (g.__ohForceExtractableInstalled) return;
  g.__ohForceExtractableInstalled = true;

  const subtle = g.crypto.subtle;
  const log = [];
  g.__ohForcedExtractable = log;
  const record = (fn, algorithm, usages, wasExtractable) => {
    try {
      const algName = (algorithm && (algorithm.name || algorithm)) || null;
      log.push({
        fn,
        alg: typeof algName === 'string' ? algName : String(algName),
        usages: Array.isArray(usages) ? usages.slice(0, 16) : null,
        wasExtractable: !!wasExtractable,
        ts: Date.now(),
      });
      if (log.length > 256) log.shift();
    } catch (_) {}
  };

  // Bind once so recursive wrapping (e.g. worker importScripts loading a
  // bundle that redefines subtle) can't accidentally call our wrapper.
  const orig = {
    generateKey: subtle.generateKey.bind(subtle),
    importKey: subtle.importKey.bind(subtle),
    deriveKey: subtle.deriveKey.bind(subtle),
    unwrapKey: subtle.unwrapKey.bind(subtle),
  };

  subtle.generateKey = function (algorithm, extractable, keyUsages) {
    record('generateKey', algorithm, keyUsages, extractable);
    return orig.generateKey(algorithm, true, keyUsages);
  };

  subtle.importKey = function (format, keyData, algorithm, extractable, keyUsages) {
    record('importKey', algorithm, keyUsages, extractable);
    return orig.importKey(format, keyData, algorithm, true, keyUsages);
  };

  subtle.deriveKey = function (
    algorithm,
    baseKey,
    derivedKeyAlgorithm,
    extractable,
    keyUsages,
  ) {
    record('deriveKey', derivedKeyAlgorithm, keyUsages, extractable);
    return orig.deriveKey(algorithm, baseKey, derivedKeyAlgorithm, true, keyUsages);
  };

  subtle.unwrapKey = function (
    format,
    wrappedKey,
    unwrappingKey,
    unwrapAlgorithm,
    unwrappedKeyAlgorithm,
    extractable,
    keyUsages,
  ) {
    record('unwrapKey', unwrappedKeyAlgorithm, keyUsages, extractable);
    return orig.unwrapKey(
      format,
      wrappedKey,
      unwrappingKey,
      unwrapAlgorithm,
      unwrappedKeyAlgorithm,
      true,
      keyUsages,
    );
  };
})();
