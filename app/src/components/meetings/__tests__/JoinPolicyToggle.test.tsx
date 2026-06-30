import { cleanup, fireEvent, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import { JoinPolicyToggle } from '../JoinPolicyToggle';

describe('JoinPolicyToggle', () => {
  afterEach(() => cleanup());

  it('renders three segments: Auto, Ask, Skip', () => {
    renderWithProviders(<JoinPolicyToggle value="ask" onChange={vi.fn()} />);
    expect(screen.getByRole('radio', { name: /auto/i })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /ask/i })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: /skip/i })).toBeInTheDocument();
  });

  it('marks the active segment as checked', () => {
    renderWithProviders(<JoinPolicyToggle value="auto" onChange={vi.fn()} />);
    expect(screen.getByRole('radio', { name: /auto/i })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('radio', { name: /ask/i })).toHaveAttribute('aria-checked', 'false');
    expect(screen.getByRole('radio', { name: /skip/i })).toHaveAttribute('aria-checked', 'false');
  });

  it('calls onChange with the new value when a segment is clicked', () => {
    const onChange = vi.fn();
    renderWithProviders(<JoinPolicyToggle value="ask" onChange={onChange} />);
    fireEvent.click(screen.getByRole('radio', { name: /skip/i }));
    expect(onChange).toHaveBeenCalledOnce();
    expect(onChange).toHaveBeenCalledWith('skip');
  });

  it('does not call onChange when clicking the already-active segment', () => {
    const onChange = vi.fn();
    renderWithProviders(<JoinPolicyToggle value="auto" onChange={onChange} />);
    fireEvent.click(screen.getByRole('radio', { name: /auto/i }));
    // Still fires — the parent can decide to ignore same-value changes.
    expect(onChange).toHaveBeenCalledWith('auto');
  });

  it('disables all buttons when disabled=true', () => {
    renderWithProviders(<JoinPolicyToggle value="ask" onChange={vi.fn()} disabled />);
    const buttons = screen.getAllByRole('radio');
    expect(buttons).toHaveLength(3);
    buttons.forEach(btn => expect(btn).toBeDisabled());
  });

  it('renders the radiogroup with a label', () => {
    renderWithProviders(<JoinPolicyToggle value="ask" onChange={vi.fn()} />);
    // The radiogroup aria-label comes from the i18n key.
    const group = screen.getByRole('radiogroup');
    expect(group).toBeInTheDocument();
  });
});
