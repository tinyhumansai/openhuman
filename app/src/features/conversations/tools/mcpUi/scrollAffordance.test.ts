import { describe, expect, it, vi } from 'vitest';

import { SCROLL_AFFORDANCE_SOURCE } from './scrollAffordance';

interface Box {
  scrollWidth: number;
  clientWidth: number;
  scrollHeight: number;
  clientHeight: number;
  scrollLeft: number;
}

interface Rect {
  left: number;
  top: number;
  width: number;
  height: number;
}

const ROW_HTML =
  '<div id="row" style="overflow-x:auto"><div>items</div></div>' +
  '<div id="clip" style="overflow-x:hidden"><div>wide</div></div>';

function setRect(el: Element, rect: () => Rect) {
  Object.defineProperty(el, 'getBoundingClientRect', {
    configurable: true,
    value: () => {
      const { left, top, width, height } = rect();
      return { left, top, width, height, right: left + width, bottom: top + height };
    },
  });
}

function fakeLayout(el: Element, box: Box, left = 10) {
  for (const key of Object.keys(box) as Array<keyof Box>) {
    Object.defineProperty(el, key, {
      configurable: true,
      get: () => box[key],
      set: (value: number) => {
        box[key] = value;
      },
    });
  }
  setRect(el, () => ({ left, top: 20, width: box.clientWidth, height: 100 }));
}

const overflowingBox = (clientWidth = 300): Box => ({
  scrollWidth: 1000,
  clientWidth,
  scrollHeight: 100,
  clientHeight: 100,
  scrollLeft: 0,
});

const settle = () => new Promise(resolve => setTimeout(resolve, 200));

async function mount(html = ROW_HTML, layout?: (document: Document) => void) {
  const frame = window.document.createElement('iframe');
  window.document.body.appendChild(frame);
  const win = frame.contentWindow as Window & typeof globalThis;
  const document = win.document;
  document.body.innerHTML = html;
  const row = document.getElementById('row')!;
  const box = overflowingBox();
  fakeLayout(row, box);
  const clip = document.getElementById('clip');
  if (clip) fakeLayout(clip, overflowingBox());
  layout?.(document);
  const scrollBy = vi.fn();
  Object.defineProperty(row, 'scrollBy', { value: scrollBy });
  new Function('window', 'document', SCROLL_AFFORDANCE_SOURCE)(win, document);
  await settle();
  const button = (label: string) =>
    document.querySelector(`button[aria-label="${label}"]`) as HTMLButtonElement | null;
  const scrolled = () => row.dispatchEvent(new win.Event('scroll'));
  const wheel = (deltaY: number) => {
    const event = new win.WheelEvent('wheel', { deltaY, cancelable: true });
    row.dispatchEvent(event);
    return event.defaultPrevented;
  };
  return { win, document, row, box, scrollBy, button, scrolled, wheel };
}

describe('scroll affordance', () => {
  it('adds buttons only to horizontally scrolling regions', async () => {
    const { document, button } = await mount();
    expect(document.querySelectorAll('button')).toHaveLength(2);
    expect(button('Scroll left')!.style.display).toBe('none');
    expect(button('Scroll right')!.style.display).toBe('flex');
    expect(button('Scroll right')!.type).toBe('button');
  });

  it('scrolls by most of the visible width', async () => {
    const { button, scrollBy } = await mount();
    button('Scroll right')!.click();
    expect(scrollBy).toHaveBeenCalledWith({ left: 240, behavior: 'smooth' });
    button('Scroll left')!.click();
    expect(scrollBy).toHaveBeenLastCalledWith({ left: -240, behavior: 'smooth' });
  });

  it('hides each arrow at its end and both once nothing overflows', async () => {
    const { box, button, scrolled } = await mount();
    box.scrollLeft = 700;
    scrolled();
    expect(button('Scroll left')!.style.display).toBe('flex');
    expect(button('Scroll right')!.style.display).toBe('none');

    box.scrollWidth = 302;
    box.scrollLeft = 0;
    scrolled();
    expect(button('Scroll left')!.style.display).toBe('none');
    expect(button('Scroll right')!.style.display).toBe('none');
  });

  it('turns vertical wheel into horizontal scroll until the end', async () => {
    const { box, wheel } = await mount();
    expect(wheel(100)).toBe(true);
    expect(box.scrollLeft).toBe(100);

    box.scrollLeft = 700;
    expect(wheel(100)).toBe(false);
    expect(box.scrollLeft).toBe(700);

    box.scrollLeft = 0;
    expect(wheel(-100)).toBe(false);
  });

  it('stands down when the widget has its own button at the row edge', async () => {
    const { document, wheel, box } = await mount(
      '<section><div id="row" style="overflow-x:auto"><div>items</div></div>' +
        '<div id="native">&gt;</div></section>',
      doc => {
        const native = doc.getElementById('native')!;
        native.style.cursor = 'pointer';
        setRect(native, () => ({ left: 290, top: 50, width: 32, height: 32 }));
      }
    );
    expect(document.querySelectorAll('button')).toHaveLength(0);
    expect(wheel(100)).toBe(false);
    expect(box.scrollLeft).toBe(0);
  });

  it('removes its buttons once native navigation appears', async () => {
    const { document, win } = await mount(
      '<section><div><div id="row" style="overflow-x:auto"><div>items</div></div></div></section>'
    );
    expect(document.querySelectorAll('button')).toHaveLength(2);

    const native = document.createElement('button');
    native.setAttribute('aria-label', 'Next slide');
    setRect(native, () => ({ left: 260, top: 0, width: 24, height: 24 }));
    document.querySelector('section')!.appendChild(native);
    await new Promise(resolve => win.setTimeout(resolve, 200));

    expect(document.querySelector('button[aria-label="Scroll right"]')).toBeNull();
    expect(document.querySelectorAll('button')).toHaveLength(1);
  });

  it('ignores navigation controls that belong to another part of the page', async () => {
    const { document } = await mount(
      '<section><div id="row" style="overflow-x:auto"><div>items</div></div>' +
        '<button id="far" aria-label="Next page">Next</button></section>',
      doc =>
        setRect(doc.getElementById('far')!, () => ({ left: 10, top: 600, width: 60, height: 24 }))
    );
    expect(document.querySelectorAll('button[aria-label^="Scroll"]')).toHaveLength(2);
  });

  it('ignores controls whose names only contain a navigation word', async () => {
    const { document } = await mount(
      '<section><div id="row" style="overflow-x:auto"><div>items</div></div>' +
        '<button id="add" class="bg-background feedback">Add</button></section>',
      doc =>
        setRect(doc.getElementById('add')!, () => ({ left: 120, top: 0, width: 40, height: 20 }))
    );
    expect(document.querySelectorAll('button[aria-label^="Scroll"]')).toHaveLength(2);
  });

  it('gives buttons to the outer row only, and none to a narrow scroller', async () => {
    const { document, scrollBy, button } = await mount(
      '<div id="row" style="overflow-x:auto"><div id="inner" style="overflow-x:auto">' +
        '<div>gallery</div></div></div>' +
        '<div id="small" style="overflow-x:auto"><div>chips</div></div>',
      doc => {
        fakeLayout(doc.getElementById('inner')!, overflowingBox(260), 20);
        fakeLayout(doc.getElementById('small')!, overflowingBox(200), 10);
      }
    );
    expect(document.querySelectorAll('button')).toHaveLength(2);
    button('Scroll right')!.click();
    expect(scrollBy).toHaveBeenCalledWith({ left: 240, behavior: 'smooth' });
  });
});
