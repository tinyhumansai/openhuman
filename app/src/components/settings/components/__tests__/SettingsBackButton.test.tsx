import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import SettingsBackButton from '../SettingsBackButton';

describe('<SettingsBackButton />', () => {
  it('renders nothing when there is no onBack handler', () => {
    const { container } = renderWithProviders(<SettingsBackButton />);
    expect(container).toBeEmptyDOMElement();
  });

  it('renders an accessible, clickable back button when onBack is provided', () => {
    const onBack = vi.fn();
    renderWithProviders(<SettingsBackButton onBack={onBack} />);

    const button = screen.getByRole('button', { name: /back/i });
    expect(button).toBeInTheDocument();

    fireEvent.click(button);
    expect(onBack).toHaveBeenCalledTimes(1);
  });
});
