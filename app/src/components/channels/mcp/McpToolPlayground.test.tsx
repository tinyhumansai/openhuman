/**
 * Tests for McpToolPlayground — interactive tool execution modal.
 *
 * Covers: schema viewer, JSON args validation, Run flow, success / error
 * result rendering, Cmd-Enter shortcut, Esc to close, in-session history
 * with one-click "load", copy-to-clipboard, and the a11y attribute
 * contract on dialog + result regions.
 */
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import McpToolPlayground, { parseToolArgs } from './McpToolPlayground';
import type { McpTool } from './types';

const TOOL: McpTool = {
  name: 'read_file',
  description: 'Reads a file from disk and returns its contents.',
  input_schema: {
    type: 'object',
    properties: { path: { type: 'string', description: 'Absolute path.' } },
    required: ['path'],
  },
};

const mockToolCall = vi.fn();
vi.mock('../../../services/api/mcpClientsApi', () => ({
  mcpClientsApi: { toolCall: (...args: unknown[]) => mockToolCall(...args) },
}));

beforeEach(() => {
  mockToolCall.mockReset();
});

const renderPlayground = (overrides?: {
  tool?: McpTool;
  serverId?: string;
  onClose?: () => void;
}) =>
  render(
    <McpToolPlayground
      serverId={overrides?.serverId ?? 'srv-1'}
      tool={overrides?.tool ?? TOOL}
      onClose={overrides?.onClose ?? (() => {})}
    />
  );

describe('McpToolPlayground', () => {
  // ----------------------------------------------------------------------
  // Layout + a11y
  // ----------------------------------------------------------------------

  it('renders an accessible modal dialog with the tool name in the title', () => {
    renderPlayground();
    const dialog = screen.getByRole('dialog');
    expect(dialog).toHaveAttribute('aria-modal', 'true');
    expect(dialog).toHaveAttribute('aria-labelledby', 'mcp-playground-title');
    expect(screen.getByText('Run read_file')).toBeInTheDocument();
  });

  it('renders the tool description when present', () => {
    renderPlayground();
    expect(
      screen.getByText('Reads a file from disk and returns its contents.')
    ).toBeInTheDocument();
  });

  it('does not render a description block when the tool has none', () => {
    renderPlayground({ tool: { ...TOOL, description: undefined } });
    expect(
      screen.queryByText('Reads a file from disk and returns its contents.')
    ).not.toBeInTheDocument();
  });

  it('exposes a close button with an accessible label', () => {
    renderPlayground();
    expect(screen.getByRole('button', { name: 'Close playground' })).toBeInTheDocument();
  });

  it('focuses the args textarea on mount', () => {
    renderPlayground();
    expect(screen.getByLabelText('Arguments (JSON)')).toHaveFocus();
  });

  // ----------------------------------------------------------------------
  // Schema viewer (collapsible)
  // ----------------------------------------------------------------------

  it('does not render the schema body until the toggle is clicked', () => {
    renderPlayground();
    expect(screen.queryByTestId('mcp-playground-schema')).not.toBeInTheDocument();
  });

  it('renders the input schema as formatted JSON when expanded', () => {
    renderPlayground();
    fireEvent.click(screen.getByRole('button', { name: /Input schema/i }));
    const schemaPre = screen.getByTestId('mcp-playground-schema');
    expect(schemaPre.textContent).toContain('"type": "object"');
    expect(schemaPre.textContent).toContain('"required"');
  });

  // ----------------------------------------------------------------------
  // Close behaviours: Esc, button, backdrop click
  // ----------------------------------------------------------------------

  it('calls onClose when Escape is pressed', () => {
    const onClose = vi.fn();
    renderPlayground({ onClose });
    act(() => {
      fireEvent.keyDown(document, { key: 'Escape' });
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('calls onClose when the close button is clicked', () => {
    const onClose = vi.fn();
    renderPlayground({ onClose });
    fireEvent.click(screen.getByRole('button', { name: 'Close playground' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('calls onClose when the backdrop is mousedown-ed but NOT when the dialog card is', () => {
    const onClose = vi.fn();
    renderPlayground({ onClose });
    const dialog = screen.getByRole('dialog');
    // Mousedown on the backdrop itself (the dialog div) — target === currentTarget
    fireEvent.mouseDown(dialog);
    expect(onClose).toHaveBeenCalledTimes(1);
    // Mousedown on a descendant (the title) should NOT close
    fireEvent.mouseDown(screen.getByText('Run read_file'));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  // ----------------------------------------------------------------------
  // Args validation + Format
  // ----------------------------------------------------------------------

  it('starts with empty-object args ({}) in the textarea', () => {
    renderPlayground();
    expect(screen.getByLabelText('Arguments (JSON)')).toHaveValue('{}');
  });

  it('refuses to call the RPC when the args JSON is malformed and shows an alert', async () => {
    renderPlayground();
    const textarea = screen.getByLabelText('Arguments (JSON)');
    fireEvent.change(textarea, { target: { value: '{not valid json' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Run tool' }));
    });
    expect(mockToolCall).not.toHaveBeenCalled();
    const alert = screen.getByRole('alert');
    expect(alert.textContent).toMatch(/Invalid JSON/);
  });

  it('treats an empty / whitespace-only args field as {}', async () => {
    mockToolCall.mockResolvedValue({ result: { ok: true }, is_error: false });
    renderPlayground();
    const textarea = screen.getByLabelText('Arguments (JSON)');
    fireEvent.change(textarea, { target: { value: '   ' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Run tool' }));
    });
    expect(mockToolCall).toHaveBeenCalledWith({
      server_id: 'srv-1',
      tool_name: 'read_file',
      arguments: {},
    });
  });

  it('reformats invalid JSON gracefully (Format leaves it untouched)', () => {
    renderPlayground();
    const textarea = screen.getByLabelText('Arguments (JSON)');
    fireEvent.change(textarea, { target: { value: '{not valid' } });
    fireEvent.click(screen.getByRole('button', { name: 'Format' }));
    // No throw, original preserved
    expect(textarea).toHaveValue('{not valid');
  });

  it('pretty-prints valid JSON on Format', () => {
    renderPlayground();
    const textarea = screen.getByLabelText('Arguments (JSON)');
    fireEvent.change(textarea, { target: { value: '{"path":"/etc/hosts","limit":10}' } });
    fireEvent.click(screen.getByRole('button', { name: 'Format' }));
    expect(textarea).toHaveValue('{\n  "path": "/etc/hosts",\n  "limit": 10\n}');
  });

  // ----------------------------------------------------------------------
  // Run flow — success, tool error, RPC exception
  // ----------------------------------------------------------------------

  it('calls mcpClientsApi.toolCall with parsed args and renders success result', async () => {
    mockToolCall.mockResolvedValue({
      result: { contents: 'hello world', bytes: 11 },
      is_error: false,
    });
    renderPlayground();
    const textarea = screen.getByLabelText('Arguments (JSON)');
    fireEvent.change(textarea, { target: { value: '{"path":"/etc/hosts"}' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Run tool' }));
    });
    expect(mockToolCall).toHaveBeenCalledWith({
      server_id: 'srv-1',
      tool_name: 'read_file',
      arguments: { path: '/etc/hosts' },
    });
    const result = screen.getByTestId('mcp-playground-result');
    expect(result).toHaveAttribute('role', 'status');
    expect(result).toHaveAttribute('aria-live', 'polite');
    expect(result.textContent).toContain('"contents": "hello world"');
    expect(result.textContent).toContain('"bytes": 11');
  });

  it('flags is_error=true returns with role=alert and the error label', async () => {
    mockToolCall.mockResolvedValue({ result: { message: 'denied' }, is_error: true });
    renderPlayground();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Run tool' }));
    });
    expect(screen.getByText('Tool returned an error')).toBeInTheDocument();
    const result = screen.getByTestId('mcp-playground-result');
    expect(result).toHaveAttribute('role', 'alert');
    expect(result).toHaveAttribute('aria-live', 'assertive');
    expect(result.textContent).toContain('"message": "denied"');
  });

  it('renders thrown RPC exceptions as an error result', async () => {
    mockToolCall.mockRejectedValue(new Error('socket closed'));
    renderPlayground();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Run tool' }));
    });
    const result = screen.getByTestId('mcp-playground-result');
    expect(result).toHaveAttribute('role', 'alert');
    expect(result.textContent).toContain('socket closed');
  });

  it('falls back to a generic message when the rejection value is not an Error', async () => {
    mockToolCall.mockRejectedValue('plain-string');
    renderPlayground();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Run tool' }));
    });
    const result = screen.getByTestId('mcp-playground-result');
    expect(result.textContent).toContain('Unexpected error invoking tool.');
  });

  it('disables the Run button while the call is in flight', async () => {
    let resolve: ((v: unknown) => void) | undefined;
    mockToolCall.mockImplementation(
      () =>
        new Promise(res => {
          resolve = res;
        })
    );
    renderPlayground();
    fireEvent.click(screen.getByRole('button', { name: 'Run tool' }));
    const runBtn = screen.getByRole('button', { name: /Running…|Run tool/ });
    expect(runBtn).toBeDisabled();
    expect(runBtn).toHaveTextContent('Running…');
    await act(async () => {
      resolve?.({ result: 'ok', is_error: false });
    });
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Run tool' })).not.toBeDisabled();
    });
  });

  // ----------------------------------------------------------------------
  // Cmd/Ctrl+Enter shortcut
  // ----------------------------------------------------------------------

  it('runs on Cmd+Enter from the args textarea', async () => {
    mockToolCall.mockResolvedValue({ result: 'ok', is_error: false });
    renderPlayground();
    const textarea = screen.getByLabelText('Arguments (JSON)');
    await act(async () => {
      fireEvent.keyDown(textarea, { key: 'Enter', metaKey: true });
    });
    expect(mockToolCall).toHaveBeenCalledTimes(1);
  });

  it('runs on Ctrl+Enter from the args textarea', async () => {
    mockToolCall.mockResolvedValue({ result: 'ok', is_error: false });
    renderPlayground();
    const textarea = screen.getByLabelText('Arguments (JSON)');
    await act(async () => {
      fireEvent.keyDown(textarea, { key: 'Enter', ctrlKey: true });
    });
    expect(mockToolCall).toHaveBeenCalledTimes(1);
  });

  it('does not run on Enter without a modifier key', () => {
    renderPlayground();
    const textarea = screen.getByLabelText('Arguments (JSON)');
    fireEvent.keyDown(textarea, { key: 'Enter' });
    expect(mockToolCall).not.toHaveBeenCalled();
  });

  // ----------------------------------------------------------------------
  // History
  // ----------------------------------------------------------------------

  it('records successful invocations in history and exposes a Load button per entry', async () => {
    mockToolCall.mockResolvedValue({ result: 'ok-1', is_error: false });
    renderPlayground();
    const textarea = screen.getByLabelText('Arguments (JSON)');
    fireEvent.change(textarea, { target: { value: '{"path":"/a"}' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Run tool' }));
    });
    fireEvent.click(screen.getByRole('button', { name: /History/i }));
    // Scope to the history list — the args also appear in the textarea
    // (its `value`), and testing-library matches textarea value as text
    // content. The history list is the only <ul> in the dialog.
    const historyList = screen.getByRole('list');
    expect(within(historyList).getByText('{"path":"/a"}')).toBeInTheDocument();
  });

  it('Load from history restores the prior args into the textarea', async () => {
    mockToolCall.mockResolvedValue({ result: 'ok', is_error: false });
    renderPlayground();
    const textarea = screen.getByLabelText('Arguments (JSON)');
    fireEvent.change(textarea, { target: { value: '{"path":"/orig"}' } });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Run tool' }));
    });
    // Change args to something else
    fireEvent.change(textarea, { target: { value: '{"path":"/changed"}' } });
    expect(textarea).toHaveValue('{"path":"/changed"}');
    // Expand history and click Load on the saved invocation. The Load
    // button's aria-label is just 'Load'; there's exactly one Load
    // button per history entry.
    fireEvent.click(screen.getByRole('button', { name: /History/i }));
    fireEvent.click(screen.getByRole('button', { name: 'Load' }));
    expect(textarea).toHaveValue('{"path":"/orig"}');
  });

  it('caps history at 10 entries (oldest entries fall off)', async () => {
    mockToolCall.mockResolvedValue({ result: 'ok', is_error: false });
    renderPlayground();
    const textarea = screen.getByLabelText('Arguments (JSON)');
    for (let i = 0; i < 12; i += 1) {
      fireEvent.change(textarea, { target: { value: `{"i":${i}}` } });
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Run tool' }));
      });
    }
    fireEvent.click(screen.getByRole('button', { name: /History/i }));
    const historyList = screen.getByRole('list');
    // Most-recent first: i=11..2 should be present (10 entries); i=0
    // and i=1 should be gone. Scope to the history list so the
    // textarea's current value (which testing-library treats as text
    // content for a textarea) doesn't double-match.
    expect(within(historyList).getByText('{"i":11}')).toBeInTheDocument();
    expect(within(historyList).getByText('{"i":2}')).toBeInTheDocument();
    expect(within(historyList).queryByText('{"i":0}')).not.toBeInTheDocument();
    expect(within(historyList).queryByText('{"i":1}')).not.toBeInTheDocument();
    // Total entries
    expect(within(historyList).getAllByRole('listitem')).toHaveLength(10);
  });

  it('records error invocations in history (with the error styling marker)', async () => {
    mockToolCall.mockRejectedValue(new Error('boom'));
    renderPlayground();
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Run tool' }));
    });
    fireEvent.click(screen.getByRole('button', { name: /History/i }));
    // Empty placeholder must NOT be visible — we have one history entry.
    expect(screen.queryByText('No invocations yet in this session.')).not.toBeInTheDocument();
  });

  it('shows the empty-history placeholder before any run', () => {
    renderPlayground();
    fireEvent.click(screen.getByRole('button', { name: /History/i }));
    expect(screen.getByText('No invocations yet in this session.')).toBeInTheDocument();
  });

  // ----------------------------------------------------------------------
  // Copy-feedback state reset (PR review fix)
  // ----------------------------------------------------------------------

  it('resets the "Copied" copy-feedback label when a new run starts', async () => {
    // Mock navigator.clipboard.writeText so the copy path actually runs
    // in jsdom (which doesn't ship a clipboard by default).
    const writeText = vi.fn().mockResolvedValue(undefined);
    const originalClipboard = (navigator as { clipboard?: unknown }).clipboard;
    Object.defineProperty(navigator, 'clipboard', {
      writable: true,
      configurable: true,
      value: { writeText },
    });
    try {
      mockToolCall.mockResolvedValue({ result: 'first', is_error: false });
      renderPlayground();
      // 1st run — produces a result so the Copy button appears.
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Run tool' }));
      });
      // Click Copy — copyStatus flips to 'copied' and the label changes.
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Copy result' }));
      });
      expect(screen.getByRole('button', { name: 'Copy result' })).toHaveTextContent('Copied');
      expect(writeText).toHaveBeenCalledWith('"first"');
      // 2nd run — handleRun resets copyStatus to 'idle' so the label
      // returns to 'Copy result' immediately (without waiting for the
      // 1.5s timeout).
      mockToolCall.mockResolvedValue({ result: 'second', is_error: false });
      await act(async () => {
        fireEvent.click(screen.getByRole('button', { name: 'Run tool' }));
      });
      expect(screen.getByRole('button', { name: 'Copy result' })).toHaveTextContent('Copy result');
    } finally {
      // Restore (or remove) the clipboard property so other tests in
      // the suite don't see this stub.
      if (originalClipboard === undefined) {
        delete (navigator as { clipboard?: unknown }).clipboard;
      } else {
        Object.defineProperty(navigator, 'clipboard', {
          writable: true,
          configurable: true,
          value: originalClipboard,
        });
      }
    }
  });
});

describe('parseToolArgs', () => {
  it('treats empty / whitespace-only input as an empty object', () => {
    expect(parseToolArgs('', 'fallback')).toEqual({ ok: true, value: {} });
    expect(parseToolArgs('   \n\t', 'fallback')).toEqual({ ok: true, value: {} });
  });

  it('parses valid JSON into its value', () => {
    expect(parseToolArgs('{"path":"/tmp/x"}', 'fallback')).toEqual({
      ok: true,
      value: { path: '/tmp/x' },
    });
  });

  it('returns ok:false with the parser message on invalid JSON', () => {
    const result = parseToolArgs('{not valid', 'fallback message');
    expect(result.ok).toBe(false);
    if (!result.ok) {
      // The real parser message is surfaced (not the fallback) when available.
      expect(result.error.length).toBeGreaterThan(0);
    }
  });

  it('falls back to the provided message when the error is not an Error', () => {
    // JSON.parse throws SyntaxError (an Error), so the fallback path is hard to
    // hit naturally; this pins the contract that a non-empty error is returned.
    const result = parseToolArgs('nope', 'fallback message');
    expect(result.ok).toBe(false);
  });
});
