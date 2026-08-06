/**
 * Per-kind palette grouping for the 15 tinyflows `NodeKind`s, shared by the
 * canvas node renderer (`FlowNodeComponent`) and the editable canvas's node
 * palette (`NodePalette`). Kept dependency-free (no React) so both a rendered
 * `<Handle>`-bearing card and a plain palette button can pull the same
 * grouping from one source of truth instead of drifting apart.
 *
 * **Iconography and colour live in `nodeKindIcons.tsx`, not here** — they are
 * React components, and keeping them out preserves this module's React-free
 * property. The two stay in lockstep because both are exhaustive
 * `Record<NodeKind, …>` maps, so adding a kind to one and not the other fails
 * the build.
 *
 * This module previously also carried a per-kind emoji and one of four semantic
 * colour ramps used to paint the card's border and header chip. Both are gone:
 * platform-supplied emoji glyphs made the same graph look materially different
 * across macOS/Windows/Linux, and the ramps carry status meaning elsewhere in
 * the product (sage = success, coral = error), so a healthy `merge` node
 * rendered coral read as a failure. Kind colour is now confined to the glyph
 * tile (`NODE_KIND_TILE`), leaving the card's border free to signal run state.
 */
import type { NodeKind } from './types';

/**
 * Palette grouping for the node kinds: `triggers` (what starts a run),
 * `actions` (do work / call out), `logic` (route, branch, reshape data). Used
 * by {@link NodePalette} to render labelled sections instead of a flat list.
 */
export type NodeGroup = 'triggers' | 'actions' | 'logic';

interface NodeKindMeta {
  group: NodeGroup;
}

/**
 * The 15 `NodeKind`s in the order they should appear in the palette. Trigger
 * leads (every graph needs exactly one); the rest follow the logical grouping
 * of the `tinyflows::model::NodeKind` enum. `memory` (issue #5226) is
 * appended after `sub_workflow` — the design doc (`08-memory-node.md`)
 * sequences it as the 13th kind deliberately, so it trails `sub_workflow`
 * here too rather than being interleaved with the other `actions`-group
 * kinds. `dedup` (issue #5263) is the 14th kind and is appended last in turn
 * — it renders in the `logic` group (alongside `condition`/`split_out`/
 * `merge`) regardless of its position here, since {@link PALETTE_ENTRIES_BY_GROUP}
 * filters by group rather than relying on interleaved array order. `loop` is
 * the 15th and is appended last for the same reason.
 */
const NODE_KINDS: NodeKind[] = [
  'trigger',
  'agent',
  'tool_call',
  'http_request',
  'code',
  'condition',
  'switch',
  'merge',
  'split_out',
  'transform',
  'output_parser',
  'sub_workflow',
  'memory',
  'dedup',
  'loop',
];

/** Per-kind palette group. See the module doc. */
const NODE_KIND_META: Record<NodeKind, NodeKindMeta> = {
  trigger: { group: 'triggers' },
  agent: { group: 'actions' },
  tool_call: { group: 'actions' },
  http_request: { group: 'actions' },
  code: { group: 'actions' },
  sub_workflow: { group: 'actions' },
  // Declarative in-graph memory access (recall/search/flavour/people/
  // remember/forget) — an "actions" node like `tool_call`/`http_request`
  // (it reads/writes state), not `logic` (it doesn't itself branch/reshape).
  memory: { group: 'actions' },
  condition: { group: 'logic' },
  switch: { group: 'logic' },
  merge: { group: 'logic' },
  split_out: { group: 'logic' },
  transform: { group: 'logic' },
  output_parser: { group: 'logic' },
  // Skips items already seen, keyed by a stable per-item `=`-expression — a
  // "logic" node like its neighbours (`condition`/`split_out`/`merge`): it
  // routes/filters the item stream rather than calling out or reading state.
  dedup: { group: 'logic' },
  // Bounded loop head: emits on `body` until its cap or condition says stop,
  // then on `done`. A "logic" node — it routes the item stream back around
  // rather than calling out or reading state.
  loop: { group: 'logic' },
};

/** Palette group render order. */
export const NODE_GROUP_ORDER: NodeGroup[] = ['triggers', 'actions', 'logic'];

/**
 * One palette entry. Usually 1:1 with a `NodeKind`, but `tool_call` splits into
 * TWO entries — an "App action" (Composio OAuth) node and a "Tool" (native
 * OpenHuman) node — distinguished by the `preset` config (`provider`) merged
 * onto the new node. `key` is the palette/testid id; `labelKey` its i18n label.
 */
export interface PaletteEntry {
  key: string;
  kind: NodeKind;
  group: NodeGroup;
  labelKey: string;
  /** Default config merged onto a node created from this entry. */
  preset?: Record<string, unknown>;
}

export const PALETTE_ENTRIES: PaletteEntry[] = NODE_KINDS.flatMap((kind): PaletteEntry[] => {
  const meta = NODE_KIND_META[kind];
  if (kind === 'tool_call') {
    return [
      {
        key: 'tool_call',
        kind: 'tool_call',
        group: 'actions',
        labelKey: 'flows.palette.appAction',
        preset: { provider: 'composio' },
      },
      {
        key: 'oh_tool',
        kind: 'tool_call',
        group: 'actions',
        labelKey: 'flows.palette.ohTool',
        preset: { provider: 'openhuman' },
      },
    ];
  }
  return [{ key: kind, kind, group: meta.group, labelKey: `flows.nodeKind.${kind}` }];
});

export const PALETTE_ENTRIES_BY_GROUP: Record<NodeGroup, PaletteEntry[]> = {
  triggers: PALETTE_ENTRIES.filter(e => e.group === 'triggers'),
  actions: PALETTE_ENTRIES.filter(e => e.group === 'actions'),
  logic: PALETTE_ENTRIES.filter(e => e.group === 'logic'),
};

/**
 * Fallback for any `kind` outside {@link NODE_KIND_META} — a saved graph is
 * `unknown` on the wire (cast in `FlowCanvasPage.tsx`), so a future 14th
 * tinyflows kind, or any other value the backend ever emits, can reach the
 * renderer at runtime even though TypeScript can't see it. Lookups fall back
 * here so an unrecognized kind renders as a plain neutral node instead of
 * crashing the whole canvas (there's no error boundary around `<ReactFlow>`).
 */
const DEFAULT_NODE_META: NodeKindMeta = { group: 'actions' };

/** Resolve a kind's metadata, falling back to {@link DEFAULT_NODE_META}. */
export function nodeKindMeta(kind: NodeKind): NodeKindMeta {
  return NODE_KIND_META[kind] ?? DEFAULT_NODE_META;
}
