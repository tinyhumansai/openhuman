// Aggressive Chrome feature-shim for services that blocklist non-Chromium
// WebViews by fingerprinting the navigator. Runs BEFORE the page's JS —
// Tauri injects this via initialization_script. Gated per-provider in
// mod.rs (see provider_ua_spoof).
//
// Covers the checks Slack / Google / LinkedIn are known to run:
//   - navigator.userAgent / vendor / platform
//   - navigator.userAgentData (client-hints API — WKWebView / WebKitGTK
//     don't expose this, and "real Chrome only" checks rely on it)
//   - navigator.brave absence, window.chrome presence
//
// We can't defeat deep behaviour-based detection (WebGL fingerprints,
// CSS feature probes, …) from pure JS, but this is enough to get past
// the "browser not supported" landing page on the providers listed.
(function () {
  const CHROME_MAJOR = '124';
  const CHROME_FULL = '124.0.6367.118';
  const UA =
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 ' +
    '(KHTML, like Gecko) Chrome/' +
    CHROME_FULL +
    ' Safari/537.36';

  function define(target, name, value) {
    try {
      Object.defineProperty(target, name, {
        get: function () { return value; },
        configurable: true,
      });
    } catch (_) {
      // Property not reconfigurable on this platform — swallow.
    }
  }

  define(navigator, 'userAgent', UA);
  define(navigator, 'vendor', 'Google Inc.');
  define(navigator, 'platform', 'MacIntel');
  define(navigator, 'appVersion',
    '5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/' +
    CHROME_FULL + ' Safari/537.36');

  // navigator.userAgentData — Client Hints API. Slack's unsupported-browser
  // check reads `.brands` for "Chromium" / "Google Chrome".
  try {
    const brands = [
      { brand: 'Chromium', version: CHROME_MAJOR },
      { brand: 'Google Chrome', version: CHROME_MAJOR },
      { brand: 'Not-A.Brand', version: '99' },
    ];
    const fullBrands = [
      { brand: 'Chromium', version: CHROME_FULL },
      { brand: 'Google Chrome', version: CHROME_FULL },
      { brand: 'Not-A.Brand', version: '99.0.0.0' },
    ];
    const uaData = {
      brands: brands,
      mobile: false,
      platform: 'macOS',
      getHighEntropyValues: function (hints) {
        return Promise.resolve({
          architecture: 'x86',
          bitness: '64',
          brands: brands,
          fullVersionList: fullBrands,
          mobile: false,
          model: '',
          platform: 'macOS',
          platformVersion: '14.0.0',
          uaFullVersion: CHROME_FULL,
          wow64: false,
        });
      },
      toJSON: function () {
        return { brands: brands, mobile: false, platform: 'macOS' };
      },
    };
    Object.defineProperty(navigator, 'userAgentData', {
      get: function () { return uaData; },
      configurable: true,
    });
  } catch (_) {}

  // window.chrome — Chromium exposes this as an object (with .runtime etc.).
  // WKWebView doesn't, and some detection scripts check for it.
  try {
    if (!window.chrome) {
      window.chrome = {
        runtime: {},
        loadTimes: function () { return {}; },
        csi: function () { return {}; },
        app: { isInstalled: false },
      };
    }
  } catch (_) {}

  // Some fingerprinters look for Safari-specific APIs and reject if found.
  try {
    delete window.safari;
  } catch (_) {}
})();
