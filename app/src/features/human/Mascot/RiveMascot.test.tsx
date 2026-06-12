/**
 * Unit tests for RiveMascot — the bridge between the mascot's face/viseme state
 * and the `tiny_mascot.riv` state machine.
 *
 * The real `@rive-app/react-webgl2` needs a WebGL context, so we mock its hooks
 * and capture every `setValue` write keyed by view-model property path. That
 * lets us assert the component:
 *   - plays the asset's `MascotSM` state machine,
 *   - writes the right `pose` / `mouthVisemeCode` enum values, and
 *   - drives random ambient poses while idle when `idlePoseRotation` is on.
 */
import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { AMBIENT_POSES } from './riveMaps';
import { RiveMascot } from './RiveMascot';

// vi.hoisted so the registries exist before the mock factory runs. We record
// every setValue write by property path into plain arrays (no vi.fn inside the
// hoisted factory — vitest can't be required there).
const h = vi.hoisted(() => ({
  useRiveParams: null as Record<string, unknown> | null,
  enumCalls: {} as Record<string, unknown[]>,
  colorCalls: {} as Record<string, unknown[]>,
}));

vi.mock('@rive-app/react-webgl2', () => ({
  Fit: { Contain: 'contain' },
  Layout: class {
    constructor(opts: unknown) {
      Object.assign(this, opts as object);
    }
  },
  useRive: (params: Record<string, unknown>) => {
    h.useRiveParams = params;
    return { rive: {}, RiveComponent: () => null };
  },
  useViewModel: () => ({}),
  useViewModelInstance: () => ({}),
  useViewModelInstanceEnum: (path: string) => ({
    setValue: (v: string) => (h.enumCalls[path] ??= []).push(v),
    value: null,
    values: [],
  }),
  useViewModelInstanceColor: (path: string) => ({
    setValue: (v: number) => (h.colorCalls[path] ??= []).push(v),
  }),
}));

function poseCalls(): string[] {
  return (h.enumCalls['pose'] ?? []) as string[];
}
function lastViseme(): string | undefined {
  return (h.enumCalls['mouthVisemeCode'] ?? []).at(-1) as string | undefined;
}

beforeEach(() => {
  h.useRiveParams = null;
  h.enumCalls = {};
  h.colorCalls = {};
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe('RiveMascot — asset wiring', () => {
  it('plays the MascotSM state machine from the bundled asset', () => {
    render(<RiveMascot face="idle" />);
    expect(h.useRiveParams?.src).toBe('/tiny_mascot.riv');
    expect(h.useRiveParams?.stateMachines).toBe('MascotSM');
    expect(h.useRiveParams?.autoplay).toBe(true);
  });

  it('maps the face to its pose', () => {
    render(<RiveMascot face="writing" />);
    expect(poseCalls()).toContain('writing');
  });

  it('normalises the viseme code to the asset vocabulary', () => {
    const { rerender } = render(<RiveMascot face="speaking" visemeCode="O" />);
    expect(lastViseme()).toBe('oh');
    rerender(<RiveMascot face="speaking" visemeCode="E" />);
    expect(lastViseme()).toBe('E');
    rerender(<RiveMascot face="speaking" visemeCode="???" />);
    expect(lastViseme()).toBe('sil');
  });

  it('defaults the mouth to sil (closed) when no viseme is given', () => {
    render(<RiveMascot face="idle" />);
    expect(lastViseme()).toBe('sil');
  });

  it('writes primary/secondary colors only when provided', () => {
    render(<RiveMascot face="idle" primaryColor={0xff112233} secondaryColor={0xff445566} />);
    expect(h.colorCalls['primaryColor']).toEqual([0xff112233]);
    expect(h.colorCalls['secondaryColor']).toEqual([0xff445566]);
  });

  it('does not write colors when the props are omitted', () => {
    render(<RiveMascot face="idle" />);
    expect(h.colorCalls['primaryColor']).toBeUndefined();
    expect(h.colorCalls['secondaryColor']).toBeUndefined();
  });
});

describe('RiveMascot — idle pose rotation', () => {
  it('does not drift when rotation is disabled', () => {
    vi.useFakeTimers();
    render(<RiveMascot face="idle" />);
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    // Only the initial idle write; no ambient poses scheduled.
    expect(poseCalls().every(p => p === 'idle')).toBe(true);
  });

  it('drifts into a random ambient pose, holds it, then returns to idle', () => {
    vi.useFakeTimers();
    // rng=0 → shortest delays and the first ambient pose deterministically.
    vi.spyOn(Math, 'random').mockReturnValue(0);
    render(<RiveMascot face="idle" idlePoseRotation />);

    act(() => {
      vi.advanceTimersByTime(6_000); // idle dwell elapses → ambient pose
    });
    expect(poseCalls()).toContain(AMBIENT_POSES[0]);

    act(() => {
      vi.advanceTimersByTime(5_000); // hold elapses → back to idle
    });
    expect(poseCalls().at(-1)).toBe('idle');
  });

  it('stops drifting once a real activity pose takes over', () => {
    vi.useFakeTimers();
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const { rerender } = render(<RiveMascot face="idle" idlePoseRotation />);
    // Switch to an activity face before any ambient timer fires.
    rerender(<RiveMascot face="writing" idlePoseRotation />);
    const before = poseCalls().length;
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    // No ambient poses after the activity took over — last write stays 'writing'.
    expect(poseCalls().at(-1)).toBe('writing');
    // The teardown restored idle once, but nothing kept cycling afterwards.
    expect(
      poseCalls()
        .slice(before)
        .every(p => p === 'idle' || p === 'writing')
    ).toBe(true);
  });
});
