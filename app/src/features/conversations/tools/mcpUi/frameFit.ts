/** A reported width at or above this asks for the whole column. */
export const FILL_WIDTH = 100000;

/**
 * The host script that makes a widget sit in the chat like native content: a
 * transparent page canvas in the host's color scheme, and the width its
 * content actually uses, reported as `ui/notifications/size-changed` with
 * `fit: true`.
 *
 * The width is measured from the content, not the viewport: a box narrower
 * than its container counts at its own width (a centred box symmetrically), a
 * full-width box with a `max-width` counts at that, and a full-width wrapper
 * is descended into. Running text, media, or a horizontally scrolling row
 * asks for the whole column, so narrowing the frame cannot narrow what is
 * measured.
 */
export function frameFitSource(theme: 'light' | 'dark'): string {
  return FRAME_FIT_TEMPLATE.replace('__THEME__', theme).replace('__FILL__', String(FILL_WIDTH));
}

const FRAME_FIT_TEMPLATE = `(function () {
  'use strict';
  if (window.__ohFrameFit) return;
  window.__ohFrameFit = true;
  var THEME = '__THEME__';
  var FILL = __FILL__;
  var MIN_ROW = 240;
  var STEP = 8;
  var MAX_DEPTH = 8;
  var LEAVES = ['IMG', 'VIDEO', 'CANVAS', 'SVG', 'svg', 'TABLE', 'IFRAME', 'PRE', 'TEXTAREA'];
  var host = window.parent;
  var style = null;
  var last = 0;
  var timer = null;

  function applyStyle() {
    var parent = document.head || document.documentElement;
    if (!parent) return;
    if (!style) {
      style = document.createElement('style');
      style.setAttribute('data-oh-host', '');
      style.textContent = 'html,body{background:transparent!important;}' +
        ':root{color-scheme:' + THEME + '!important;}';
    }
    if (style.parentNode !== parent || parent.lastChild !== style) parent.appendChild(style);
  }
  function px(value) {
    var n = parseFloat(value);
    return isFinite(n) ? n : 0;
  }
  function hasScrollRow() {
    if (!document.body) return false;
    var all = document.body.getElementsByTagName('*');
    for (var i = 0; i < all.length; i++) {
      var el = all[i];
      var x = window.getComputedStyle(el).overflowX;
      if ((x === 'auto' || x === 'scroll') && el.scrollWidth > el.clientWidth + 4 &&
          el.getBoundingClientRect().width >= MIN_ROW) {
        return true;
      }
    }
    return false;
  }
  function hasText(el) {
    for (var node = el.firstChild; node; node = node.nextSibling) {
      if (node.nodeType === 3 && node.nodeValue.trim()) return true;
    }
    return false;
  }
  function contentBox(el) {
    var rect = el.getBoundingClientRect();
    var cs = window.getComputedStyle(el);
    var left = rect.left + px(cs.paddingLeft) + px(cs.borderLeftWidth);
    var right = rect.right - px(cs.paddingRight) - px(cs.borderRightWidth);
    return { left: left, width: Math.max(0, right - left) };
  }
  function extent(el, container, depth) {
    var rect = el.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return 0;
    var cs = window.getComputedStyle(el);
    if (cs.position === 'fixed' || cs.position === 'absolute') return 0;
    var lead = container.left;
    var trail = Math.max(0, document.documentElement.clientWidth - container.left - container.width);
    if (rect.width < container.width - 1) {
      var ml = px(cs.marginLeft);
      var centred = ml > 0 && Math.abs(ml - px(cs.marginRight)) < 1;
      return (centred ? lead + rect.width : rect.right) + trail;
    }
    var maxWidth = cs.maxWidth === 'none' ? 0 : px(cs.maxWidth);
    if (maxWidth > 0) {
      if (cs.boxSizing.slice(0, 6) !== 'border') {
        maxWidth += px(cs.paddingLeft) + px(cs.paddingRight) +
          px(cs.borderLeftWidth) + px(cs.borderRightWidth);
      }
      return lead + maxWidth + trail;
    }
    if (hasText(el) || LEAVES.indexOf(el.tagName) !== -1 || depth >= MAX_DEPTH) return FILL;
    var inner = contentBox(el);
    var widest = 0;
    for (var child = el.firstElementChild; child; child = child.nextElementSibling) {
      widest = Math.max(widest, extent(child, inner, depth + 1));
    }
    return widest > 0 ? widest : FILL;
  }
  function measure() {
    if (!document.body) return 0;
    if (hasScrollRow()) return FILL;
    var root = document.documentElement;
    if (root.scrollWidth > root.clientWidth + 1) return FILL;
    var body = contentBox(document.body);
    var widest = 0;
    for (var child = document.body.firstElementChild; child; child = child.nextElementSibling) {
      widest = Math.max(widest, extent(child, body, 0));
    }
    if (widest === 0 && hasText(document.body)) return FILL;
    return widest;
  }
  function report() {
    timer = null;
    applyStyle();
    var width = Math.ceil(measure());
    if (!width || (last && Math.abs(width - last) < STEP)) return;
    last = width;
    if (!host || host === window) return;
    host.postMessage({
      jsonrpc: '2.0',
      method: 'ui/notifications/size-changed',
      params: { width: width, height: Math.ceil(document.documentElement.scrollHeight), fit: true }
    }, '*');
  }
  function schedule() {
    if (timer !== null) return;
    timer = setTimeout(report, 150);
  }
  applyStyle();
  document.addEventListener('DOMContentLoaded', schedule);
  window.addEventListener('load', schedule);
  window.addEventListener('resize', schedule);
  if (typeof MutationObserver === 'function') {
    new MutationObserver(schedule).observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ['class', 'hidden']
    });
  }
  if (typeof ResizeObserver === 'function') {
    new ResizeObserver(schedule).observe(document.documentElement);
  }
  schedule();
})();`;
