/**
 * Tests for McpToolList — covers empty state, tool count display,
 * expand/collapse toggle, and tool name + description rendering.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import McpToolList from './McpToolList';
import type { McpTool } from './types';

const TOOLS: McpTool[] = [
  { name: 'read_file', description: 'Reads a file from disk', input_schema: {} },
  { name: 'write_file', description: 'Writes content to a file', input_schema: {} },
  { name: 'list_dir', input_schema: {} }, // no description
];

describe('McpToolList', () => {
  it('renders "No tools available" when tools list is empty', () => {
    render(<McpToolList tools={[]} />);
    expect(screen.getByText(/no tools available/i)).toBeInTheDocument();
  });

  it('shows collapsed tool count when tools are provided', () => {
    render(<McpToolList tools={TOOLS} />);
    expect(screen.getByRole('button', { name: /3 tools available/i })).toBeInTheDocument();
    // Tool names should not be visible until expanded
    expect(screen.queryByText('read_file')).not.toBeInTheDocument();
  });

  it('uses singular "tool" when count is 1', () => {
    render(<McpToolList tools={[TOOLS[0]]} />);
    expect(screen.getByRole('button', { name: /1 tool available/i })).toBeInTheDocument();
  });

  it('expands to show tool names when toggle is clicked', () => {
    render(<McpToolList tools={TOOLS} />);
    fireEvent.click(screen.getByRole('button', { name: /3 tools available/i }));
    expect(screen.getByText('read_file')).toBeInTheDocument();
    expect(screen.getByText('write_file')).toBeInTheDocument();
    expect(screen.getByText('list_dir')).toBeInTheDocument();
  });

  it('shows tool descriptions when expanded', () => {
    render(<McpToolList tools={TOOLS} />);
    fireEvent.click(screen.getByRole('button', { name: /3 tools available/i }));
    expect(screen.getByText('Reads a file from disk')).toBeInTheDocument();
    expect(screen.getByText('Writes content to a file')).toBeInTheDocument();
  });

  it('omits description element when tool has no description', () => {
    render(<McpToolList tools={[{ name: 'list_dir', input_schema: {} }]} />);
    fireEvent.click(screen.getByRole('button', { name: /1 tool available/i }));
    expect(screen.getByText('list_dir')).toBeInTheDocument();
    // No description paragraph should exist for this tool
    expect(screen.queryByText(/undefined/i)).not.toBeInTheDocument();
  });

  it('collapses tool list when toggle is clicked a second time', () => {
    render(<McpToolList tools={TOOLS} />);
    const toggle = screen.getByRole('button', { name: /3 tools available/i });

    fireEvent.click(toggle); // expand
    expect(screen.getByText('read_file')).toBeInTheDocument();

    fireEvent.click(toggle); // collapse
    expect(screen.queryByText('read_file')).not.toBeInTheDocument();
  });
});
