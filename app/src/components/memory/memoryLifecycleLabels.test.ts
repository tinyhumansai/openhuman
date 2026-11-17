import { describe, expect, it } from 'vitest';

import { brainSourceLabel } from './memoryLifecycleLabels';

const t = (key: string) => `t:${key}`;

describe('brainSourceLabel', () => {
  it('translates the known sources, local files included', () => {
    expect(brainSourceLabel('files', t)).toBe('t:memoryPage.brain.source.files');
    expect(brainSourceLabel('web', t)).toBe('t:memoryPage.brain.source.web');
  });

  it('shows a connected app by its own slug', () => {
    expect(brainSourceLabel('gmail', t)).toBe('gmail');
  });
});
