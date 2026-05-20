import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { CustomGifMascot } from './CustomGifMascot';

describe('CustomGifMascot', () => {
  it('renders a no-referrer animated avatar image', () => {
    render(<CustomGifMascot src="https://example.com/avatar.gif" face="speaking" />);

    const mascot = screen.getByTestId('custom-gif-mascot') as HTMLImageElement;
    expect(mascot).toHaveAttribute('src', 'https://example.com/avatar.gif');
    expect(mascot).toHaveAttribute('data-face', 'speaking');
    expect(mascot).toHaveAttribute('referrerpolicy', 'no-referrer');
    expect(mascot).toHaveAttribute('aria-hidden', 'true');
  });
});
