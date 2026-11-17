/**
 * Previous/next buttons for horizontally scrolling regions of a widget, and
 * vertical wheel mapped onto them. Injected into every widget document; it
 * never moves the widget's own nodes, it overlays fixed-position buttons that
 * follow each scroller.
 *
 * Only the outermost scrolling row of a region gets buttons, never one
 * narrower than `MIN_WIDTH`, and none while the widget shows its own
 * navigation next to the row.
 */
export const SCROLL_AFFORDANCE_SOURCE = `(function () {
  'use strict';
  if (window.__ohScrollAffordance) return;
  window.__ohScrollAffordance = true;
  var PREFIX = 'ohsa-';
  var SIZE = 28;
  var SLACK = 4;
  var MIN_WIDTH = 240;
  var EDGE = 72;
  var HEADER = 64;
  var ANCESTOR_LEVELS = 3;
  var MAX_CONTROLS = 600;
  var NAV_WORDS = ['next', 'prev', 'previous', 'forward', 'back', 'scroll', 'arrow', 'chevron',
    'left', 'right', 'carousel', 'slide'];
  var tracked = [];
  var layer = null;
  var timer = null;
  var CHEVRON_LEFT = '<svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" focusable="false"><path d="M10 3 5 8l5 5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>';
  var CHEVRON_RIGHT = '<svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true" focusable="false"><path d="m6 3 5 5-5 5" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>';

  function overflowing(el) {
    return el.scrollWidth > el.clientWidth + SLACK;
  }
  function inLayer(node) {
    return !!layer && layer.contains(node);
  }
  function isCandidate(el) {
    if (!el || el.nodeType !== 1 || inLayer(el)) return false;
    var x = window.getComputedStyle(el).overflowX;
    if ((x !== 'auto' && x !== 'scroll') || !overflowing(el)) return false;
    return el.getBoundingClientRect().width >= MIN_WIDTH;
  }
  function atStart(el) {
    return el.scrollLeft <= 0;
  }
  function atEnd(el) {
    return el.scrollLeft + el.clientWidth >= el.scrollWidth - 1;
  }
  function clickable(node) {
    var tag = node.tagName;
    if (tag === 'BUTTON' || tag === 'A') return true;
    if (node.getAttribute('role') === 'button') return true;
    return window.getComputedStyle(node).cursor === 'pointer';
  }
  function looksLikeNav(node) {
    var text = [node.getAttribute('aria-label'), node.getAttribute('title'), node.getAttribute('class')]
      .join(' ')
      .replace(/([a-z])([A-Z])/g, '$1 $2')
      .toLowerCase()
      .split(/[^a-z]+/);
    for (var i = 0; i < text.length; i++) {
      if (NAV_WORDS.indexOf(text[i]) !== -1) return true;
    }
    return false;
  }
  function nearEdge(control, row) {
    if (control.bottom <= row.top || control.top >= row.bottom) return false;
    var nearLeft = control.left < row.left + EDGE && control.right > row.left - EDGE;
    var nearRight = control.right > row.right - EDGE && control.left < row.right + EDGE;
    return nearLeft || nearRight;
  }
  function nearRow(control, row) {
    if (control.bottom <= row.top - HEADER || control.top >= row.bottom + SLACK * 4) return false;
    return control.right > row.left - EDGE && control.left < row.right + EDGE;
  }
  function hasNativeNav(el) {
    var top = el;
    for (var level = 0; level < ANCESTOR_LEVELS && top.parentElement; level++) {
      top = top.parentElement;
    }
    if (top === el) return false;
    var row = el.getBoundingClientRect();
    var nodes = top.getElementsByTagName('*');
    var limit = Math.min(nodes.length, MAX_CONTROLS);
    for (var i = 0; i < limit; i++) {
      var node = nodes[i];
      if (node === el || el.contains(node) || inLayer(node) || !clickable(node)) continue;
      var rect = node.getBoundingClientRect();
      if (rect.width <= 0 || rect.height <= 0) continue;
      if (nearEdge(rect, row) || (looksLikeNav(node) && nearRow(rect, row))) return true;
    }
    return false;
  }
  function ensureLayer() {
    if (layer && layer.isConnected) return layer;
    if (!document.body) return null;
    layer = document.createElement('div');
    layer.className = PREFIX + 'layer';
    layer.style.cssText = 'position:fixed;left:0;top:0;width:0;height:0;z-index:2147483000;';
    document.body.appendChild(layer);
    return layer;
  }
  function makeButton(label, icon, direction, el) {
    var button = document.createElement('button');
    button.type = 'button';
    button.className = PREFIX + 'button';
    button.setAttribute('aria-label', label);
    button.innerHTML = icon;
    button.style.cssText = 'position:fixed;display:none;align-items:center;justify-content:center;' +
      'width:' + SIZE + 'px;height:' + SIZE + 'px;padding:0;margin:0;' +
      'border:1px solid rgba(0,0,0,0.14);background:rgba(255,255,255,0.94);color:#1f2328;' +
      'box-shadow:0 1px 4px rgba(0,0,0,0.22);cursor:pointer;font:inherit;line-height:0;';
    button.style.borderRadius = '50%';
    button.addEventListener('click', function () {
      var amount = Math.max(1, Math.round(el.clientWidth * 0.8)) * direction;
      if (typeof el.scrollBy === 'function') {
        el.scrollBy({ left: amount, behavior: 'smooth' });
      } else {
        el.scrollLeft += amount;
      }
    });
    return button;
  }
  function place(entry) {
    if (entry.suppressed) return;
    var el = entry.el;
    var rect = el.getBoundingClientRect();
    var visible = el.isConnected && overflowing(el) && rect.width > SIZE * 2 && rect.height > 0;
    var top = Math.round(rect.top + rect.height / 2 - SIZE / 2);
    entry.left.style.top = top + 'px';
    entry.left.style.left = Math.round(rect.left + 4) + 'px';
    entry.right.style.top = top + 'px';
    entry.right.style.left = Math.round(rect.right - SIZE - 4) + 'px';
    entry.left.style.display = visible && !atStart(el) ? 'flex' : 'none';
    entry.right.style.display = visible && !atEnd(el) ? 'flex' : 'none';
  }
  function setSuppressed(entry, suppressed) {
    if (entry.suppressed === suppressed) return;
    entry.suppressed = suppressed;
    if (suppressed) {
      entry.left.remove();
      entry.right.remove();
      return;
    }
    var host = ensureLayer();
    if (!host) return;
    host.appendChild(entry.left);
    host.appendChild(entry.right);
  }
  function untrack(entry) {
    entry.left.remove();
    entry.right.remove();
    entry.el.removeEventListener('scroll', entry.onScroll);
    entry.el.removeEventListener('wheel', entry.onWheel);
    if (entry.observer) entry.observer.disconnect();
    tracked.splice(tracked.indexOf(entry), 1);
  }
  function track(el) {
    if (!ensureLayer()) return;
    var entry = {
      el: el,
      suppressed: true,
      left: makeButton('Scroll left', CHEVRON_LEFT, -1, el),
      right: makeButton('Scroll right', CHEVRON_RIGHT, 1, el),
      observer: null
    };
    entry.onScroll = function () { place(entry); };
    entry.onWheel = function (event) {
      if (entry.suppressed) return;
      if (Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
      if (el.scrollHeight > el.clientHeight + 1) return;
      if (event.deltaY > 0 ? atEnd(el) : atStart(el)) return;
      el.scrollLeft += event.deltaY;
      event.preventDefault();
    };
    el.addEventListener('scroll', entry.onScroll, { passive: true });
    el.addEventListener('wheel', entry.onWheel, { passive: false });
    if (typeof ResizeObserver === 'function') {
      entry.observer = new ResizeObserver(entry.onScroll);
      entry.observer.observe(el);
    }
    tracked.push(entry);
  }
  function update() {
    for (var i = tracked.length - 1; i >= 0; i--) {
      if (!tracked[i].el.isConnected) {
        untrack(tracked[i]);
        continue;
      }
      place(tracked[i]);
    }
  }
  function adopt(el) {
    for (var i = tracked.length - 1; i >= 0; i--) {
      var other = tracked[i].el;
      if (other === el || other.contains(el)) return;
      if (el.contains(other)) untrack(tracked[i]);
    }
    track(el);
  }
  function scan() {
    timer = null;
    if (!document.body) return;
    var all = document.body.getElementsByTagName('*');
    var found = [];
    for (var i = 0; i < all.length; i++) {
      if (isCandidate(all[i])) found.push(all[i]);
    }
    for (var j = 0; j < found.length; j++) adopt(found[j]);
    for (var k = 0; k < tracked.length; k++) {
      setSuppressed(tracked[k], hasNativeNav(tracked[k].el));
    }
    update();
  }
  function schedule() {
    if (timer !== null) return;
    timer = setTimeout(scan, 120);
  }
  document.addEventListener('DOMContentLoaded', schedule);
  window.addEventListener('load', schedule);
  window.addEventListener('resize', function () { update(); schedule(); });
  document.addEventListener('scroll', update, { passive: true, capture: true });
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
