import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { SettingsModalFrame } from './SettingsModalFrame';

// Identity translator so we can assert on stable i18n keys.
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

describe('SettingsModalFrame', () => {
  it('renders its children', () => {
    render(
      <SettingsModalFrame onClose={vi.fn()}>
        <div data-testid="child">hello</div>
      </SettingsModalFrame>
    );
    expect(screen.getByTestId('child')).toHaveTextContent('hello');
  });

  it('calls onClose when the X button is clicked', () => {
    const onClose = vi.fn();
    render(
      <SettingsModalFrame onClose={onClose}>
        <div>body</div>
      </SettingsModalFrame>
    );
    fireEvent.click(screen.getByTestId('settings-modal-close'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('calls onClose when Escape is pressed', () => {
    const onClose = vi.fn();
    render(
      <SettingsModalFrame onClose={onClose}>
        <div>body</div>
      </SettingsModalFrame>
    );
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('calls onClose when the backdrop is clicked', () => {
    const onClose = vi.fn();
    render(
      <SettingsModalFrame onClose={onClose}>
        <div>body</div>
      </SettingsModalFrame>
    );
    fireEvent.click(screen.getByTestId('settings-modal-backdrop'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('does not call onClose when the card body is clicked', () => {
    const onClose = vi.fn();
    render(
      <SettingsModalFrame onClose={onClose}>
        <div data-testid="child">body</div>
      </SettingsModalFrame>
    );
    fireEvent.click(screen.getByTestId('child'));
    expect(onClose).not.toHaveBeenCalled();
  });

  // The card is sized by fixed CSS (80vh tall, capped at max-w-5xl), never by its
  // content — this is what keeps the modal from resizing when switching tabs.
  // Render with deliberately different-sized children and assert the size classes
  // are identical and content-independent.
  it('sizes the card with fixed, content-independent dimensions', () => {
    const tiny = render(
      <SettingsModalFrame onClose={vi.fn()}>
        <div>x</div>
      </SettingsModalFrame>
    );
    const tinyCard = tiny.getByTestId('settings-modal-card');
    expect(tinyCard.className).toContain('h-full');
    expect(tinyCard.className).toContain('w-full');
    // The card's positioning wrapper carries the fixed height + max width.
    const tinyWrapper = tinyCard.parentElement as HTMLElement;
    expect(tinyWrapper.className).toContain('h-[80vh]');
    expect(tinyWrapper.className).toContain('max-w-5xl');
    expect(tinyWrapper.className).toContain('w-full');
    tiny.unmount();

    const big = render(
      <SettingsModalFrame onClose={vi.fn()}>
        <div style={{ height: 4000, width: 4000 }}>huge</div>
      </SettingsModalFrame>
    );
    const bigWrapper = big.getByTestId('settings-modal-card').parentElement as HTMLElement;
    // Same size classes regardless of how large the panel content is.
    expect(bigWrapper.className).toBe(tinyWrapper.className);
  });
});
