// Zoom recipe.
//
// V1 scope:
//   * Intercept Zoom native-app deep-link schemes (`zoomus://`, `zoommtg://`)
//     and rewrite them to Zoom WebClient URLs so calls open inside the
//     embedded webview instead of bouncing out to the system browser
//     with ERR_UNKNOWN_URL_SCHEME.
//   * Log bootstrap + reserve event-name namespace for future call-lifecycle
//     and caption capture (zoom_call_started/zoom_captions/zoom_call_ended).
//
// Event kinds reserved (not yet emitted — caption DOM scraping is a follow-up
// that needs live-call investigation):
//   zoom_call_started  { meetingId, url, startedAt }
//   zoom_captions      { meetingId, captions:[{speaker,text}], ts }
//   zoom_call_ended    { meetingId, endedAt, reason }
(function (api) {
  if (!api) return;
  api.log('info', '[zoom-recipe] loaded (deep-link rewrite active; captions TBD)');

  var DEEP_LINK_RE = /^(zoomus|zoommtg):\/\//i;

  // Rewrite `zoomus://zoom.us/join?action=join&confno=<id>&pwd=<x>&...`
  // to `https://app.zoom.us/wc/join/<id>?pwd=<x>&...`. Falls back to the
  // WebClient home page if no confno is present.
  function rewriteDeepLink(url) {
    try {
      var marker = url.indexOf('://');
      if (marker < 0) return null;
      var rest = url.slice(marker + 3);
      var queryIdx = rest.indexOf('?');
      var query = queryIdx >= 0 ? rest.slice(queryIdx + 1) : '';
      var params = new URLSearchParams(query);
      var confno = params.get('confno') || '';
      if (!confno) return 'https://app.zoom.us/wc/home';
      var pwd = params.get('pwd') || params.get('tk') || '';
      var webUrl = 'https://app.zoom.us/wc/join/' + encodeURIComponent(confno);
      if (pwd) webUrl += '?pwd=' + encodeURIComponent(pwd);
      return webUrl;
    } catch (_) {
      return null;
    }
  }

  // Intercept left-clicks on <a href="zoomus://..."> before the browser tries
  // to navigate. Capture phase so we beat any page-level handlers.
  document.addEventListener(
    'click',
    function (ev) {
      try {
        var target = ev.target;
        while (target && target !== document) {
          if (target.tagName === 'A' && target.href && DEEP_LINK_RE.test(target.href)) {
            var rewritten = rewriteDeepLink(target.href);
            if (rewritten) {
              ev.preventDefault();
              ev.stopPropagation();
              api.log('info', '[zoom-recipe] rewrote click: ' + target.href + ' -> ' + rewritten);
              window.location.href = rewritten;
            }
            return;
          }
          target = target.parentNode;
        }
      } catch (_) {}
    },
    true
  );

  // Zoom's join page fires `window.location.href = 'zoomus://...'` from an
  // inline script after a short countdown. Proxy window.location writes so
  // we catch and rewrite those assignments.
  try {
    var origAssign = window.location.assign.bind(window.location);
    var origReplace = window.location.replace.bind(window.location);
    window.location.assign = function (url) {
      if (typeof url === 'string' && DEEP_LINK_RE.test(url)) {
        var rewritten = rewriteDeepLink(url);
        if (rewritten) {
          api.log('info', '[zoom-recipe] rewrote assign: ' + url + ' -> ' + rewritten);
          return origAssign(rewritten);
        }
      }
      return origAssign(url);
    };
    window.location.replace = function (url) {
      if (typeof url === 'string' && DEEP_LINK_RE.test(url)) {
        var rewritten = rewriteDeepLink(url);
        if (rewritten) {
          api.log('info', '[zoom-recipe] rewrote replace: ' + url + ' -> ' + rewritten);
          return origReplace(rewritten);
        }
      }
      return origReplace(url);
    };
  } catch (_) {}

  // Catch-all: if a script bypasses the overrides above by using
  // `location.href = '...'` directly (which webkit may treat as a setter on
  // the location object itself), monkey-patch the setter via a proxy on the
  // ancestor Location prototype.
  try {
    var proto = Object.getPrototypeOf(window.location);
    var desc = Object.getOwnPropertyDescriptor(proto, 'href');
    if (desc && desc.set) {
      var origHrefSet = desc.set;
      Object.defineProperty(window.location, 'href', {
        configurable: true,
        get: desc.get,
        set: function (url) {
          if (typeof url === 'string' && DEEP_LINK_RE.test(url)) {
            var rewritten = rewriteDeepLink(url);
            if (rewritten) {
              api.log('info', '[zoom-recipe] rewrote href-set: ' + url + ' -> ' + rewritten);
              return origHrefSet.call(window.location, rewritten);
            }
          }
          return origHrefSet.call(window.location, url);
        },
      });
    }
  } catch (_) {}

  // Intercept window.open for popup-based deep-link launches.
  try {
    var origOpen = window.open.bind(window);
    window.open = function (url, target, features) {
      if (typeof url === 'string' && DEEP_LINK_RE.test(url)) {
        var rewritten = rewriteDeepLink(url);
        if (rewritten) {
          api.log('info', '[zoom-recipe] rewrote window.open: ' + url + ' -> ' + rewritten);
          return origOpen(rewritten, target, features);
        }
      }
      return origOpen(url, target, features);
    };
  } catch (_) {}
})(window.__openhumanRecipe);
