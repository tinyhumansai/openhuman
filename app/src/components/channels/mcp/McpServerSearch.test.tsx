/**
 * Tests for McpServerSearch — controlled filter input with clear button.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import McpServerSearch from './McpServerSearch';

describe('McpServerSearch', () => {
  it('renders the search landmark with an accessible label', () => {
    render(<McpServerSearch value="" onChange={() => {}} />);
    const landmark = screen.getByRole('search', { name: 'Search installed MCP servers' });
    expect(landmark).toBeInTheDocument();
  });

  it('renders a search-type input with placeholder and aria-label', () => {
    render(<McpServerSearch value="" onChange={() => {}} />);
    const input = screen.getByRole('searchbox', { name: 'Filter installed MCP servers by name' });
    expect(input).toHaveAttribute('type', 'search');
    expect(input).toHaveAttribute('placeholder', 'Filter servers…');
  });

  it('does not render the clear button when value is empty', () => {
    render(<McpServerSearch value="" onChange={() => {}} />);
    expect(screen.queryByRole('button', { name: 'Clear filter' })).not.toBeInTheDocument();
  });

  it('renders the clear button when value is non-empty', () => {
    render(<McpServerSearch value="redis" onChange={() => {}} />);
    expect(screen.getByRole('button', { name: 'Clear filter' })).toBeInTheDocument();
  });

  it('reflects the current value as the input value', () => {
    render(<McpServerSearch value="redis" onChange={() => {}} />);
    expect(
      screen.getByRole('searchbox', { name: 'Filter installed MCP servers by name' })
    ).toHaveValue('redis');
  });

  it('fires onChange with the new value on typing', () => {
    const onChange = vi.fn();
    render(<McpServerSearch value="" onChange={onChange} />);
    const input = screen.getByRole('searchbox', { name: 'Filter installed MCP servers by name' });
    fireEvent.change(input, { target: { value: 'gh' } });
    expect(onChange).toHaveBeenCalledWith('gh');
  });

  it('fires onChange with an empty string when the clear button is clicked', () => {
    const onChange = vi.fn();
    render(<McpServerSearch value="redis" onChange={onChange} />);
    fireEvent.click(screen.getByRole('button', { name: 'Clear filter' }));
    expect(onChange).toHaveBeenCalledWith('');
  });

  it('renders the clear icon as decorative (aria-hidden)', () => {
    const { container } = render(<McpServerSearch value="x" onChange={() => {}} />);
    // The svg inside the clear button must be aria-hidden so the button's
    // own aria-label is the sole accessible name.
    const svg = container.querySelector('button[aria-label="Clear filter"] svg');
    expect(svg).not.toBeNull();
    expect(svg).toHaveAttribute('aria-hidden', 'true');
  });
});
