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

  // navigator.permissions — return "granted" for notifications queries.
  // Simple property assignment on Blink platform objects (like Permissions)
  // is silently ignored; Object.defineProperty on the navigator itself works
  // because navigator exposes configurable getters (same mechanism used above
  // for navigator.userAgent).
  try {
    var _rp = navigator && navigator.permissions;
    if (_rp && typeof _rp.query === 'function') {
      var _rq = _rp.query.bind(_rp);
      var _fp = {
        query: function (d) {
          if (d && (d.name === 'notifications' || d.name === 'push')) {
            return Promise.resolve({ state: 'granted', onchange: null });
          }
          return _rq(d);
        },
      };
      Object.defineProperty(navigator, 'permissions', {
        get: function () { return _fp; },
        configurable: true,
      });
    }
  } catch (_) {}

  // PushManager permission checks — Slack relies on these and can keep showing
  // the notification permission banner when this still returns "prompt".
  try {
    if (
      window.PushManager &&
      window.PushManager.prototype &&
      typeof window.PushManager.prototype.permissionState === 'function'
    ) {
      var __ohPushPermissionState = function () {
        return Promise.resolve('granted');
      };
      Object.defineProperty(window.PushManager.prototype, 'permissionState', {
        get: function () { return __ohPushPermissionState; },
        configurable: true,
      });
    }
  } catch (_) {}

  // Notification.permission — ensure it reads "granted" without replacing the
  // constructor. Under CEF the native notification shim needs to own
  // `new Notification(...)` so it can forward payloads over the renderer →
  // browser process bridge. Overwriting the constructor here breaks that path.
  try {
    if (typeof window.Notification === 'function') {
      try {
        var __nativeFnToString = Function.prototype.toString;
        var __wrappedRequest = function () {
          return Promise.resolve('granted');
        };
        Function.prototype.toString = new Proxy(__nativeFnToString, {
          apply: function (target, thisArg, args) {
            if (thisArg === __wrappedRequest) {
              return 'function requestPermission() { [native code] }';
            }
            return Reflect.apply(target, thisArg, args);
          },
        });
        window.Notification.requestPermission = __wrappedRequest;
      } catch (_) {
        window.Notification.requestPermission = function () {
          return Promise.resolve('granted');
        };
      }
      Object.defineProperty(window.Notification, 'permission', {
        get: function () { return 'granted'; },
        configurable: true,
      });
    }
  } catch (_) {}
  window.__OH_NOTIF_SHIM = true;
})();
