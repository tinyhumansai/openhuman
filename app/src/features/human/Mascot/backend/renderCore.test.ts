// Behavior parity with backend `src/services/mascots/__tests__/render-core.test.ts`.
// If this test diverges from the upstream JS file, the WebRTC stream and
// the in-app render will fall out of lockstep.
import { describe, expect, it } from 'vitest';

import {
  composeFrameSvg,
  hideRestingMouth,
  injectViseme,
  setTransformOnId,
  tweenTransform,
} from './renderCore';
import type { MascotDetail, MascotState, MascotTween } from './types';

const baseSvg = `
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000">
  <g id="m-bob"><path d="M0 0"/></g>
  <g id="m-left-arm"><path d="M1 1"/></g>
  <g id="m-eye-blink"><path d="M2 2"/></g>
  <g id="m-smile" opacity="1.000"><path d="M3 3"/></g>
  <path id="m-hmm" d="M4 4" stroke="#000" fill="none" opacity="0.5"/>
  <g id="m-viseme" opacity="0"></g>
</svg>`.trim();

const visemes = [
  { label: 'sil', description: '', svg: '' },
  { label: 'aa', description: '', svg: '<path d="MOUTH-AA"/>' },
];

const mascot: Pick<MascotDetail, 'visemeSlot' | 'visemes' | 'hidesOnViseme'> = {
  visemeSlot: '#m-viseme',
  visemes,
  hidesOnViseme: ['m-smile', 'm-hmm'],
};

describe('renderCore', () => {
  describe('tweenTransform', () => {
    it('translateY: amplitude * sin(2π·freq·(t+phase))', () => {
      expect(tweenTransform(0, { id: 'x', kind: 'translateY', amp: 14, freq: 1 })).toBe(
        'translate(0 0.00)'
      );
      expect(tweenTransform(0.25, { id: 'x', kind: 'translateY', amp: 14, freq: 1 })).toBe(
        'translate(0 14.00)'
      );
    });

    it('rotate: includes pivot in the matrix call', () => {
      expect(
        tweenTransform(0.25, { id: 'x', kind: 'rotate', amp: 30, freq: 1, pivot: [100, 200] })
      ).toBe('rotate(30.00 100 200)');
    });

    it('blink: closed during the duration window, open otherwise', () => {
      const tw: MascotTween = { id: 'x', kind: 'blink', period: 2, duration: 0.2, pivot: [10, 20] };
      expect(tweenTransform(0, tw)).toContain('scale(1 1.000)');
      expect(tweenTransform(1, tw)).toContain('scale(1 0.120)');
    });

    it('blink respects custom closed value', () => {
      const tw: MascotTween = {
        id: 'x',
        kind: 'blink',
        period: 2,
        duration: 0.2,
        pivot: [0, 0],
        closed: 0.5,
      };
      expect(tweenTransform(1, tw)).toContain('scale(1 0.500)');
    });

    it('uses sane defaults when amp/freq/pivot are omitted', () => {
      expect(tweenTransform(1, { id: 'x', kind: 'translateY' })).toBe('translate(0 0.00)');
      expect(tweenTransform(0, { id: 'x', kind: 'rotate' })).toBe('rotate(0.00 0 0)');
    });
  });

  describe('injectViseme', () => {
    it('rewrites slot opacity to 1 and injects markup for a known label', () => {
      const out = injectViseme(baseSvg, mascot, 'aa');
      expect(out).toContain('<g id="m-viseme" opacity="1"><path d="MOUTH-AA"/></g>');
    });

    it('leaves slot opacity=0 with empty inner for null label', () => {
      const out = injectViseme(baseSvg, mascot, null);
      expect(out).toContain('<g id="m-viseme" opacity="0"></g>');
    });

    it('treats "sil" as silence (slot stays hidden)', () => {
      const out = injectViseme(baseSvg, mascot, 'sil');
      expect(out).toContain('<g id="m-viseme" opacity="0"></g>');
    });

    it('falls through to no-op for an unknown label', () => {
      const out = injectViseme(baseSvg, mascot, 'unknown');
      expect(out).toContain('<g id="m-viseme" opacity="0"></g>');
    });

    it('returns input unchanged when visemeSlot is missing', () => {
      const out = injectViseme(baseSvg, { visemes } as never, 'aa');
      expect(out).toBe(baseSvg);
    });

    it('returns input unchanged when slot id is not in the svg', () => {
      const out = injectViseme('<svg></svg>', mascot, 'aa');
      expect(out).toBe('<svg></svg>');
    });
  });

  describe('setTransformOnId', () => {
    it('adds a fresh transform attribute', () => {
      const out = setTransformOnId('<g id="x"></g>', 'x', 'rotate(45)');
      expect(out).toBe('<g id="x" transform="rotate(45)"></g>');
    });

    it('replaces an existing transform', () => {
      const out = setTransformOnId(
        '<g id="x" transform="translate(0 0)" opacity="1"></g>',
        'x',
        'translate(0 5)'
      );
      expect(out).toContain('transform="translate(0 5)"');
      expect(out).not.toContain('translate(0 0)');
      expect(out).toContain('opacity="1"');
    });

    it('preserves self-closing slash for void-style elements', () => {
      const out = setTransformOnId('<path id="x" d="M0 0" fill="none"/>', 'x', 'rotate(10)');
      expect(out).toMatch(/transform="rotate\(10\)"\s*\/>/);
    });

    it('is a no-op when the id is not present', () => {
      expect(setTransformOnId('<g id="other"></g>', 'x', 'rotate(45)')).toBe('<g id="other"></g>');
    });
  });

  describe('hideRestingMouth', () => {
    it('sets opacity=0 on every id in hidesOnViseme', () => {
      const out = hideRestingMouth(baseSvg, mascot);
      expect(out).toContain('<g id="m-smile" opacity="0">');
      expect(out).toMatch(/<path id="m-hmm"[^>]*opacity="0"\s*\/>/);
    });

    it('is a no-op when hidesOnViseme is empty', () => {
      expect(hideRestingMouth(baseSvg, { hidesOnViseme: [] })).toBe(baseSvg);
    });
  });

  describe('composeFrameSvg', () => {
    const state: MascotState = {
      id: 'idle',
      label: 'Idle',
      description: '',
      svg: baseSvg,
      tween: [
        { id: 'm-bob', kind: 'translateY', freq: 1, amp: 10 },
        { id: 'm-left-arm', kind: 'rotate', freq: 0, amp: 7, pivot: [50, 60] },
      ],
    };

    it('applies viseme injection + smile-hiding + tween transforms', () => {
      const out = composeFrameSvg(state, mascot, 0.25, 'aa');
      expect(out).toContain('<path d="MOUTH-AA"/>');
      expect(out).toContain('<g id="m-smile" opacity="0">');
      expect(out).toMatch(/<path id="m-hmm"[^>]*opacity="0"/);
      expect(out).toContain('<g id="m-bob" transform="translate(0 10.00)">');
      expect(out).toContain('rotate(0.00 50 60)');
    });

    it('does not hide resting mouth when no viseme is active', () => {
      const out = composeFrameSvg(state, mascot, 0, null);
      expect(out).toContain('<g id="m-smile" opacity="1.000">');
    });

    it('treats "sil" as no viseme (resting mouth visible)', () => {
      const out = composeFrameSvg(state, mascot, 0, 'sil');
      expect(out).toContain('<g id="m-smile" opacity="1.000">');
    });

    it('handles state without tween gracefully', () => {
      const noTween: MascotState = { ...state, tween: undefined };
      const out = composeFrameSvg(noTween, mascot, 0, null);
      expect(out).toContain('<g id="m-bob">');
      expect(out).not.toContain('transform=');
    });
  });
});
