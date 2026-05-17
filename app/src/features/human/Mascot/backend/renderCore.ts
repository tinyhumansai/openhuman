// TS port of backend `src/services/mascots/render-core.js`. Single source
// of truth for tween math, viseme injection, smile-hiding, and transform
// substitution. Pure functions — no DOM. The animated `BackendMascot`
// renderer applies tween transforms directly to live DOM nodes via
// rAF, but reuses these helpers for viseme injection and tests.
//
// MUST stay behavior-equivalent to the backend's JS file — covered by
// `renderCore.test.ts`, which mirrors the backend's `render-core.test.ts`.
import type { MascotDetail, MascotState, MascotTween } from './types';

type VisemeCarrier = Pick<MascotDetail, 'visemeSlot' | 'visemes'>;
type HiderCarrier = Pick<MascotDetail, 'hidesOnViseme'>;

/**
 * Escape regex metacharacters in interpolated id / attribute names so a
 * malformed backend manifest (or a future schema that allows broader id
 * characters) can't inject pattern fragments or throw at RegExp construction.
 */
function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * Compute the SVG `transform` value for a tween entry at time t (seconds).
 */
export function tweenTransform(t: number, tw: MascotTween): string {
  const phase = tw.phase ?? 0;
  if (tw.kind === 'translateY') {
    const amp = tw.amp ?? 0;
    const freq = tw.freq ?? 0;
    const y = amp * Math.sin(2 * Math.PI * freq * (t + phase));
    return `translate(0 ${y.toFixed(2)})`;
  }
  if (tw.kind === 'rotate') {
    const ramp = tw.amp ?? 0;
    const rfreq = tw.freq ?? 0;
    const deg = ramp * Math.sin(2 * Math.PI * rfreq * (t + phase));
    const rp = tw.pivot ?? [0, 0];
    return `rotate(${deg.toFixed(2)} ${rp[0]} ${rp[1]})`;
  }
  if (tw.kind === 'blink') {
    const period = tw.period ?? 2.6;
    const duration = tw.duration ?? 0.2;
    const closed = tw.closed != null ? tw.closed : 0.12;
    // Stagger so the blink doesn't fire on t=0.
    const inBlink = (t + period / 2) % period < duration;
    const sy = inBlink ? closed : 1;
    const bp = tw.pivot ?? [0, 0];
    return `translate(${bp[0]} ${bp[1]}) scale(1 ${sy.toFixed(3)}) translate(${-bp[0]} ${-bp[1]})`;
  }
  return '';
}

/** Strip `name="..."` from an opening tag string and re-append it with `value`. */
function setOpeningTagAttr(tag: string, name: string, value: string): string {
  const nameEsc = escapeRegExp(name);
  const stripped = tag.replace(new RegExp(`\\s*\\b${nameEsc}="[^"]*"`, 'g'), '');
  return stripped.replace(/(\/?>)$/, ` ${name}="${value}"$1`);
}

/**
 * Replace the inner content of <g id="visemeSlotId">…</g> with the viseme's
 * SVG fragment AND force the slot's opacity to 1 so the markup is visible.
 * When label is null/empty, restores opacity="0" and clears the slot so
 * the resting mouth (smile/hmm) shows through.
 */
export function injectViseme(svg: string, mascot: VisemeCarrier, label: string | null): string {
  if (!mascot.visemeSlot) return svg;
  const slotId = String(mascot.visemeSlot).replace(/^#/, '');
  const slotIdEsc = escapeRegExp(slotId);
  const openRe = new RegExp(`<g\\s[^>]*\\bid="${slotIdEsc}"[^>]*>`);
  const openMatch = svg.match(openRe);
  if (!openMatch) return svg;
  let inner = '';
  let visible = false;
  // 'sil' is the silence sentinel — same as null. Anything else looked
  // up against the viseme map; only non-empty markup counts as visible
  // so the resting mouth still shows when a label has no svg.
  if (label && label !== 'sil') {
    const visemes = mascot.visemes ?? [];
    for (const v of visemes) {
      if (v.label === label) {
        inner = v.svg || '';
        visible = inner.length > 0;
        break;
      }
    }
  }
  const newOpening = setOpeningTagAttr(openMatch[0], 'opacity', visible ? '1' : '0');
  const blockRe = new RegExp(`<g\\s[^>]*\\bid="${slotIdEsc}"[^>]*>[\\s\\S]*?</g>`);
  return svg.replace(blockRe, newOpening + inner + '</g>');
}

/** Generic single-attribute string set on the element with the given id. */
export function setAttrOnId(svg: string, id: string, name: string, value: string): string {
  // Lazy `[^>]*?` + greedy `\/?>` ordering avoids swallowing the `/` of
  // self-closing tags (`<path .../>`) and emitting malformed
  // `attr"/ transform="...">` output.
  const idEsc = escapeRegExp(id);
  const nameEsc = escapeRegExp(name);
  const re = new RegExp(`(<\\w+\\b[^>]*\\bid="${idEsc}")([^>]*?)(\\s*/?>)`);
  return svg.replace(re, (_full, head: string, rest: string, close: string) => {
    const stripped = rest.replace(new RegExp(`\\s*\\b${nameEsc}="[^"]*"`, 'g'), '');
    return `${head}${stripped} ${name}="${value}"${close}`;
  });
}

/** Rewrite the opening tag of element `id` to carry a fresh `transform="value"`. */
export function setTransformOnId(svg: string, id: string, value: string): string {
  return setAttrOnId(svg, id, 'transform', value);
}

/** Hide resting-mouth ids (smile, hmm, …) while a viseme overlay is active. */
export function hideRestingMouth(svg: string, mascot: HiderCarrier): string {
  const ids = mascot.hidesOnViseme ?? [];
  let out = svg;
  for (const id of ids) {
    out = setAttrOnId(out, id, 'opacity', '0');
  }
  return out;
}

/**
 * Compose a full frame SVG for the given state at time t with optional
 * viseme overlay. Pure string-pipeline parity with the backend renderer —
 * used by tests and for the once-on-mount static SVG (no rAF) path.
 */
export function composeFrameSvg(
  state: MascotState,
  mascot: VisemeCarrier & HiderCarrier,
  t: number,
  viseme: string | null
): string {
  const active = viseme && viseme !== 'sil' ? viseme : null;
  let svg = injectViseme(state.svg, mascot, active);
  if (active) svg = hideRestingMouth(svg, mascot);
  const tweens = state.tween ?? [];
  for (const tw of tweens) {
    svg = setTransformOnId(svg, tw.id, tweenTransform(t, tw));
  }
  return svg;
}
