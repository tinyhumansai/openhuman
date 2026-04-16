// BrowserScan bot-detection recipe (dev only).
//
// We don't scrape or post anything back — the page itself renders the
// detection verdict. The recipe exists purely so the provider slots into
// the same lifecycle as the other webview accounts and we get a boot log
// line confirming the page loaded under our UA/CEF stack.
(function (api) {
  if (!api) return;
  api.log('info', '[browserscan-recipe] page booted; read the verdict on-screen');
})(window.__openhumanRecipe);
