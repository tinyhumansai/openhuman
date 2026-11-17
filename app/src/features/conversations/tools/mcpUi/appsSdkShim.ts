/**
 * `window.openai` for OpenAI Apps SDK widgets, implemented over the same
 * MCP Apps JSON-RPC channel the frame bridge speaks. Injected at the top of
 * the widget document, so it runs before the widget's own scripts.
 *
 * The documented subset: `toolInput`, `toolOutput`, `toolResponseMetadata`,
 * `widgetState`, `theme`, `locale`, `displayMode`, `maxHeight`, `callTool`,
 * `sendFollowUpMessage`, `openExternal`, `requestDisplayMode`,
 * `setWidgetState`, and the `openai:set_globals` event. Widget state lives in
 * memory only.
 */
export const APPS_SDK_SHIM_SOURCE = `(function () {
  'use strict';
  var host = window.parent;
  if (!host || host === window || window.openai) return;
  var nextId = 1;
  var pending = {};
  var globals = {
    toolInput: null,
    toolOutput: null,
    toolResponseMetadata: null,
    widgetState: null,
    theme: 'light',
    locale: 'en',
    displayMode: 'inline',
    maxHeight: 640,
    userAgent: { device: { type: 'desktop' }, capabilities: { hover: true, touch: false } },
    safeArea: { insets: { top: 0, bottom: 0, left: 0, right: 0 } }
  };
  function send(message) { host.postMessage(message, '*'); }
  function request(method, params) {
    return new Promise(function (resolve, reject) {
      var id = 'openai-' + nextId++;
      pending[id] = { resolve: resolve, reject: reject };
      send({ jsonrpc: '2.0', id: id, method: method, params: params || {} });
    });
  }
  function notify(method, params) { send({ jsonrpc: '2.0', method: method, params: params || {} }); }
  function setGlobals(patch) {
    for (var key in patch) {
      if (Object.prototype.hasOwnProperty.call(patch, key)) globals[key] = patch[key];
    }
    window.dispatchEvent(new CustomEvent('openai:set_globals', { detail: { globals: patch } }));
  }
  function applyContext(context) {
    if (!context || typeof context !== 'object') return;
    var patch = {};
    if (context.theme === 'dark' || context.theme === 'light') patch.theme = context.theme;
    if (typeof context.locale === 'string') patch.locale = context.locale;
    setGlobals(patch);
  }
  window.addEventListener('message', function (event) {
    if (event.source !== host) return;
    var data = event.data;
    if (!data || typeof data !== 'object' || data.jsonrpc !== '2.0') return;
    if (data.id != null && !data.method) {
      var waiter = pending[data.id];
      if (!waiter) return;
      delete pending[data.id];
      if (data.error) waiter.reject(new Error((data.error && data.error.message) || 'Request failed'));
      else waiter.resolve(data.result);
      return;
    }
    var params = data.params || {};
    if (data.method === 'ui/notifications/tool-input') {
      setGlobals({ toolInput: params.arguments == null ? null : params.arguments });
    } else if (data.method === 'ui/notifications/tool-result') {
      setGlobals({
        toolOutput: params.structuredContent == null ? null : params.structuredContent,
        toolResponseMetadata: params._meta == null ? null : params._meta
      });
    } else if (data.method === 'ui/notifications/host-context-changed') {
      applyContext(params);
    }
  });
  var api = {
    callTool: function (name, args) {
      return request('tools/call', { name: String(name), arguments: args || {} });
    },
    sendFollowUpMessage: function (options) {
      var text = String((options && options.prompt) || '');
      return request('ui/message', { role: 'user', content: [{ type: 'text', text: text }] }).then(function () {});
    },
    openExternal: function (options) {
      return request('ui/open-link', { url: String((options && options.href) || '') }).then(function () {});
    },
    requestDisplayMode: function () { return Promise.resolve({ mode: 'inline' }); },
    setWidgetState: function (state) { setGlobals({ widgetState: state }); return Promise.resolve(); },
    notifyIntrinsicHeight: function () { reportSize(); }
  };
  var openai = {};
  Object.keys(globals).forEach(function (key) {
    Object.defineProperty(openai, key, { enumerable: true, get: function () { return globals[key]; } });
  });
  Object.keys(api).forEach(function (key) {
    Object.defineProperty(openai, key, { enumerable: true, value: api[key] });
  });
  Object.defineProperty(window, 'openai', { value: openai, configurable: false, writable: false });
  var lastHeight = 0;
  function reportSize() {
    var height = Math.ceil(document.documentElement.scrollHeight);
    if (height === lastHeight) return;
    lastHeight = height;
    notify('ui/notifications/size-changed', { height: height });
  }
  if (typeof ResizeObserver === 'function') {
    new ResizeObserver(reportSize).observe(document.documentElement);
  }
  window.addEventListener('load', reportSize);
  request('ui/initialize', {
    protocolVersion: '2025-06-18',
    appInfo: { name: 'openai-apps-sdk-shim', version: '1' },
    appCapabilities: {}
  }).then(function (result) {
    applyContext(result && result.hostContext);
    notify('ui/notifications/initialized', {});
  }, function () {});
})();`;

/** `html` with `script` run first: placed right after `<head>`, or at the top. */
export function injectScript(html: string, script: string): string {
  const tag = `<script>${script.replace(/<\/script/gi, '<\\/script')}</script>`;
  const head = /<head(\s[^>]*)?>/i.exec(html);
  if (head) {
    const at = head.index + head[0].length;
    return html.slice(0, at) + tag + html.slice(at);
  }
  const doctype = /^\s*<!doctype[^>]*>/i.exec(html);
  if (doctype) {
    const at = doctype[0].length;
    return html.slice(0, at) + tag + html.slice(at);
  }
  return tag + html;
}
