import { describe, expect, it } from 'vitest';

import type { MascotFace } from './Ghosty';
import {
  AMBIENT_POSES,
  FACE_TO_POSE,
  faceToPose,
  MASCOT_STATE_MACHINE,
  pickAmbientPose,
  RIVE_POSES,
  RIVE_VISEME_SET,
  type RivePose,
  toRiveVisemeCode,
} from './riveMaps';

describe('riveMaps — asset contract', () => {
  it('targets the asset state machine name', () => {
    expect(MASCOT_STATE_MACHINE).toBe('MascotSM');
  });

  it('maps every MascotFace to a pose the asset actually exposes', () => {
    const valid = new Set<string>(RIVE_POSES);
    for (const [face, pose] of Object.entries(FACE_TO_POSE)) {
      expect(valid, `${face} → ${pose}`).toContain(pose);
    }
  });

  it('faceToPose falls back to idle for an unknown face', () => {
    expect(faceToPose('not-a-face' as MascotFace)).toBe('idle');
  });
});

describe('toRiveVisemeCode', () => {
  it('only ever emits codes in the asset visme_codes enum', () => {
    const valid = new Set<string>(RIVE_VISEME_SET);
    const inputs = [
      ...RIVE_VISEME_SET,
      'I',
      'O',
      'U',
      'pp',
      'PP',
      'aa',
      'a',
      'e',
      'E',
      'm',
      'b',
      'f',
      'v',
      't',
      'd',
      'k',
      'g',
      's',
      'z',
      'n',
      'r',
      'w',
      'y',
      'silence',
      'totally-unknown',
      '???',
    ];
    for (const code of inputs) {
      expect(valid, `${code} → ${toRiveVisemeCode(code)}`).toContain(toRiveVisemeCode(code));
    }
  });

  it('normalises Oculus close vowels to the asset vocabulary', () => {
    expect(toRiveVisemeCode('I')).toBe('ih');
    expect(toRiveVisemeCode('O')).toBe('oh');
    expect(toRiveVisemeCode('U')).toBe('ou');
  });

  it('keeps the distinct E viseme instead of collapsing it to ih', () => {
    expect(toRiveVisemeCode('E')).toBe('E');
    expect(toRiveVisemeCode('e')).toBe('E');
  });

  it('is case-insensitive for consonants so the mouth never freezes', () => {
    expect(toRiveVisemeCode('pp')).toBe('PP');
    expect(toRiveVisemeCode('PP')).toBe('PP');
    expect(toRiveVisemeCode('ff')).toBe('FF');
    expect(toRiveVisemeCode('Ch')).toBe('CH');
  });

  it('falls back to sil for unrecognised codes', () => {
    expect(toRiveVisemeCode('???')).toBe('sil');
    expect(toRiveVisemeCode('unknown_code')).toBe('sil');
    expect(toRiveVisemeCode('silence')).toBe('sil');
  });
});

describe('pickAmbientPose', () => {
  it('never returns idle (the resting state it drifts away from)', () => {
    const valid = new Set<string>(AMBIENT_POSES);
    for (let i = 0; i < AMBIENT_POSES.length; i++) {
      const rng = () => i / AMBIENT_POSES.length;
      const pose = pickAmbientPose(undefined, rng);
      expect(pose).not.toBe('idle');
      expect(valid).toContain(pose);
    }
  });

  it('avoids repeating the excluded pose', () => {
    for (const exclude of AMBIENT_POSES) {
      // Sweep the rng across the whole range; none should land on `exclude`.
      for (let k = 0; k < 20; k++) {
        const rng = () => k / 20;
        expect(pickAmbientPose(exclude, rng)).not.toBe(exclude);
      }
    }
  });

  it('selects deterministically from the pool for a given rng', () => {
    const first = pickAmbientPose(undefined, () => 0);
    expect(first).toBe(AMBIENT_POSES[0]);
    const last = pickAmbientPose(undefined, () => 0.999);
    expect(last).toBe(AMBIENT_POSES[AMBIENT_POSES.length - 1]);
  });

  it('stays within bounds even when rng returns exactly 1', () => {
    const pose = pickAmbientPose(undefined, () => 1);
    expect(AMBIENT_POSES).toContain(pose as RivePose);
  });
});
