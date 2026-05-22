/**
 * Tests for McpToolList — collapsible tool list.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import McpToolList from './McpToolList';
import type { McpTool } from './types';

const TOOLS: McpTool[] = [
  { name: 'read_file', description: 'Reads a file from disk', input_schema: {} },
  { name: 'write_file', description: 'Writes data to a file', input_schema: {} },
  { name: 'list_dir', description: undefined, input_schema: {} },
];

describe('McpToolList', () => {
  it('shows empty state when no tools', () => {
    render(<McpToolList tools={[]} />);
    expect(screen.getByText('No tools available.')).toBeInTheDocument();
  });

  it('shows collapsed state with correct tool count', () => {
    render(<McpToolList tools={TOOLS} />);
    expect(screen.getByText('3 tools available')).toBeInTheDocument();
    // Tool names are not visible until expanded
    expect(screen.queryByText('read_file')).not.toBeInTheDocument();
  });

  it('shows singular "tool" for a single tool', () => {
    render(<McpToolList tools={[TOOLS[0]]} />);
    expect(screen.getByText('1 tool available')).toBeInTheDocument();
  });

  it('expands tool list when toggle button is clicked', () => {
    render(<McpToolList tools={TOOLS} />);
    fireEvent.click(screen.getByRole('button', { name: /tools available/i }));
    expect(screen.getByText('read_file')).toBeInTheDocument();
    expect(screen.getByText('write_file')).toBeInTheDocument();
    expect(screen.getByText('list_dir')).toBeInTheDocument();
  });

  it('shows tool descriptions when expanded', () => {
    render(<McpToolList tools={TOOLS} />);
    fireEvent.click(screen.getByRole('button', { name: /tools available/i }));
    expect(screen.getByText('Reads a file from disk')).toBeInTheDocument();
    expect(screen.getByText('Writes data to a file')).toBeInTheDocument();
  });

  it('does not render description paragraph when description is undefined', () => {
    render(<McpToolList tools={TOOLS} />);
    fireEvent.click(screen.getByRole('button', { name: /tools available/i }));
    // list_dir has no description — only 2 description paragraphs should exist
    const descriptions = screen
      .getAllByRole('listitem')
      .filter(item => item.querySelector('p + p'));
    // We expect 2 of the 3 items to have a description paragraph
    expect(screen.getByText('Reads a file from disk')).toBeInTheDocument();
    expect(screen.queryByText('undefined')).not.toBeInTheDocument();
    expect(descriptions).toHaveLength(2);
  });

  it('collapses again when toggle button is clicked twice', () => {
    render(<McpToolList tools={TOOLS} />);
    const btn = screen.getByRole('button', { name: /tools available/i });
    fireEvent.click(btn);
    expect(screen.getByText('read_file')).toBeInTheDocument();
    fireEvent.click(btn);
    expect(screen.queryByText('read_file')).not.toBeInTheDocument();
  });

  it('arrow rotates when expanded', () => {
    render(<McpToolList tools={TOOLS} />);
    const arrow = screen.getByText('▶');
    expect(arrow.className).not.toMatch(/rotate-90/);
    fireEvent.click(screen.getByRole('button', { name: /tools available/i }));
    expect(arrow.className).toMatch(/rotate-90/);
  });
});
