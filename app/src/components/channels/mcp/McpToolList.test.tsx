/**
 * Tests for McpToolList — collapsible tool list with optional Try button.
 */
import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

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
    // Each described tool's description text is rendered exactly where
    // expected — inside its own list-item row.
    const readFileItem = screen.getByText('read_file').closest('li')!;
    const writeFileItem = screen.getByText('write_file').closest('li')!;
    expect(within(readFileItem).getByText('Reads a file from disk')).toBeInTheDocument();
    expect(within(writeFileItem).getByText('Writes data to a file')).toBeInTheDocument();
    // Behaviour-level assertion for the description-less tool: its row
    // contains only the tool name (no Try button is rendered because
    // `onTryTool` isn't passed in this test), so the row's full visible
    // text is exactly the name with no description content.
    const listDirItem = screen.getByText('list_dir').closest('li')!;
    expect(listDirItem.textContent?.trim()).toBe('list_dir');
    // And the literal string 'undefined' must never appear (would
    // indicate the conditional `{tool.description && …}` was bypassed).
    expect(screen.queryByText('undefined')).not.toBeInTheDocument();
  });

  it('collapses again when toggle button is clicked twice', () => {
    render(<McpToolList tools={TOOLS} />);
    const btn = screen.getByRole('button', { name: /tools available/i });
    fireEvent.click(btn);
    expect(screen.getByText('read_file')).toBeInTheDocument();
    fireEvent.click(btn);
    expect(screen.queryByText('read_file')).not.toBeInTheDocument();
  });

  it('shows empty state when tools is undefined (malformed prop)', () => {
    // McpToolList receives `tools` typed as McpTool[] but defensive test for runtime safety.
    // tools.length would throw if undefined; the component must guard or fall back.
    render(<McpToolList tools={undefined as unknown as McpTool[]} />);
    // Should render empty state, not crash
    expect(screen.getByText('No tools available.')).toBeInTheDocument();
  });

  it('arrow rotates when expanded', () => {
    render(<McpToolList tools={TOOLS} />);
    const arrow = screen.getByText('▶');
    expect(arrow.className).not.toMatch(/rotate-90/);
    fireEvent.click(screen.getByRole('button', { name: /tools available/i }));
    expect(arrow.className).toMatch(/rotate-90/);
  });

  // ---------------------------------------------------------------------
  // Try-button (the optional onTryTool integration with the playground)
  // ---------------------------------------------------------------------

  it('does NOT render any "Try" button when onTryTool is omitted', () => {
    render(<McpToolList tools={TOOLS} />);
    fireEvent.click(screen.getByRole('button', { name: /tools available/i }));
    expect(screen.queryByRole('button', { name: /Try/i })).not.toBeInTheDocument();
  });

  it('renders a "Try" button per tool when onTryTool is provided', () => {
    render(<McpToolList tools={TOOLS} onTryTool={() => {}} />);
    fireEvent.click(screen.getByRole('button', { name: /tools available/i }));
    // One per tool, accessible name = "Open execution playground for {name}"
    expect(
      screen.getByRole('button', { name: 'Open execution playground for read_file' })
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Open execution playground for write_file' })
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Open execution playground for list_dir' })
    ).toBeInTheDocument();
  });

  it('clicking "Try" invokes onTryTool with the corresponding tool object', () => {
    const onTryTool = vi.fn();
    render(<McpToolList tools={TOOLS} onTryTool={onTryTool} />);
    fireEvent.click(screen.getByRole('button', { name: /tools available/i }));
    fireEvent.click(
      screen.getByRole('button', { name: 'Open execution playground for write_file' })
    );
    expect(onTryTool).toHaveBeenCalledTimes(1);
    expect(onTryTool).toHaveBeenCalledWith(TOOLS[1]); // write_file
  });

  it('Try button is shown for tools without a description as well', () => {
    const onTryTool = vi.fn();
    render(<McpToolList tools={[TOOLS[2]]} onTryTool={onTryTool} />);
    fireEvent.click(screen.getByRole('button', { name: /tool available/i }));
    expect(
      screen.getByRole('button', { name: 'Open execution playground for list_dir' })
    ).toBeInTheDocument();
  });
});
