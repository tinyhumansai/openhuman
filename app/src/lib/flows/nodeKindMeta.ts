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

export interface NodeKindMeta {
  emoji: string;
  color: NodeColor;
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

/** Per-kind emoji + border/chip color. See the module doc for the color model. */
export const NODE_KIND_META: Record<NodeKind, NodeKindMeta> = {
  trigger: { emoji: '⚡', color: 'sage' },
  agent: { emoji: '🤖', color: 'primary' },
  tool_call: { emoji: '🔧', color: 'amber' },
  http_request: { emoji: '🌐', color: 'coral' },
  code: { emoji: '📝', color: 'sage' },
  condition: { emoji: '🔀', color: 'primary' },
  switch: { emoji: '🔁', color: 'amber' },
  merge: { emoji: '🔗', color: 'coral' },
  split_out: { emoji: '📤', color: 'sage' },
  transform: { emoji: '♻️', color: 'primary' },
  output_parser: { emoji: '📋', color: 'amber' },
  sub_workflow: { emoji: '🧩', color: 'coral' },
};

/**
 * Fallback for any `kind` outside {@link NODE_KIND_META} — a saved graph is
 * `unknown` on the wire (cast in `FlowCanvasPage.tsx`), so a future 13th
 * tinyflows kind, or any other value the backend ever emits, can reach the
 * renderer at runtime even though TypeScript can't see it. Lookups fall back
 * here so an unrecognized kind renders as a plain neutral node instead of
 * crashing the whole canvas (there's no error boundary around `<ReactFlow>`).
 */
export const DEFAULT_NODE_META: NodeKindMeta = { emoji: '❔', color: 'neutral' };

/** Resolve a kind's metadata, falling back to {@link DEFAULT_NODE_META}. */
export function nodeKindMeta(kind: NodeKind): NodeKindMeta {
  return NODE_KIND_META[kind] ?? DEFAULT_NODE_META;
}

export const COLOR_CLASSES: Record<NodeColor, { border: string; chip: string }> = {
  sage: {
    border: 'border-sage-400 dark:border-sage-500/60',
    chip: 'bg-sage-100 dark:bg-sage-500/20',
  },
  primary: {
    border: 'border-primary-400 dark:border-primary-500/60',
    chip: 'bg-primary-100 dark:bg-primary-500/20',
  },
  amber: {
    border: 'border-amber-400 dark:border-amber-500/60',
    chip: 'bg-amber-100 dark:bg-amber-500/20',
  },
  coral: {
    border: 'border-coral-400 dark:border-coral-500/60',
    chip: 'bg-coral-100 dark:bg-coral-500/20',
  },
  neutral: { border: 'border-line-strong', chip: 'bg-surface-subtle' },
};

/** Even vertical offsets (in %) for `count` handles along one side of a card. */
export function handleOffsets(count: number): number[] {
  if (count <= 1) return [50];
  return Array.from({ length: count }, (_, i) => ((i + 1) / (count + 1)) * 100);
}
