import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Ghosty, type MascotFace } from './Ghosty';
import { VISEMES } from './visemes';

const FACES: MascotFace[] = [
  'sleep',
  'idle',
  'normal',
  'listening',
  'thinking',
  'confused',
  'speaking',
  'happy',
  'concerned',
];

describe('Ghosty', () => {
  it.each(FACES)('renders the %s face preset without crashing', face => {
    const { container } = render(<Ghosty face={face} />);
    const svg = container.querySelector('svg[data-face]');
    expect(svg).not.toBeNull();
    expect(svg!.getAttribute('data-face')).toBe(face);
  });

  it('renders eyebrows for states that signal worry / focus', () => {
    for (const face of ['listening', 'thinking', 'confused', 'concerned'] as MascotFace[]) {
      const { container } = render(<Ghosty face={face} />);
      expect(container.querySelector(`g[data-face-brows="${face}"]`)).not.toBeNull();
    }
  });

  it('omits eyebrows for neutral / acknowledgement states', () => {
    for (const face of ['sleep', 'idle', 'normal', 'speaking', 'happy'] as MascotFace[]) {
      const { container } = render(<Ghosty face={face} />);
      expect(container.querySelector('g[data-face-brows]')).toBeNull();
    }
  });

  it('renders a viseme-driven mouth when speaking, distinct from the rest mouth', () => {
    const { container: speaking } = render(
      <Ghosty face="speaking" viseme={VISEMES.A} idPrefix="m1" />
    );
    const { container: idle } = render(<Ghosty face="idle" idPrefix="m2" />);
    const speakingMouth = speaking.querySelector('path[data-face="speaking"]')?.getAttribute('d');
    const idleMouth = idle.querySelector('path[data-face="idle"]')?.getAttribute('d');
    expect(speakingMouth).toBeTruthy();
    expect(idleMouth).toBeTruthy();
    expect(speakingMouth).not.toBe(idleMouth);
  });

  it('respects the size override', () => {
    const { container } = render(<Ghosty face="idle" size={42} />);
    const svg = container.querySelector('svg');
    expect(svg?.getAttribute('width')).toBe('42');
    expect(svg?.getAttribute('height')).toBe('42');
  });
});
