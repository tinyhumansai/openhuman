import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import HarnessGlyph, { type GlyphKind } from './HarnessGlyph';

describe('HarnessGlyph', () => {
  it.each<[GlyphKind, string]>([
    ['claude', 'C'],
    ['codex', 'Cx'],
    ['gemini', 'G'],
    ['openhuman', 'OH'],
  ])('renders the %s mark', (harness, label) => {
    render(<HarnessGlyph harness={harness} />);
    const glyph = screen.getByTestId('harness-glyph');
    expect(glyph).toHaveAttribute('data-harness', harness);
    expect(glyph).toHaveTextContent(label);
  });
});
