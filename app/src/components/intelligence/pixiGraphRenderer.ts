/**
 * WebGL memory-graph renderer — Pixi.js draw loop driven by a d3-force
 * simulation. This is the same stack Obsidian's graph view uses (Pixi for
 * GPU rendering, force-directed physics) so it stays smooth well past the
 * 1000-node cap.
 *
 * The renderer is fully imperative: a React wrapper mounts it into a host
 * element and feeds hover/open back through callbacks. All interaction
 * (drag a node, drag the background to pan, wheel to zoom) is hit-tested
 * against the simulation positions in `memoryGraphLayout`, so there are no
 * per-node DOM objects — the whole graph is a single canvas.
 *
 * Drawing is dirty-flagged: while the simulation is warm (or the user is
 * interacting) we redraw each frame; once it cools the loop idles.
 */
import { Application, Container, type FederatedPointerEvent, Graphics } from 'pixi.js';

import {
  createSimulation,
  nodeColor,
  nodeGlows,
  nodeRadius,
  pickNode,
  type SimLink,
  type SimNode,
  ZOOM_MAX,
  ZOOM_MIN,
} from './memoryGraphLayout';

export interface PixiGraphOptions {
  simNodes: SimNode[];
  links: SimLink[];
  dark: boolean;
  onHover: (node: SimNode | null) => void;
  onOpen: (node: SimNode) => void;
  /** Fired once the force simulation first cools (graph is laid out). */
  onReady?: () => void;
}

export interface PixiGraphHandle {
  resetView(): void;
  setTheme(dark: boolean): void;
  updateGraph(simNodes: SimNode[], links: SimLink[]): void;
  destroy(): void;
}

function colorNum(hex: string): number {
  return parseInt(hex.replace('#', ''), 16);
}

/**
 * Mount a Pixi graph into `host`. Resolves once the WebGL context is live;
 * rejects/throws if Pixi can't initialise (caller falls back to SVG).
 */
export async function mountPixiGraph(
  host: HTMLElement,
  opts: PixiGraphOptions
): Promise<PixiGraphHandle> {
  const app = new Application();
  await app.init({
    resizeTo: host,
    backgroundAlpha: 0,
    antialias: true,
    autoDensity: true,
    resolution: typeof window !== 'undefined' ? window.devicePixelRatio || 1 : 1,
    // Match Obsidian — force the WebGL backend rather than letting Pixi
    // probe WebGPU, which is uneven across the CEF runtime.
    preference: 'webgl',
  });
  host.appendChild(app.canvas);
  app.canvas.style.width = '100%';
  app.canvas.style.height = '100%';
  app.canvas.style.display = 'block';

  const world = new Container();
  const edgeG = new Graphics();
  const nodeG = new Graphics();
  world.addChild(edgeG);
  world.addChild(nodeG);
  app.stage.addChild(world);

  const recenter = () => world.position.set(app.screen.width / 2, app.screen.height / 2);
  recenter();

  const sim = createSimulation(opts.simNodes, opts.links);
  sim.alpha(1);

  let dark = opts.dark;
  let dirty = true;
  let hoveredId: string | null = null;
  // Fires `onReady` exactly once when the sim first cools — the signal a
  // loading overlay (e.g. the Brain page) waits on before revealing the graph.
  let readyFired = false;
  // Auto-fit the whole graph into view until the user pans/zooms/drags,
  // so the initial frame is zoomed out to show as much as possible.
  let userInteracted = false;

  /** Scale + centre the world so every node's disc fits the viewport. */
  const fitToView = () => {
    if (opts.simNodes.length === 0) return;
    let minX = Infinity;
    let minY = Infinity;
    let maxX = -Infinity;
    let maxY = -Infinity;
    for (const n of opts.simNodes) {
      const r = nodeRadius(n) + 6;
      if (n.x - r < minX) minX = n.x - r;
      if (n.y - r < minY) minY = n.y - r;
      if (n.x + r > maxX) maxX = n.x + r;
      if (n.y + r > maxY) maxY = n.y + r;
    }
    if (!Number.isFinite(minX)) return;
    const pad = 48;
    const w = Math.max(1, maxX - minX);
    const h = Math.max(1, maxY - minY);
    const scale = Math.min(
      ZOOM_MAX,
      Math.max(ZOOM_MIN, Math.min((app.screen.width - pad) / w, (app.screen.height - pad) / h))
    );
    world.scale.set(scale);
    const cx = (minX + maxX) / 2;
    const cy = (minY + maxY) / 2;
    world.position.set(app.screen.width / 2 - cx * scale, app.screen.height / 2 - cy * scale);
  };

  const draw = () => {
    edgeG.clear();
    for (const l of opts.links) {
      const s = l.source as SimNode;
      const t = l.target as SimNode;
      if (!s || !t || typeof s.x !== 'number' || typeof t.x !== 'number') continue;
      edgeG.moveTo(s.x, s.y);
      edgeG.lineTo(t.x, t.y);
    }
    edgeG.stroke({ width: 0.8, color: dark ? 0x475569 : 0xcbd5e1, alpha: 0.7 });

    nodeG.clear();
    // Halos first so the structural levels "light up" beneath the discs.
    for (const n of opts.simNodes) {
      if (!nodeGlows(n)) continue;
      nodeG
        .circle(n.x, n.y, nodeRadius(n) + 5)
        .fill({ color: colorNum(nodeColor(n)), alpha: 0.18 });
    }
    for (const n of opts.simNodes) {
      const hover = n.id === hoveredId;
      const r = nodeRadius(n) + (hover ? 2 : 0);
      nodeG.circle(n.x, n.y, r).fill({ color: colorNum(nodeColor(n)), alpha: 1 });
      if (hover) nodeG.circle(n.x, n.y, r).stroke({ width: 1.4, color: 0x0f172a, alpha: 0.9 });
    }
  };

  app.ticker.add(() => {
    let changed = dirty;
    if (sim.alpha() > sim.alphaMin()) {
      sim.tick();
      changed = true;
    } else if (!readyFired) {
      // Simulation has cooled → the layout has settled. Signal readiness once.
      readyFired = true;
      opts.onReady?.();
    }
    if (changed) {
      // Keep the whole graph framed while it settles, until the user
      // takes over the camera.
      if (!userInteracted) fitToView();
      draw();
      dirty = false;
    }
  });

  // ── interaction ────────────────────────────────────────────────────
  app.stage.eventMode = 'static';
  app.stage.hitArea = app.screen;
  let drag:
    | { node: SimNode; moved: boolean }
    | { panX: number; panY: number; px: number; py: number; moved: boolean }
    | null = null;

  const setCursor = (c: string) => {
    app.canvas.style.cursor = c;
  };

  app.stage.on('pointerdown', (e: FederatedPointerEvent) => {
    userInteracted = true; // hand the camera to the user
    const p = world.toLocal(e.global);
    const node = pickNode(opts.simNodes, p.x, p.y);
    if (node) {
      sim.alpha(0.3);
      node.fx = node.x;
      node.fy = node.y;
      drag = { node, moved: false };
      setCursor('grabbing');
    } else {
      drag = {
        panX: world.position.x,
        panY: world.position.y,
        px: e.global.x,
        py: e.global.y,
        moved: false,
      };
      setCursor('grabbing');
    }
  });

  app.stage.on('pointermove', (e: FederatedPointerEvent) => {
    if (drag) {
      if ('node' in drag) {
        const p = world.toLocal(e.global);
        drag.node.fx = p.x;
        drag.node.fy = p.y;
        drag.moved = true;
        if (sim.alpha() < 0.1) sim.alpha(0.1);
      } else {
        world.position.set(drag.panX + (e.global.x - drag.px), drag.panY + (e.global.y - drag.py));
        drag.moved = true;
      }
      dirty = true;
      return;
    }
    const p = world.toLocal(e.global);
    const node = pickNode(opts.simNodes, p.x, p.y);
    const id = node ? node.id : null;
    setCursor(node ? 'pointer' : 'grab');
    if (id !== hoveredId) {
      hoveredId = id;
      dirty = true;
      opts.onHover(node ?? null);
    }
  });

  const endDrag = (open: boolean) => {
    const d = drag;
    if (d && 'node' in d) {
      // Release the pin so physics resumes for that node.
      d.node.fx = null;
      d.node.fy = null;
      if (open && !d.moved) opts.onOpen(d.node);
    }
    drag = null;
    setCursor('grab');
  };
  app.stage.on('pointerup', () => endDrag(true));
  app.stage.on('pointerupoutside', () => endDrag(false));

  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    userInteracted = true;
    const gx = e.offsetX;
    const gy = e.offsetY;
    // Graph point under the cursor, kept fixed across the zoom.
    const lx = (gx - world.position.x) / world.scale.x;
    const ly = (gy - world.position.y) / world.scale.y;
    const next = Math.min(
      ZOOM_MAX,
      Math.max(ZOOM_MIN, world.scale.x * Math.exp(-e.deltaY * 0.0015))
    );
    world.scale.set(next);
    world.position.set(gx - lx * next, gy - ly * next);
    dirty = true;
  };
  app.canvas.addEventListener('wheel', onWheel, { passive: false });

  app.renderer.on('resize', () => {
    dirty = true;
  });

  return {
    resetView() {
      userInteracted = false;
      sim.alpha(0.3);
      dirty = true;
    },
    setTheme(next: boolean) {
      dark = next;
      dirty = true;
    },
    updateGraph(nextNodes: SimNode[], nextLinks: SimLink[]) {
      const oldById = new Map(opts.simNodes.map(n => [n.id, n]));

      for (const n of nextNodes) {
        const old = oldById.get(n.id);
        if (old) {
          n.x = old.x;
          n.y = old.y;
          n.vx = old.vx ?? 0;
          n.vy = old.vy ?? 0;
          n.fx = old.fx ?? undefined;
          n.fy = old.fy ?? undefined;
        } else {
          // New node — seed near its parent or at a small random offset
          // from the centroid so it animates into place.
          const parentLink = nextLinks.find(
            l => (typeof l.source === 'string' ? l.source : (l.source as SimNode).id) === n.id
          );
          const parentId =
            parentLink &&
            (typeof parentLink.target === 'string'
              ? parentLink.target
              : (parentLink.target as SimNode).id);
          const parent = parentId ? oldById.get(parentId) : undefined;
          if (parent) {
            n.x = parent.x + (Math.random() - 0.5) * 40;
            n.y = parent.y + (Math.random() - 0.5) * 40;
          } else {
            n.x = (Math.random() - 0.5) * 100;
            n.y = (Math.random() - 0.5) * 100;
          }
        }
      }

      // Hot-swap the simulation's node and link arrays.
      opts.simNodes = nextNodes;
      opts.links = nextLinks;
      sim.nodes(nextNodes);
      const linkForce = sim.force('link') as ReturnType<typeof import('d3-force').forceLink>;
      if (linkForce && typeof linkForce.links === 'function') {
        linkForce.links(nextLinks);
      }
      // Gentle reheat so new nodes settle without disrupting existing ones.
      sim.alpha(0.3);
      dirty = true;
    },
    destroy() {
      sim.stop();
      app.canvas.removeEventListener('wheel', onWheel);
      app.destroy(true, { children: true });
    },
  };
}
