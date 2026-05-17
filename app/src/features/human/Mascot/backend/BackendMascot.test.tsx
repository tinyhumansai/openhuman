import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { BackendMascot } from './BackendMascot';
import type { MascotDetail } from './types';

const detail: MascotDetail = {
  id: 'test',
  name: 'Test',
  version: '0.0.0',
  description: '',
  viewBox: '0 0 100 100',
  defaultState: 'idle',
  variables: [],
  visemeSlot: '#m-viseme',
  hidesOnViseme: ['m-smile'],
  states: [
    {
      id: 'idle',
      label: 'Idle',
      description: '',
      svg: `<svg viewBox="0 0 100 100"><g id="m-bob"></g><g id="m-smile" opacity="1"></g><g id="m-viseme" opacity="0"></g></svg>`,
      tween: [{ id: 'm-bob', kind: 'translateY', freq: 1, amp: 10 }],
    },
  ],
  visemes: [
    { label: 'sil', description: '', svg: '' },
    { label: 'aa', description: '', svg: '<path d="MOUTH-AA"/>' },
  ],
};

describe('BackendMascot', () => {
  afterEach(() => cleanup());

  it('renders the chosen state svg', () => {
    const { container } = render(<BackendMascot mascot={detail} />);
    expect(container.querySelector('#m-bob')).toBeTruthy();
  });

  it('shows active viseme overlay and hides resting mouth', () => {
    const { container } = render(<BackendMascot mascot={detail} viseme="aa" paused />);
    const slot = container.querySelector('#m-viseme');
    expect(slot?.getAttribute('opacity')).toBe('1');
    expect(slot?.innerHTML).toContain('MOUTH-AA');
    expect(container.querySelector('#m-smile')?.getAttribute('opacity')).toBe('0');
  });

  it('clears overlay and restores resting mouth when viseme is sil', () => {
    const { container } = render(<BackendMascot mascot={detail} viseme="sil" paused />);
    const slot = container.querySelector('#m-viseme');
    expect(slot?.getAttribute('opacity')).toBe('0');
    expect(container.querySelector('#m-smile')?.getAttribute('opacity')).toBe('1');
  });
});
