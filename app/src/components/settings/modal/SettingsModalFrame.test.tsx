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
});
