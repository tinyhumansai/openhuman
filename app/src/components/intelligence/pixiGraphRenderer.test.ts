import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { SimLink, SimNode } from './memoryGraphLayout';
import { mountPixiGraph } from './pixiGraphRenderer';

// Minimal Pixi stubs that record the handlers the renderer wires up so we
// can drive them from the test. Shared with the mock via vi.hoisted.
const h = vi.hoisted(() => {
  const state: {
    stageHandlers: Record<string, (e: unknown) => void>;
    canvasListeners: Record<string, (e: unknown) => void>;
    rendererHandlers: Record<string, () => void>;
    tickerCb: (() => void) | null;
    destroyed: boolean;
  } = {
    stageHandlers: {},
    canvasListeners: {},
    rendererHandlers: {},
    tickerCb: null,
    destroyed: false,
  };
  class Graphics {
    clear() {
      return this;
    }
    circle() {
      return this;
    }
    fill() {
      return this;
    }
    moveTo() {
      return this;
    }
    lineTo() {
      return this;
    }
    stroke() {
      return this;
    }
  }
  class Container {
    position = {
      x: 0,
      y: 0,
      set(x: number, y: number) {
        this.x = x;
        this.y = y;
      },
    };
    scale = {
      x: 1,
      y: 1,
      set(s: number) {
        this.x = s;
        this.y = s;
      },
    };
    children: unknown[] = [];
    eventMode = '';
    hitArea: unknown = null;
    addChild(c: unknown) {
      this.children.push(c);
    }
    removeChildren() {
      this.children = [];
    }
    toLocal(p: { x: number; y: number }) {
      return {
        x: (p.x - this.position.x) / this.scale.x,
        y: (p.y - this.position.y) / this.scale.y,
      };
    }
    on(ev: string, cb: (e: unknown) => void) {
      state.stageHandlers[ev] = cb;
    }
  }
  class Application {
    canvas = {
      style: {} as Record<string, string>,
      addEventListener: (ev: string, cb: (e: unknown) => void) => {
        state.canvasListeners[ev] = cb;
      },
      removeEventListener: vi.fn(),
    };
    stage = new Container();
    screen = { width: 800, height: 600 };
    ticker = {
      add: (cb: () => void) => {
        state.tickerCb = cb;
      },
    };
    renderer = {
      on: (ev: string, cb: () => void) => {
        state.rendererHandlers[ev] = cb;
      },
    };
    init = vi.fn().mockResolvedValue(undefined);
    destroy = vi.fn(() => {
      state.destroyed = true;
    });
  }
  return { state, Graphics, Container, Application };
});

vi.mock('pixi.js', () => ({
  Application: h.Application,
  Container: h.Container,
  Graphics: h.Graphics,
}));
vi.mock('pixi.js/unsafe-eval', () => ({}));

function makeNodes(): SimNode[] {
  return [
    { kind: 'summary', id: 'root', label: 'R', level: 0, parent_id: null, x: 0, y: 0 },
    { kind: 'chunk', id: 'leaf', label: 'L', x: 200, y: 0 },
  ];
}

describe('mountPixiGraph', () => {
  beforeEach(() => {
    h.state.stageHandlers = {};
    h.state.canvasListeners = {};
    h.state.rendererHandlers = {};
    h.state.tickerCb = null;
    h.state.destroyed = false;
  });

  async function mount() {
    const simNodes = makeNodes();
    const links: SimLink[] = [{ source: simNodes[1], target: simNodes[0] }];
    const onHover = vi.fn();
    const onOpen = vi.fn();
    const host = { appendChild: vi.fn() } as unknown as HTMLElement;
    const handle = await mountPixiGraph(host, { simNodes, links, dark: false, onHover, onOpen });
    return { handle, simNodes, onHover, onOpen, host };
  }

  it('wires interaction handlers and a render loop', async () => {
    const { host } = await mount();
    expect(host.appendChild).toHaveBeenCalled();
    expect(h.state.tickerCb).toBeTypeOf('function');
    expect(h.state.stageHandlers.pointerdown).toBeTypeOf('function');
    // Render loop draws while the simulation is warm.
    expect(() => h.state.tickerCb?.()).not.toThrow();
  });

  it('opens a node on a click without movement', async () => {
    const { onOpen } = await mount();
    // World is centred at (400,300); a click there maps to graph (0,0) = "root".
    h.state.stageHandlers.pointerdown({ global: { x: 400, y: 300 } });
    h.state.stageHandlers.pointerup({});
    expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ id: 'root' }));
  });

  it('drags a node instead of opening it once the pointer moves', async () => {
    const { onOpen, simNodes } = await mount();
    h.state.stageHandlers.pointerdown({ global: { x: 400, y: 300 } });
    h.state.stageHandlers.pointermove({ global: { x: 450, y: 320 } });
    h.state.stageHandlers.pointerup({});
    expect(onOpen).not.toHaveBeenCalled();
    // Pin released after the drag so physics can resume.
    expect(simNodes[0].fx ?? null).toBeNull();
  });

  it('emits hover when the pointer is over a node and clears it off-node', async () => {
    const { onHover } = await mount();
    h.state.stageHandlers.pointermove({ global: { x: 400, y: 300 } }); // over root
    expect(onHover).toHaveBeenLastCalledWith(expect.objectContaining({ id: 'root' }));
    h.state.stageHandlers.pointermove({ global: { x: 10, y: 10 } }); // empty space
    expect(onHover).toHaveBeenLastCalledWith(null);
  });

  it('pans on background drag', async () => {
    await mount();
    // Pointer down on empty space starts a pan, not a node drag.
    expect(() => {
      h.state.stageHandlers.pointerdown({ global: { x: 10, y: 10 } });
      h.state.stageHandlers.pointermove({ global: { x: 60, y: 40 } });
      h.state.stageHandlers.pointerup({});
    }).not.toThrow();
  });

  it('zooms on wheel without throwing and prevents default scroll', async () => {
    await mount();
    const preventDefault = vi.fn();
    h.state.canvasListeners.wheel({ offsetX: 400, offsetY: 300, deltaY: -120, preventDefault });
    expect(preventDefault).toHaveBeenCalled();
  });

  it('resetView, setTheme and resize redraw without error', async () => {
    const { handle } = await mount();
    expect(() => {
      handle.setTheme(true);
      handle.resetView();
      h.state.rendererHandlers.resize?.();
      h.state.tickerCb?.();
    }).not.toThrow();
  });

  it('destroy tears down the Pixi application', async () => {
    const { handle } = await mount();
    handle.destroy();
    expect(h.state.destroyed).toBe(true);
  });
});
