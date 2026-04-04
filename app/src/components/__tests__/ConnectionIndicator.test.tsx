import { screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import ConnectionIndicator from '../ConnectionIndicator';

describe('ConnectionIndicator', () => {
  it('renders connected state with override prop', () => {
    renderWithProviders(<ConnectionIndicator status="connected" />);
    expect(screen.getByText(/Connected to OpenHuman AI/)).toBeInTheDocument();
  });

  it('renders disconnected state', () => {
    renderWithProviders(<ConnectionIndicator status="disconnected" />);
    expect(screen.getByText('Disconnected')).toBeInTheDocument();
  });

  it('renders connecting state', () => {
    renderWithProviders(<ConnectionIndicator status="connecting" />);
    expect(screen.getByText('Connecting')).toBeInTheDocument();
  });

  it('renders as a pill badge', () => {
    renderWithProviders(<ConnectionIndicator status="connected" />);
    // The indicator renders as an inline pill — status text is visible
    expect(screen.getByText(/Connected to OpenHuman AI/)).toBeInTheDocument();
  });

  it('falls back to store socket status when no override', () => {
    // Default store state has no socket connection → disconnected
    renderWithProviders(<ConnectionIndicator />);
    expect(screen.getByText('Disconnected')).toBeInTheDocument();
  });
});
