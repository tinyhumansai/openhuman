import { cleanup, fireEvent, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import { PlatformChips } from '../PlatformChips';

describe('PlatformChips', () => {
  afterEach(() => cleanup());

  it('renders all four platform chips', () => {
    const onSelect = vi.fn();
    renderWithProviders(<PlatformChips selected="gmeet" onSelect={onSelect} />);

    expect(screen.getByRole('radio', { name: /google meet/i })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /zoom/i })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /microsoft teams/i })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /webex/i })).toBeInTheDocument();
  });

  it('marks the selected platform as checked', () => {
    renderWithProviders(<PlatformChips selected="zoom" onSelect={vi.fn()} />);

    expect(screen.getByRole('radio', { name: /zoom/i })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('radio', { name: /google meet/i })).toHaveAttribute(
      'aria-checked',
      'false'
    );
  });

  it('calls onSelect with the clicked platform', () => {
    const onSelect = vi.fn();
    renderWithProviders(<PlatformChips selected="gmeet" onSelect={onSelect} />);

    fireEvent.click(screen.getByRole('radio', { name: /zoom/i }));
    expect(onSelect).toHaveBeenCalledWith('zoom');
  });

  it('does not call onSelect when disabled', () => {
    const onSelect = vi.fn();
    renderWithProviders(<PlatformChips selected="gmeet" onSelect={onSelect} disabled />);

    const zoomChip = screen.getByRole('radio', { name: /zoom/i });
    expect(zoomChip).toBeDisabled();
    fireEvent.click(zoomChip);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('is a pure selector — never renders a Connect badge', () => {
    renderWithProviders(<PlatformChips selected="gmeet" onSelect={vi.fn()} />);

    // The chips do not gate on or imply any account connection.
    expect(screen.queryByText(/^connect$/i)).not.toBeInTheDocument();
  });

  it('is keyboard accessible via Enter key', () => {
    const onSelect = vi.fn();
    renderWithProviders(<PlatformChips selected="gmeet" onSelect={onSelect} />);

    const teamsChip = screen.getByRole('radio', { name: /microsoft teams/i });
    fireEvent.keyDown(teamsChip, { key: 'Enter' });
    expect(onSelect).toHaveBeenCalledWith('teams');
  });

  it('is keyboard accessible via Space key', () => {
    const onSelect = vi.fn();
    renderWithProviders(<PlatformChips selected="gmeet" onSelect={onSelect} />);

    const webexChip = screen.getByRole('radio', { name: /webex/i });
    fireEvent.keyDown(webexChip, { key: ' ' });
    expect(onSelect).toHaveBeenCalledWith('webex');
  });
});
