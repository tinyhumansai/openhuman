// Animated renderer for a backend-driven mascot manifest.
//
// Mount strategy:
//  - Inject the chosen state's `<svg>...</svg>` once via dangerouslySetInnerHTML
//    inside a wrapper <div>.
//  - In a rAF loop, find each tween's target element by id and set its
//    `transform` attribute directly. No SVG re-parse per frame.
//  - When `viseme` changes, run `injectViseme` on the live slot element
//    so the mouth overlay swaps without a full remount.
//
// This is the browser-side counterpart of the backend's ffmpeg renderer:
// same render-core math, same per-state SVG, same viseme map — so the
// in-app mascot animates in lockstep with the WebRTC video.
import { useEffect, useMemo, useRef } from 'react';

import { injectViseme, tweenTransform } from './renderCore';
import type { MascotDetail, MascotState } from './types';

export interface BackendMascotProps {
  mascot: MascotDetail;
  /** Active state id. Falls back to `mascot.defaultState` when unknown. */
  stateId?: string;
  /** Active viseme label, or `null` for resting mouth. */
  viseme?: string | null;
  /** CSS variable overrides for the mascot's declared variables. */
  variables?: Record<string, string>;
  /** Override SVG width/height; defaults to filling the parent. */
  size?: number | string;
  /** Pause the rAF loop — used when the mascot is offscreen or in a hidden tab. */
  paused?: boolean;
}

function pickState(mascot: MascotDetail, stateId?: string): MascotState | null {
  if (mascot.states.length === 0) return null;
  const id = stateId ?? mascot.defaultState;
  return mascot.states.find(s => s.id === id) ?? mascot.states[0];
}

export function BackendMascot({
  mascot,
  stateId,
  viseme = null,
  variables,
  size = '100%',
  paused = false,
}: BackendMascotProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const rafRef = useRef<number | null>(null);

  const state = useMemo(() => pickState(mascot, stateId), [mascot, stateId]);

  // Initial SVG mount — re-runs when the state SVG changes. Viseme is
  // injected once here so the first painted frame is correct; subsequent
  // viseme changes are handled in a separate effect that doesn't tear
  // down the SVG.
  const initialMarkup = useMemo(() => {
    if (!state) return '';
    const active = viseme && viseme !== 'sil' ? viseme : null;
    return injectViseme(state.svg, mascot, active);
    // We intentionally only re-inject on state change here; viseme effect
    // below handles in-place swaps.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mascot, state]);

  // Apply CSS variables to the wrapper so the mascot's color tokens
  // (--body-fill, --eye-color, …) cascade into the inline SVG without
  // requiring per-mascot stylesheets.
  useEffect(() => {
    const el = containerRef.current;
    if (!el || !variables) return;
    for (const [k, v] of Object.entries(variables)) {
      el.style.setProperty(k, v);
    }
  }, [variables]);

  // Live viseme swaps without remounting the SVG.
  useEffect(() => {
    const root = containerRef.current?.querySelector('svg');
    if (!root || !mascot.visemeSlot) return;
    const slotId = mascot.visemeSlot.replace(/^#/, '');
    const slot = root.querySelector<SVGGElement>(`#${CSS.escape(slotId)}`);
    if (!slot) return;
    const active = viseme && viseme !== 'sil' ? viseme : null;
    const entry = active ? mascot.visemes.find(v => v.label === active) : null;
    const inner = entry?.svg ?? '';
    const visible = inner.length > 0;
    slot.innerHTML = inner;
    slot.setAttribute('opacity', visible ? '1' : '0');
    // Hide resting-mouth elements while a viseme is active so the
    // overlay reads cleanly. Restore when the viseme clears.
    for (const id of mascot.hidesOnViseme ?? []) {
      const target = root.querySelector<SVGElement>(`#${CSS.escape(id)}`);
      if (!target) continue;
      target.setAttribute('opacity', visible ? '0' : '1');
    }
    // `state` is included so the effect re-runs when the SVG remounts
    // with a different state's structure (slot + hidesOnViseme nodes are
    // queried by id directly on the live DOM, so they're per-state).
  }, [mascot, state, viseme]);

  // rAF tween loop. Mutates `transform` attrs in place — no React re-render.
  useEffect(() => {
    if (paused || !state) return;
    const root = containerRef.current?.querySelector('svg');
    if (!root) return;
    const tweens = state.tween ?? [];
    if (tweens.length === 0) return;

    // Resolve target elements once; rAF tick just rewrites the attr.
    const targets = tweens
      .map(tw => {
        const node = root.querySelector<SVGElement>(`#${CSS.escape(tw.id)}`);
        return node ? { tw, node } : null;
      })
      .filter((x): x is { tw: (typeof tweens)[number]; node: SVGElement } => x != null);

    if (targets.length === 0) return;

    const start = window.performance.now();

    const tick = (now: number) => {
      const t = (now - start) / 1000;
      for (const { tw, node } of targets) {
        node.setAttribute('transform', tweenTransform(t, tw));
      }
      rafRef.current = window.requestAnimationFrame(tick);
    };
    rafRef.current = window.requestAnimationFrame(tick);

    return () => {
      if (rafRef.current != null) window.cancelAnimationFrame(rafRef.current);
      rafRef.current = null;
    };
  }, [state, paused]);

  if (!state) return null;

  return (
    <div
      ref={containerRef}
      style={{ width: size, height: size, display: 'inline-block' }}
      // Backend-controlled SVG, same source the WebRTC ffmpeg pipeline
      // rasterizes from. Treated as trusted.
      dangerouslySetInnerHTML={{ __html: initialMarkup }}
    />
  );
}
