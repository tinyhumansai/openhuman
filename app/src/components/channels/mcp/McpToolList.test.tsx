/**
 * Tests for McpToolList — collapsible tool list with optional Try button.
 */
import { fireEvent, render, screen } from '@testing-library/react';
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
    // list_dir has no description — only the two described tools should
    // render their description text. (Earlier this test relied on a
    // `p + p` selector against the previous flat list structure; the
    // current row wraps the name + Try button in a div, so the
    // description is `div + p` rather than `p + p`. We assert intent
    // directly: each described tool's text is present, the
    // non-described tool's row has no description-class paragraph.)
    expect(screen.getByText('Reads a file from disk')).toBeInTheDocument();
    expect(screen.getByText('Writes data to a file')).toBeInTheDocument();
    expect(screen.queryByText('undefined')).not.toBeInTheDocument();
    // The list_dir item must NOT have a description-styled paragraph —
    // find its row and verify it has only the name paragraph.
    const listDirItem = screen.getByText('list_dir').closest('li')!;
    const descriptionPara = listDirItem.querySelector('p.text-\\[11px\\]');
    expect(descriptionPara).toBeNull();
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
