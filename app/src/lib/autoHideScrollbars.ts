import debugFactory from 'debug';

const log = debugFactory('scrollbars');

/** How long a pane keeps its scrollbar visible after the last scroll event. */
export const SCROLL_IDLE_MS = 900;

/** Attribute the stylesheet keys the visible-thumb rule off (see index.css). */
export const SCROLLING_ATTR = 'data-scrolling';

/**
 * Reveal a pane's scrollbar while it is scrolling, then fade it out — the macOS
 * overlay behaviour, which CEF does not provide (it paints classic
 * always-visible scrollbars).
 *
 * CSS has no "is scrolling" state, so this stamps {@link SCROLLING_ATTR} on the
 * element that scrolled and clears it once that element has been idle for
 * {@link SCROLL_IDLE_MS}. Everything visual lives in the stylesheet; this only
 * owns the flag.
 *
 * Listens in the **capture** phase on the document because `scroll` does not
 * bubble — a single listener therefore covers every pane in the app, including
 * ones mounted later, with no per-component wiring.
 *
 * @param doc document to instrument (injectable for tests)
 * @returns disposer that removes the listener and clears any pending timers
 */
export function installAutoHideScrollbars(doc: Document = document): () => void {
  const timers = new Map<Element, ReturnType<typeof setTimeout>>();

  const clear = (el: Element) => {
    const pending = timers.get(el);
    if (pending !== undefined) clearTimeout(pending);
    timers.delete(el);
    el.removeAttribute(SCROLLING_ATTR);
  };

  const onScroll = (event: Event) => {
    // A document-level scroll reports `document` as the target; the element that
    // actually scrolls (and owns the scrollbar) is the root element.
    const target = event.target;
    const el = target instanceof Element ? target : doc.documentElement;
    if (!el) return;

    if (!timers.has(el)) {
      log('scroll start: <%s>', el.tagName.toLowerCase());
    } else {
      // Continued scrolling: restart the idle countdown. Deliberately silent —
      // `scroll` fires per animation frame, so a diagnostic here would emit
      // ~60 lines/second per pane and bury the start/idle transitions that
      // actually carry information. The pair of those two logs already bounds
      // every scroll gesture, which is what an investigation needs.
      clearTimeout(timers.get(el)!);
    }

    el.setAttribute(SCROLLING_ATTR, '');
    timers.set(
      el,
      setTimeout(() => {
        log('scroll idle: <%s>', el.tagName.toLowerCase());
        clear(el);
      }, SCROLL_IDLE_MS)
    );
  };

  doc.addEventListener('scroll', onScroll, { capture: true, passive: true });
  log('installed (idle=%dms)', SCROLL_IDLE_MS);

  return () => {
    doc.removeEventListener('scroll', onScroll, { capture: true });
    for (const el of [...timers.keys()]) clear(el);
    log('disposed');
  };
}
