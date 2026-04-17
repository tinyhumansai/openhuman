// WhatsApp Web recipe.
//
// IndexedDB reading + decryption happens on the Rust side via CDP
// (`Runtime.evaluate` against the embedded CEF instance) — see
// `app/src-tauri/src/cdp_indexeddb/`. Browser notifications are
// intercepted natively by the cef-helper render-process patch in our
// tauri-cef fork. This file is intentionally tiny: the recipe runtime
// only needs to confirm the page booted.
(function (api) {
  if (!api) return;
  api.log('info', '[whatsapp-recipe] starting (CDP-only; no DOM scraping)');
})(window.__openhumanRecipe);
