import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { MarkdownImage } from './MarkdownImage';

describe('MarkdownImage', () => {
  it('renders a capped, lazy, no-referrer image', () => {
    render(<MarkdownImage src="https://i.imgflip.com/82yaur.png" alt="Moye Moye" />);
    const img = screen.getByAltText('Moye Moye');
    expect(img).toHaveAttribute('src', 'https://i.imgflip.com/82yaur.png');
    expect(img).toHaveAttribute('loading', 'lazy');
    expect(img).toHaveAttribute('referrerpolicy', 'no-referrer');
    expect(img.className).toContain('max-h-72');
  });
});
