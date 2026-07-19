/**
 * Per-kind visual metadata for the 12 tinyflows `NodeKind`s, shared by the
 * canvas node renderer (`FlowNodeComponent`) and the editable canvas's node
 * palette (`NodePalette`). Kept dependency-free (no React) so both a rendered
 * `<Handle>`-bearing card and a plain palette button can pull the same
 * emoji/accent from one source of truth instead of drifting apart.
 *
 * Colors cycle through the four CSS-variable-backed semantic ramps
 * (primary/sage/amber/coral) that support Tailwind's `/opacity` modifiers in
 * this codebase (see `tailwind.config.js`) so light/dark theming comes for
 * free; with 12 kinds and 4 ramps some kinds share a color family — the emoji
 * + name remain the primary distinguishers.
 */
import type { NodeKind } from './types';

export type NodeColor = 'sage' | 'primary' | 'amber' | 'coral' | 'neutral';

/**
 * Palette grouping for the node kinds: `triggers` (what starts a run),
 * `actions` (do work / call out), `logic` (route, branch, reshape data). Used
 * by {@link NodePalette} to render labelled sections instead of a flat list.
 */
export type NodeGroup = 'triggers' | 'actions' | 'logic';

export interface NodeKindMeta {
  emoji: string;
  color: NodeColor;
  group: NodeGroup;
}

/**
 * The 12 `NodeKind`s in the order they should appear in the palette. Trigger
 * leads (every graph needs exactly one); the rest follow the logical grouping
 * of the `tinyflows::model::NodeKind` enum.
 */
export const NODE_KINDS: NodeKind[] = [
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
];

/** Per-kind emoji + border/chip color + palette group. See the module doc. */
export const NODE_KIND_META: Record<NodeKind, NodeKindMeta> = {
  trigger: { emoji: '⚡', color: 'sage', group: 'triggers' },
  agent: { emoji: '🤖', color: 'primary', group: 'actions' },
  tool_call: { emoji: '🔧', color: 'amber', group: 'actions' },
  http_request: { emoji: '🌐', color: 'coral', group: 'actions' },
  code: { emoji: '📝', color: 'sage', group: 'actions' },
  sub_workflow: { emoji: '🧩', color: 'coral', group: 'actions' },
  condition: { emoji: '🔀', color: 'primary', group: 'logic' },
  switch: { emoji: '🔁', color: 'amber', group: 'logic' },
  merge: { emoji: '🔗', color: 'coral', group: 'logic' },
  split_out: { emoji: '📤', color: 'sage', group: 'logic' },
  transform: { emoji: '♻️', color: 'primary', group: 'logic' },
  output_parser: { emoji: '📋', color: 'amber', group: 'logic' },
};

/** Palette group render order + the kinds in each, derived from NODE_KIND_META. */
export const NODE_GROUP_ORDER: NodeGroup[] = ['triggers', 'actions', 'logic'];

export const NODE_KINDS_BY_GROUP: Record<NodeGroup, NodeKind[]> = {
  triggers: NODE_KINDS.filter(k => NODE_KIND_META[k].group === 'triggers'),
  actions: NODE_KINDS.filter(k => NODE_KIND_META[k].group === 'actions'),
  logic: NODE_KINDS.filter(k => NODE_KIND_META[k].group === 'logic'),
};

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
  emoji: string;
  color: NodeColor;
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
        emoji: '🔌',
        color: 'amber',
        labelKey: 'flows.palette.appAction',
        preset: { provider: 'composio' },
      },
      {
        key: 'oh_tool',
        kind: 'tool_call',
        group: 'actions',
        emoji: '🛠️',
        color: 'primary',
        labelKey: 'flows.palette.ohTool',
        preset: { provider: 'openhuman' },
      },
    ];
  }
  return [
    {
      key: kind,
      kind,
      group: meta.group,
      emoji: meta.emoji,
      color: meta.color,
      labelKey: `flows.nodeKind.${kind}`,
    },
  ];
});

export const PALETTE_ENTRIES_BY_GROUP: Record<NodeGroup, PaletteEntry[]> = {
  triggers: PALETTE_ENTRIES.filter(e => e.group === 'triggers'),
  actions: PALETTE_ENTRIES.filter(e => e.group === 'actions'),
  logic: PALETTE_ENTRIES.filter(e => e.group === 'logic'),
};

/**
 * Fallback for any `kind` outside {@link NODE_KIND_META} — a saved graph is
 * `unknown` on the wire (cast in `FlowCanvasPage.tsx`), so a future 13th
 * tinyflows kind, or any other value the backend ever emits, can reach the
 * renderer at runtime even though TypeScript can't see it. Lookups fall back
 * here so an unrecognized kind renders as a plain neutral node instead of
 * crashing the whole canvas (there's no error boundary around `<ReactFlow>`).
 */
export const DEFAULT_NODE_META: NodeKindMeta = { emoji: '❔', color: 'neutral', group: 'actions' };

/** Resolve a kind's metadata, falling back to {@link DEFAULT_NODE_META}. */
export function nodeKindMeta(kind: NodeKind): NodeKindMeta {
  return NODE_KIND_META[kind] ?? DEFAULT_NODE_META;
}

// `tint` is a faint wash of the accent for the node card body, so the whole
// node reads as gently colour-coded rather than a stark white box under the
// stronger header `chip`.
export const COLOR_CLASSES: Record<NodeColor, { border: string; chip: string; tint: string }> = {
  sage: {
    border: 'border-sage-400 dark:border-sage-500/60',
    chip: 'bg-sage-100 dark:bg-sage-500/20',
    tint: 'bg-sage-50/60 dark:bg-sage-500/[0.07]',
  },
  primary: {
    border: 'border-primary-400 dark:border-primary-500/60',
    chip: 'bg-primary-100 dark:bg-primary-500/20',
    tint: 'bg-primary-50/60 dark:bg-primary-500/[0.07]',
  },
  amber: {
    border: 'border-amber-400 dark:border-amber-500/60',
    chip: 'bg-amber-100 dark:bg-amber-500/20',
    tint: 'bg-amber-50/60 dark:bg-amber-500/[0.07]',
  },
  coral: {
    border: 'border-coral-400 dark:border-coral-500/60',
    chip: 'bg-coral-100 dark:bg-coral-500/20',
    tint: 'bg-coral-50/60 dark:bg-coral-500/[0.07]',
  },
  neutral: { border: 'border-line-strong', chip: 'bg-surface-subtle', tint: 'bg-surface' },
};
