import { afterEach, describe, expect, it, vi } from 'vitest';

import { FILL_WIDTH, frameFitSource } from './frameFit';

interface Rect {
  left: number;
  width: number;
}

function setRect(el: Element, rect: () => Rect) {
  Object.defineProperty(el, 'getBoundingClientRect', {
    configurable: true,
    value: () => {
      const { left, width } = rect();
      return { left, top: 0, width, height: 100, right: left + width, bottom: 100 };
    },
  });
}

const settle = () => new Promise(resolve => setTimeout(resolve, 200));

let posted: Array<Record<string, unknown>> = [];

afterEach(() => {
  vi.restoreAllMocks();
  posted = [];
  for (const frame of Array.from(window.document.querySelectorAll('iframe'))) frame.remove();
});

async function mount(
  html: string,
  layout: (document: Document) => void,
  theme: 'light' | 'dark' = 'dark'
) {
  const frame = window.document.createElement('iframe');
  window.document.body.appendChild(frame);
  const win = frame.contentWindow as Window & typeof globalThis;
  const document = win.document;
  document.body.innerHTML = html;
  setRect(document.body, () => ({ left: 0, width: 800 }));
  Object.defineProperty(document.documentElement, 'clientWidth', {
    configurable: true,
    value: 800,
  });
  layout(document);
  vi.spyOn(win.parent, 'postMessage').mockImplementation((message: unknown) => {
    posted.push(message as Record<string, unknown>);
  });
  new Function('window', 'document', frameFitSource(theme))(win, document);
  await settle();
  return document;
}

const widths = () =>
  posted
    .filter(message => message.method === 'ui/notifications/size-changed')
    .map(message => (message.params as { width: number; fit: boolean }).width);

describe('frameFitSource', () => {
  it('makes the page canvas transparent in the host scheme, last in head', async () => {
    const document = await mount('<div>x</div>', () => undefined, 'light');
    const head = document.head;
    const extra = document.createElement('style');
    head.appendChild(extra);
    await settle();
    const style = head.lastElementChild as HTMLStyleElement;
    expect(style.hasAttribute('data-oh-host')).toBe(true);
    expect(style.textContent).toContain('html,body{background:transparent!important;}');
    expect(style.textContent).toContain(':root{color-scheme:light!important;}');
  });

  it('reports a card narrower than the page at its own width', async () => {
    await mount('<div id="card"></div>', document => {
      setRect(document.getElementById('card')!, () => ({ left: 0, width: 480 }));
    });
    expect(widths()).toEqual([480]);
    expect(posted[0]).toMatchObject({
      jsonrpc: '2.0',
      params: { fit: true, height: expect.any(Number) },
    });
  });

  it('reports a centred card symmetrically', async () => {
    await mount('<div id="card" style="margin-left:160px;margin-right:160px"></div>', document => {
      setRect(document.body, () => ({ left: 8, width: 784 }));
      setRect(document.getElementById('card')!, () => ({ left: 160, width: 480 }));
    });
    expect(widths()).toEqual([496]);
  });

  it('reports a full-width box at its max-width', async () => {
    await mount('<div id="card" style="max-width:480px"></div>', document => {
      setRect(document.getElementById('card')!, () => ({ left: 0, width: 800 }));
    });
    expect(widths()).toEqual([480]);
  });

  it('counts the padding of a content-box max-width box, the same at any frame width', async () => {
    let bodyWidth = 784;
    const style = 'max-width:480px;padding:0 16px';
    const document = await mount(`<div id="card" style="${style}"></div>`, document => {
      setRect(document.body, () => ({ left: 8, width: bodyWidth }));
      setRect(document.getElementById('card')!, () => ({
        left: 8,
        width: Math.min(bodyWidth, 512),
      }));
    });
    expect(widths()).toEqual([528]);

    bodyWidth = 512;
    Object.defineProperty(document.documentElement, 'clientWidth', { value: 528 });
    document.body.appendChild(document.createElement('span'));
    await settle();
    expect(widths()).toEqual([528]);
  });

  it('descends full-width wrappers to the content', async () => {
    await mount('<div id="wrap"><div id="card"></div></div>', document => {
      setRect(document.getElementById('wrap')!, () => ({ left: 0, width: 800 }));
      setRect(document.getElementById('card')!, () => ({ left: 0, width: 400 }));
    });
    expect(widths()).toEqual([400]);
  });

  it('asks for the whole column for running text or a scrolling row', async () => {
    await mount('<p id="text">hello</p>', document => {
      setRect(document.getElementById('text')!, () => ({ left: 0, width: 800 }));
    });
    expect(widths()).toEqual([FILL_WIDTH]);

    posted = [];
    await mount('<div id="card"><div id="row" style="overflow-x:auto"></div></div>', document => {
      setRect(document.getElementById('card')!, () => ({ left: 0, width: 480 }));
      const row = document.getElementById('row')!;
      setRect(row, () => ({ left: 0, width: 300 }));
      Object.defineProperty(row, 'scrollWidth', { configurable: true, value: 1000 });
      Object.defineProperty(row, 'clientWidth', { configurable: true, value: 300 });
    });
    expect(widths()).toEqual([FILL_WIDTH]);
  });

  it('ignores changes smaller than its hysteresis step', async () => {
    let width = 480;
    const document = await mount('<div id="card"></div>', document => {
      setRect(document.getElementById('card')!, () => ({ left: 0, width }));
    });
    width = 484;
    document.body.appendChild(document.createElement('span'));
    await settle();
    expect(widths()).toEqual([480]);

    width = 600;
    document.body.appendChild(document.createElement('span'));
    await settle();
    expect(widths()).toEqual([480, 600]);
  });
});
