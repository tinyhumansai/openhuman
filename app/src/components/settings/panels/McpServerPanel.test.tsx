/**
 * Tests for McpServerPanel — MCP client configuration UI.
 *
 * Covers: tool list rendering, client tab selector, JSON snippet shape for
 * each client, copy-to-clipboard, binary-path error fallback, open-config
 * invoke, and Tauri-gate on the "Open Config File" button.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';

// ---------------------------------------------------------------------------
// Hoisted mocks — must be defined before any imports that trigger the module
// ---------------------------------------------------------------------------

const hoisted = vi.hoisted(() => ({ invoke: vi.fn(), isTauri: vi.fn(() => true) }));

// McpServerPanel imports `invoke` from `tauriCommands/common` (aliased from
// `safeInvoke` — see OPENHUMAN-TAURI-REACT-7 / TAURI-REACT-6). Route
// `hoisted.invoke` through `safeInvoke` so assertions on
// `hoisted.invoke.toHaveBeenCalledWith(...)` work unchanged. The
// `@tauri-apps/api/core` mock is omitted because `safeInvoke` shadows it
// for all panel call sites.
vi.mock('../../../utils/tauriCommands/common', () => ({
  isTauri: hoisted.isTauri,
  safeInvoke: (...args: unknown[]) => hoisted.invoke(...args),
}));

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateToSettings: vi.fn(),
    navigateBack: vi.fn(),
    breadcrumbs: [],
  }),
}));

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const BINARY_PATH = '/usr/local/bin/openhuman-core';
const DEFAULT_BINARY_INFO = { path: BINARY_PATH, os: 'macos' };

async function importPanel() {
  const mod = await import('./McpServerPanel');
  return mod.default;
}

function setupClipboard() {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } });
  return writeText;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('McpServerPanel — tool list', () => {
  beforeEach(() => {
    hoisted.invoke.mockReset();
    hoisted.invoke.mockResolvedValue(DEFAULT_BINARY_INFO);
    hoisted.isTauri.mockReset();
    hoisted.isTauri.mockReturnValue(true);
  });

  test('renders the panel with all 10 tool names visible', async () => {
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    const expectedTools = [
      'core.list_tools',
      'core.tool_instructions',
      'agent.list_subagents',
      'agent.run_subagent',
      'memory.search',
      'memory.recall',
      'tree.read_chunk',
      'tree.browse',
      'tree.top_entities',
      'tree.list_sources',
    ];

    for (const tool of expectedTools) {
      expect(screen.getByText(tool)).toBeInTheDocument();
    }
  });
});

describe('McpServerPanel — client tabs', () => {
  beforeEach(() => {
    hoisted.invoke.mockReset();
    hoisted.invoke.mockResolvedValue(DEFAULT_BINARY_INFO);
    hoisted.isTauri.mockReset();
    hoisted.isTauri.mockReturnValue(true);
  });

  test('renders all four client tabs', async () => {
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    expect(screen.getByRole('tab', { name: /Claude Desktop/i })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /Cursor/i })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /Codex/i })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /Zed/i })).toBeInTheDocument();
  });
});

describe('McpServerPanel — JSON snippets', () => {
  beforeEach(() => {
    hoisted.invoke.mockReset();
    hoisted.invoke.mockResolvedValue(DEFAULT_BINARY_INFO);
    hoisted.isTauri.mockReset();
    hoisted.isTauri.mockReturnValue(true);
  });

  test('Claude Desktop snippet contains mcpServers key', async () => {
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // Claude Desktop is selected by default
    await waitFor(() => {
      expect(hoisted.invoke).toHaveBeenCalledWith('mcp_resolve_binary_path');
    });

    // The snippet should be in a <pre> element
    const preEl = document.querySelector('pre');
    expect(preEl).not.toBeNull();
    const content = preEl!.textContent ?? '';
    expect(content).toContain('mcpServers');
    expect(content).toContain(BINARY_PATH);
  });

  test('Zed snippet contains context_servers key', async () => {
    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(hoisted.invoke).toHaveBeenCalledWith('mcp_resolve_binary_path');
    });

    fireEvent.click(screen.getByRole('tab', { name: /Zed/i }));

    const preEl = document.querySelector('pre');
    expect(preEl).not.toBeNull();
    const content = preEl!.textContent ?? '';
    expect(content).toContain('context_servers');
    expect(content).toContain(BINARY_PATH);
  });
});

describe('McpServerPanel — copy to clipboard', () => {
  beforeEach(() => {
    hoisted.invoke.mockReset();
    hoisted.invoke.mockResolvedValue(DEFAULT_BINARY_INFO);
    hoisted.isTauri.mockReset();
    hoisted.isTauri.mockReturnValue(true);
  });

  test('copy button calls clipboard.writeText with JSON containing the binary path', async () => {
    const writeText = setupClipboard();

    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(hoisted.invoke).toHaveBeenCalledWith('mcp_resolve_binary_path');
    });

    fireEvent.click(screen.getByRole('button', { name: /Copy to Clipboard/i }));

    await waitFor(() => {
      expect(writeText).toHaveBeenCalledTimes(1);
    });

    const written: string = writeText.mock.calls[0][0];
    expect(written).toContain(BINARY_PATH);
    expect(written).toContain('mcpServers');
  });
});

describe('McpServerPanel — binary path error', () => {
  beforeEach(() => {
    hoisted.invoke.mockReset();
    hoisted.isTauri.mockReset();
    hoisted.isTauri.mockReturnValue(true);
  });

  test('shows fallback placeholder when binary resolution fails', async () => {
    hoisted.invoke.mockRejectedValue(new Error('binary not found on disk'));

    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(screen.getByText(/OpenHuman binary not found/i)).toBeInTheDocument();
    });
  });
});

describe('McpServerPanel — open config file', () => {
  beforeEach(() => {
    hoisted.invoke.mockReset();
    hoisted.invoke.mockResolvedValue(DEFAULT_BINARY_INFO);
    hoisted.isTauri.mockReset();
    hoisted.isTauri.mockReturnValue(true);
  });

  test('Open Config File calls invoke with the active client', async () => {
    hoisted.invoke.mockImplementation((cmd: string) => {
      if (cmd === 'mcp_resolve_binary_path') return Promise.resolve(DEFAULT_BINARY_INFO);
      if (cmd === 'mcp_open_client_config') return Promise.resolve();
      return Promise.resolve();
    });

    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    await waitFor(() => {
      expect(hoisted.invoke).toHaveBeenCalledWith('mcp_resolve_binary_path');
    });

    fireEvent.click(screen.getByRole('button', { name: /Open Config File/i }));

    await waitFor(() => {
      expect(hoisted.invoke).toHaveBeenCalledWith('mcp_open_client_config', {
        client: 'claude-desktop',
      });
    });
  });

  test('Open Config File button is hidden when not in Tauri', async () => {
    hoisted.isTauri.mockReturnValue(false);

    vi.resetModules();
    const Panel = await importPanel();
    renderWithProviders(<Panel />);

    // The button should not appear
    expect(screen.queryByRole('button', { name: /Open Config File/i })).toBeNull();
  });
});
