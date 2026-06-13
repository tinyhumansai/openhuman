/**
 * Off-main-thread force layout for the SVG fallback renderer.
 *
 * The SVG path used to settle its graph with a synchronous O(n²) all-pairs
 * relaxation (`relaxLayout` in MemoryGraph.tsx) that ran 220 iterations in one
 * blocking call — fine at a few hundred nodes, a multi-second freeze at ~1k,
 * and a renderer-killing hang at ~10k. This worker moves that work off the UI
 * thread: it runs the same d3-force simulation the WebGL path uses (Barnes–Hut
 * charge, link springs, centring, collision) and streams node positions back
 * as a transferable Float32Array each tick. The main thread only applies
 * positions and repaints, so it never blocks regardless of graph size.
 *
 * Protocol (main → worker):
 *   { type: 'init', nodes: {x,y,r}[], links: {source,target}[] }  // source/target = node indices
 *   { type: 'drag', index, x, y, fixed }                          // pin/unpin a node during drag
 *   { type: 'stop' }                                              // halt + tear down the sim
 * Worker → main:
 *   { type: 'tick', positions: Float32Array([x0,y0,x1,y1,…]), alpha }
 *   { type: 'end' }                                               // sim cooled below alphaMin
 */
import {
  forceCenter,
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  type Simulation,
  type SimulationNodeDatum,
} from 'd3-force';

interface WNode extends SimulationNodeDatum {
  r: number;
}
interface WLink {
  source: number;
  target: number;
}

// Local worker-global type so we don't pull the `webworker` lib (which would
// clash with the DOM lib across the whole tsc program). `self` is the worker's
// global scope at runtime.
const ctx = self as unknown as {
  onmessage: ((e: MessageEvent) => void) | null;
  postMessage: (message: unknown, transfer?: Transferable[]) => void;
};

let sim: Simulation<WNode, WLink> | null = null;
let nodes: WNode[] = [];
let timer: ReturnType<typeof setTimeout> | 0 = 0;

// d3 charge/link/collide params mirror createSimulation() in
// memoryGraphLayout.ts so the worker layout matches the WebGL path. `cx`/`cy`
// centre the cloud on the SVG viewport (the WebGL path recentres its own
// camera, but the SVG path renders in fixed viewBox coordinates).
function build(
  initNodes: { x: number; y: number; r: number }[],
  links: WLink[],
  cx: number,
  cy: number,
  alpha: number
) {
  nodes = initNodes.map(n => ({ x: n.x, y: n.y, r: n.r }));
  sim = forceSimulation<WNode, WLink>(nodes)
    .force('charge', forceManyBody<WNode>().strength(-140).distanceMax(420))
    .force(
      'link',
      forceLink<WNode, WLink>(links)
        .id((_d, i) => i)
        .distance(58)
        .strength(0.35)
    )
    .force('center', forceCenter(cx, cy).strength(0.04))
    .force(
      'collide',
      forceCollide<WNode>().radius(d => d.r + 2)
    )
    .stop();
  // 1 on a fresh graph (full settle); ~0.3 on an incremental update so carried
  // positions barely move and only new nodes ease into place.
  sim.alpha(alpha > 0 ? alpha : 1);
}

function postPositions() {
  const buf = new Float32Array(nodes.length * 2);
  for (let i = 0; i < nodes.length; i++) {
    buf[i * 2] = nodes[i].x ?? 0;
    buf[i * 2 + 1] = nodes[i].y ?? 0;
  }
  ctx.postMessage({ type: 'tick', positions: buf, alpha: sim?.alpha() ?? 0 }, [buf.buffer]);
}

function loop() {
  if (!sim) return;
  sim.tick(); // one integration step per frame
  postPositions();
  if (sim.alpha() > sim.alphaMin()) {
    timer = setTimeout(loop, 16); // ~60fps; workers have no requestAnimationFrame
  } else {
    timer = 0;
    ctx.postMessage({ type: 'end' });
  }
}

function stop() {
  if (timer) {
    clearTimeout(timer);
    timer = 0;
  }
  sim?.stop();
  sim = null;
}

ctx.onmessage = (e: MessageEvent) => {
  const msg = e.data as
    | {
        type: 'init';
        nodes: { x: number; y: number; r: number }[];
        links: WLink[];
        cx: number;
        cy: number;
        alpha: number;
      }
    | { type: 'drag'; index: number; x: number; y: number; fixed: boolean }
    | { type: 'stop' };

  if (msg.type === 'init') {
    stop();
    build(msg.nodes, msg.links, msg.cx, msg.cy, msg.alpha);
    loop();
  } else if (msg.type === 'drag') {
    const n = nodes[msg.index];
    if (n) {
      if (msg.fixed) {
        n.fx = msg.x;
        n.fy = msg.y;
      } else {
        n.fx = null;
        n.fy = null;
      }
    }
    if (sim) sim.alpha(Math.max(sim.alpha(), 0.2));
    if (!timer) loop();
  } else if (msg.type === 'stop') {
    stop();
  }
};
