/**
 * Per-kind canvas iconography for the 14 tinyflows `NodeKind`s.
 *
 * Split out of `nodeKindMeta.ts` on purpose: that module is deliberately
 * dependency-free (no React) so a plain palette button and a `<Handle>`-bearing
 * canvas card can share one source of truth. Icon *components* are React, so
 * they live here and are keyed by the same `NodeKind` union — the two modules
 * stay in lockstep because both are exhaustive `Record<NodeKind, …>` maps, and
 * TypeScript fails the build if a kind is added to one and not the other.
 *
 * These replace the emoji the canvas previously rendered. Emoji were a
 * liability for a workflow surface: their glyphs are supplied by the platform,
 * so the same graph looked materially different on macOS, Windows and Linux;
 * they carry their own colour, which fought the card's own state colouring; and
 * they read as informal next to the rest of the product. Lucide (via
 * `react-icons/lu`) is already the house set — it is by far the most-used icon
 * family in `app/src` — so these inherit `currentColor` and sit at the same
 * optical weight as the icons in Settings and Chat.
 */
import { createElement } from 'react';
import type { IconType } from 'react-icons';
import {
  LuBinary,
  LuBot,
  LuBraces,
  LuBrain,
  LuCode,
  LuFilter,
  LuGitBranch,
  LuGitFork,
  LuGitMerge,
  LuGlobe,
  LuLayers,
  LuRepeat,
  LuSplit,
  LuWand,
  LuWrench,
  LuZap,
} from 'react-icons/lu';

import type { NodeKind } from './types';

/**
 * The glyph each node kind renders on the canvas, in its palette entry, and in
 * the config drawer header.
 *
 * Chosen so the shape alone hints at the operation — branching kinds use the
 * git-family glyphs (`condition` splits two ways, `switch` forks many,
 * `merge` joins, `split_out` fans out), data-shaping kinds use symbolic
 * glyphs (`transform` a wand, `output_parser` braces), and the three
 * "reach outside" kinds are visually distinct from each other (`tool_call` a
 * wrench, `http_request` a globe, `code` angle brackets). `loop` is the one
 * kind whose glyph shows a cycle, matching the back-edge it draws on canvas.
 */
export const NODE_KIND_ICON: Record<NodeKind, IconType> = {
  trigger: LuZap,
  agent: LuBot,
  tool_call: LuWrench,
  http_request: LuGlobe,
  code: LuCode,
  condition: LuGitBranch,
  switch: LuGitFork,
  merge: LuGitMerge,
  split_out: LuSplit,
  transform: LuWand,
  output_parser: LuBraces,
  sub_workflow: LuLayers,
  memory: LuBrain,
  dedup: LuFilter,
  loop: LuRepeat,
};

/**
 * Icon for a possibly-unknown kind. The canvas renders whatever the persisted
 * graph holds, and a graph saved by a newer build can carry a kind this one has
 * never heard of — that must render as a plain node, never throw, since there
 * is no error boundary around `<ReactFlow>`.
 */
export function nodeKindIcon(kind: NodeKind | string): IconType {
  return NODE_KIND_ICON[kind as NodeKind] ?? LuBinary;
}

/**
 * Renders a kind's glyph. The canvas card, the palette and the config drawer
 * all go through this rather than resolving the icon themselves.
 *
 * Uses `createElement` instead of the more natural
 * `const Icon = nodeKindIcon(kind); return <Icon />` because binding the result
 * of a call to a capitalized local trips `react-hooks/static-components` — the
 * rule cannot distinguish a lookup in a module-level constant map (stable
 * identity, no remount) from a component defined inline during render (new
 * identity every render, subtree remounts). This is the former; the indirection
 * exists solely to say so once, here, instead of at three call sites.
 *
 * `icon` overrides the per-kind lookup for nodes that need a distinct glyph
 * without a distinct `NodeKind` — today only the native OpenHuman tool, which
 * is a `tool_call` but reads as one of the assistant's own capabilities.
 */
export function NodeKindGlyph({
  kind,
  icon,
  className,
}: {
  kind: NodeKind | string;
  icon?: IconType;
  className?: string;
}) {
  return createElement(icon ?? nodeKindIcon(kind), { className, 'aria-hidden': true });
}

/**
 * The tile fill behind each kind's glyph — the one saturated element on an
 * otherwise neutral card.
 *
 * **Why colour lives on the tile and not the card.** A fully neutral card is
 * unreadable on this product's dark theme: the canvas is pure black, `surface`
 * is `#171717` and `line` is `#262626`, so a grey-on-grey card has almost no
 * hue and almost no luminance contrast. But colouring the *card* by kind is
 * what the previous design did, and it broke status: `sage` and `coral` mean
 * success and error everywhere else in the product, so a healthy `merge` node
 * rendered green and a `tool_call` amber read as run state rather than type.
 *
 * A 32px tile resolves both. It is saturated enough to give the canvas life and
 * to make kinds recognisable at a glance before the label is readable, while
 * run state stays legible because it is drawn on a different surface — a ring
 * around the whole card. Identity is a filled square; status is an outline.
 * The two never compete for the same pixels.
 *
 * Gradients rather than flat fills: the darker stop guarantees the white glyph
 * clears contrast even where the lighter stop (`accent-lavender`, `accent-sky`)
 * is too pale to carry white on its own.
 *
 * Hues are grouped by role so the mapping is learnable rather than arbitrary —
 * warm for control flow that branches, ocean for anything reaching outside the
 * graph, violet for the AI-backed kinds, green for kinds that recombine, and
 * cool grey for pure data shaping.
 */
export const NODE_KIND_TILE: Record<NodeKind, string> = {
  // Start — the spark that begins a run.
  trigger: 'bg-gradient-to-br from-amber-400 to-amber-600',
  // Intelligence — model-backed kinds.
  agent: 'bg-gradient-to-br from-accent-lavender to-primary-600',
  memory: 'bg-gradient-to-br from-accent-lavender to-primary-700',
  // Reaching outside the graph.
  tool_call: 'bg-gradient-to-br from-primary-400 to-primary-600',
  http_request: 'bg-gradient-to-br from-accent-sky to-primary-500',
  sub_workflow: 'bg-gradient-to-br from-primary-500 to-primary-700',
  // Control flow that splits.
  condition: 'bg-gradient-to-br from-amber-400 to-coral-500',
  switch: 'bg-gradient-to-br from-amber-500 to-coral-600',
  // Control flow that recombines / fans out.
  merge: 'bg-gradient-to-br from-sage-400 to-sage-600',
  split_out: 'bg-gradient-to-br from-sage-500 to-sage-700',
  // Data shaping — deliberately the quietest family.
  code: 'bg-gradient-to-br from-slate-500 to-slate-700',
  transform: 'bg-gradient-to-br from-accent-lavender to-accent-rose',
  output_parser: 'bg-gradient-to-br from-slate-400 to-slate-600',
  dedup: 'bg-gradient-to-br from-slate-500 to-slate-600',
  // Control flow that repeats — sage like the other recombining kinds, since a
  // loop head is where the body's output flows back in.
  loop: 'bg-gradient-to-br from-sage-400 to-sage-700',
};

/** Neutral tile for a kind this build does not recognise. See {@link nodeKindIcon}. */
const UNKNOWN_KIND_TILE = 'bg-gradient-to-br from-stone-400 to-stone-600';

/** Tile fill for a possibly-unknown kind. */
export function nodeKindTile(kind: NodeKind | string): string {
  return NODE_KIND_TILE[kind as NodeKind] ?? UNKNOWN_KIND_TILE;
}

/**
 * Tile size variants. Deliberately whole static class strings rather than
 * numeric props interpolated into `h-[${n}px]`: Tailwind's JIT compiler
 * discovers classes by scanning source text, so a class assembled at runtime
 * from a template literal is never generated and the element renders unstyled.
 */
const TILE_SIZES = {
  /** Palette rows — small enough that the glyph needs the tighter radius. */
  sm: { tile: 'h-5 w-5 rounded-md', glyph: 'h-3 w-3' },
  /** Canvas cards and the config-drawer header. */
  md: { tile: 'h-8 w-8 rounded-lg', glyph: 'h-[18px] w-[18px]' },
} as const;

export type NodeKindTileSize = keyof typeof TILE_SIZES;

/**
 * A kind's glyph on its colour-coded tile — the single element the canvas card,
 * the palette row and the config-drawer header all render.
 *
 * Keeping the tile chrome (gradient, inset ring, shadow, radius) in one place
 * means the swatch a user picks in the palette is provably the swatch that
 * lands on the graph: the three surfaces cannot drift because they no longer
 * each own a copy of the markup.
 *
 * `aria-hidden` because the tile is decorative — a card's accessible name is
 * its node name, and the kind is conveyed textually by the kind label rather
 * than by colour or glyph alone.
 */
export function NodeKindTile({
  kind,
  icon,
  size = 'md',
  className = '',
  testId,
}: {
  kind: NodeKind | string;
  /** Overrides the per-kind glyph. See {@link NodeKindGlyph}. */
  icon?: IconType;
  size?: NodeKindTileSize;
  /** Extra positioning classes for the call site (e.g. `mt-0.5`). */
  className?: string;
  testId?: string;
}) {
  const { tile, glyph } = TILE_SIZES[size];
  return (
    <span
      aria-hidden="true"
      data-testid={testId}
      className={`flex shrink-0 items-center justify-center text-white shadow-sm ring-1 ring-inset ring-white/15 ${tile} ${nodeKindTile(kind)} ${className}`}>
      <NodeKindGlyph kind={kind} icon={icon} className={glyph} />
    </span>
  );
}
