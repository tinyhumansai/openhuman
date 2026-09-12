import { cn } from '@/components/assistant-ui/lib/utils';
import type { ComponentProps, CSSProperties } from 'react';

const GRID = 5;
const CENTER = (GRID - 1) / 2;
const DOT_INDEXES = Array.from({ length: GRID * GRID }, (_, i) => i);

/* Deterministic bit-mixing hash so server and client render identical markup; takes a range in milliseconds and returns seconds. A plain (i * prime) % range correlates indexes a grid-stride apart and renders as column-synchronized waves instead of a twinkle. */
const hash = (n: number, salt: number, range: number) => {
  let h = (Math.imul(n, 374761393) + Math.imul(salt, 668265263)) >>> 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177) >>> 0;
  return ((h ^ (h >>> 16)) % range) / 1000;
};

const glyph = (dots: [number, number][]) => new Set(dots.map(([row, col]) => row * GRID + col));

const CHECK = glyph([
  [1, 4],
  [2, 3],
  [3, 0],
  [3, 2],
  [4, 1],
]);
const CROSS = glyph([
  [0, 0],
  [0, 4],
  [1, 1],
  [1, 3],
  [2, 2],
  [3, 1],
  [3, 3],
  [4, 0],
  [4, 4],
]);
const BANG = glyph([
  [0, 2],
  [1, 2],
  [2, 2],
  [4, 2],
]);
const INFO = glyph([
  [0, 2],
  [2, 2],
  [3, 2],
  [4, 2],
]);
const PAUSE = glyph([
  [1, 1],
  [2, 1],
  [3, 1],
  [1, 3],
  [2, 3],
  [3, 3],
]);
const STOP = glyph([
  [1, 1],
  [1, 2],
  [1, 3],
  [2, 1],
  [2, 2],
  [2, 3],
  [3, 1],
  [3, 2],
  [3, 3],
]);
const RECORD = glyph([
  [1, 2],
  [2, 1],
  [2, 2],
  [2, 3],
  [3, 2],
]);
const ELLIPSIS = glyph([
  [2, 0],
  [2, 2],
  [2, 4],
]);

type Blink = { duration: number; delay: number; lo: number };

type StateConfig = {
  /** Text color class; dots inherit the surrounding color when omitted. */
  color?: string;
  /** Dots that render at full opacity; all others rest at `dim`. Omit for the full grid. */
  glyph?: Set<number>;
  /** Resting opacity of on dots. */
  base?: number;
  /** Resting opacity of off dots when a glyph is set. */
  dim?: number;
  /** Blink parameters per on dot, keyed by index and grid position. */
  blink?: (i: number, row: number, col: number) => Blink;
};

const STATES = {
  idle: { color: 'text-muted-foreground', base: 0.3 },
  loading: {
    blink: i => ({ duration: 0.9 + hash(i, 2, 700), delay: -hash(i, 1, 1200), lo: 0.15 }),
  },
  thinking: { blink: (_i, row, col) => ({ duration: 1.2, delay: -(row + col) * 0.09, lo: 0.2 }) },
  streaming: {
    blink: (_i, row, col) => ({
      duration: 0.9,
      delay: -(row * 0.12 + hash(col, 3, 900)),
      lo: 0.15,
    }),
  },
  searching: { blink: (_i, _row, col) => ({ duration: 1.1, delay: -col * 0.12, lo: 0.2 }) },
  syncing: {
    blink: (_i, row, col) => {
      const turn = (Math.atan2(row - CENTER, col - CENTER) + Math.PI) / (2 * Math.PI);
      return { duration: 1.3, delay: -turn * 1.3, lo: 0.2 };
    },
  },
  connecting: {
    blink: (_i, row, col) => ({
      duration: 1.4,
      delay: -Math.max(Math.abs(row - CENTER), Math.abs(col - CENTER)) * 0.18,
      lo: 0.15,
    }),
  },
  waiting: {
    glyph: ELLIPSIS,
    blink: (_i, _row, col) => ({ duration: 1.2, delay: -col * 0.09, lo: 0.2 }),
  },
  uploading: { blink: (_i, row) => ({ duration: 1, delay: -(GRID - 1 - row) * 0.12, lo: 0.2 }) },
  downloading: { blink: (_i, row) => ({ duration: 1, delay: -row * 0.12, lo: 0.2 }) },
  listening: {
    blink: (_i, _row, col) => ({
      duration: 0.7 + hash(col, 4, 500),
      delay: -hash(col, 5, 900),
      lo: 0.25,
    }),
  },
  speaking: {
    blink: (_i, _row, col) => ({
      duration: 0.4 + hash(col, 6, 350),
      delay: -hash(col, 7, 700),
      lo: 0.2,
    }),
  },
  recording: {
    color: 'text-red-500',
    glyph: RECORD,
    dim: 0.12,
    blink: () => ({ duration: 1.4, delay: 0, lo: 0.3 }),
  },
  success: { color: 'text-emerald-500', glyph: CHECK },
  error: {
    color: 'text-red-500',
    glyph: CROSS,
    blink: () => ({ duration: 1.1, delay: 0, lo: 0.4 }),
  },
  warning: {
    color: 'text-amber-500',
    glyph: BANG,
    blink: () => ({ duration: 1.6, delay: 0, lo: 0.45 }),
  },
  info: { color: 'text-blue-500', glyph: INFO },
  paused: { color: 'text-muted-foreground', glyph: PAUSE },
  stopped: { color: 'text-muted-foreground', glyph: STOP },
  offline: { color: 'text-muted-foreground', base: 0.15 },
} satisfies Record<string, StateConfig>;

export type DotMatrixState = keyof typeof STATES;

export type DotMatrixProps = Omit<ComponentProps<'span'>, 'children'> & {
  state?: DotMatrixState;
  label?: string;
};

/* The blink animation runs on every dot in every state (static states set hi = lo) and the registered hi/lo custom properties carry a transition, because removing or adding an animation never triggers a CSS transition on the animated property itself; transitioning the amplitude bounds is what makes state changes cross-fade. */
const DOT_MATRIX_CSS =
  '@property --aui-dot-matrix-hi{syntax:"<number>";inherits:false;initial-value:1}@property --aui-dot-matrix-lo{syntax:"<number>";inherits:false;initial-value:0.15}@keyframes aui-dot-matrix-blink{0%,100%{opacity:var(--aui-dot-matrix-hi,1)}50%{opacity:var(--aui-dot-matrix-lo,0.15)}}';

/**
 * Tiny 5x5 dot-matrix status indicator with 20 built-in states. Dots inherit the text color and animate in state-specific patterns: twinkle, waves, ripples, sweeps, equalizer columns, and check/cross/bang glyphs. State changes cross-fade per dot.
 *
 * ```tsx
 * <DotMatrix state={isRunning ? "loading" : "success"} />
 * ```
 */
function DotMatrix({ className, state = 'loading', label, ...props }: DotMatrixProps) {
  const config: StateConfig = STATES[state];
  return (
    <span
      data-slot="dot-matrix"
      data-state={state}
      role="status"
      className={cn('inline-block size-4 shrink-0', config.color, className)}
      {...props}>
      <span className="sr-only">{label ?? state}</span>
      {/* Hoisted and deduplicated across instances by React; must live in HTML scope, inside the SVG it would be an SVG-namespace element React does not hoist. */}
      <style href="aui-dot-matrix" precedence="low">
        {DOT_MATRIX_CSS}
      </style>
      <svg aria-hidden viewBox="0 0 20 20" fill="currentColor" className="size-full">
        {DOT_INDEXES.map(i => {
          const row = Math.floor(i / GRID);
          const col = i % GRID;
          const on = !config.glyph || config.glyph.has(i);
          const hi = on ? (config.base ?? 1) : (config.dim ?? 0.15);
          const blink = on ? config.blink?.(i, row, col) : undefined;
          return (
            <circle
              key={i}
              data-slot="dot-matrix-dot"
              cx={2 + col * 4}
              cy={2 + row * 4}
              r={1.3}
              className="[transition-property:--aui-dot-matrix-hi,--aui-dot-matrix-lo,opacity] duration-300 [animation-iteration-count:infinite] [animation-name:aui-dot-matrix-blink] [animation-timing-function:ease-in-out] motion-reduce:[animation-name:none]"
              style={
                {
                  opacity: hi,
                  animationDuration: `${blink?.duration ?? 1}s`,
                  animationDelay: `${blink?.delay ?? 0}s`,
                  '--aui-dot-matrix-hi': hi,
                  '--aui-dot-matrix-lo': blink?.lo ?? hi,
                } as CSSProperties
              }
            />
          );
        })}
      </svg>
    </span>
  );
}

export { DotMatrix };
