/**
 * McpServerPanel — the per-OS config-file path table, and the embedded render.
 *
 * The sibling `McpServerPanel.test.tsx` runs every case on macOS
 * (`DEFAULT_BINARY_INFO = { os: 'macos' }`), so `configFilePathFor`'s Windows
 * and Linux branches were unexecuted — lines 59-64 and 67-68, the reason the
 * panel sat at 68.18% branch coverage. Line 269, the `embedded` wrapper, was
 * unreached for the same reason: nothing rendered the panel with the prop.
 *
 * These paths are worth pinning rather than eyeballing. The path is what the
 * "Open config file" row shows and what a user is told to edit by hand; a wrong
 * one sends them to a file that does not exist, and nothing errors — the string
 * is just displayed. `configFilePathFor` is a pure `switch` with no test, and
 * three of its four clients branch on OS in different shapes: `claude-desktop`
 * and `zed` have three arms each, `cursor` has two, `codex` has one.
 */
import { cleanup, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';

const hoisted = vi.hoisted(() => ({ invoke: vi.fn(), isTauri: vi.fn(() => true) }));

vi.mock('../../../../utils/tauriCommands/common', () => ({
  isTauri: hoisted.isTauri,
  safeInvoke: (...args: unknown[]) => hoisted.invoke(...args),
}));

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

const BINARY_PATH = '/usr/local/bin/openhuman-core';

async function importPanel() {
  const mod = await import('../McpServerPanel');
  return mod.default;
}

/** Render with the resolver reporting a given OS, then wait for first paint. */
async function renderForOs(os: string, props: { embedded?: boolean } = {}) {
  hoisted.invoke.mockResolvedValue({ path: BINARY_PATH, os });
  const Panel = await importPanel();
  renderWithProviders(<Panel {...props} />);
  await waitFor(() =>
    expect(screen.getByRole('tab', { name: /Claude Desktop/i })).toBeInTheDocument()
  );
}

/** Switch to a client tab and read back the config path the panel displays. */
async function pathForClient(name: RegExp): Promise<string> {
  const tab = screen.getByRole('tab', { name });
  tab.click();
  await waitFor(() => expect(tab).toHaveAttribute('aria-selected', 'true'));
  // Anchor on the "Config file:" label and read its sibling, rather than a CSS
  // class — the panel renders several font-mono spans (tool names, the JSON
  // snippet) and a class-based selector picks up the first of them.
  const label = screen.getByText('Config file:');
  const value = label.nextElementSibling;
  expect(value, 'the config path should sit beside its label').not.toBeNull();
  return value!.textContent ?? '';
}

beforeEach(() => {
  vi.resetModules();
  hoisted.invoke.mockReset();
  hoisted.isTauri.mockReset();
  hoisted.isTauri.mockReturnValue(true);
});

describe('<McpServerPanel /> config paths per OS', () => {
  test('macOS uses the Application Support locations', async () => {
    await renderForOs('macos');
    expect(await pathForClient(/Claude Desktop/i)).toBe(
      '~/Library/Application Support/Claude/claude_desktop_config.json'
    );
    expect(await pathForClient(/Zed/i)).toBe('~/Library/Application Support/Zed/settings.json');
  });

  test('Windows uses %APPDATA% / %USERPROFILE% with backslashes', async () => {
    // The Windows arms are the ones the macOS-only sibling never reached. Note
    // these are backslash paths — asserting the exact string is the point,
    // since a forward-slash regression would still "look like a path".
    await renderForOs('windows');
    expect(await pathForClient(/Claude Desktop/i)).toBe(
      '%APPDATA%\\Claude\\claude_desktop_config.json'
    );
    expect(await pathForClient(/Cursor/i)).toBe('%USERPROFILE%\\.cursor\\mcp.json');
    expect(await pathForClient(/Zed/i)).toBe('%APPDATA%\\Zed\\settings.json');
  });

  test('Linux falls through to the XDG-style ~/.config locations', async () => {
    // Linux is the `return` after both guards — reached by any os that is
    // neither 'macos' nor 'windows'.
    await renderForOs('linux');
    expect(await pathForClient(/Claude Desktop/i)).toBe(
      '~/.config/Claude/claude_desktop_config.json'
    );
    expect(await pathForClient(/Cursor/i)).toBe('~/.cursor/mcp.json');
    expect(await pathForClient(/Zed/i)).toBe('~/.config/zed/settings.json');
  });

  test('codex is OS-independent', async () => {
    // The one client with a single arm: its path must not vary with the OS.
    // Pinning this stops a future edit from "helpfully" adding branches that
    // disagree with where codex actually reads its config.
    await renderForOs('windows');
    expect(await pathForClient(/Codex/i)).toBe('~/.codex/config.json');

    // Tear the first tree down before mounting the second — otherwise both
    // panels are in the document and the label lookup matches two nodes.
    cleanup();
    vi.resetModules();
    await renderForOs('linux');
    expect(await pathForClient(/Codex/i)).toBe('~/.codex/config.json');
  });

  test('an unrecognised OS is treated as Linux rather than blank', async () => {
    // `configFilePathFor` has no default arm per OS — anything not macOS and
    // not Windows takes the final return. A future refactor that returned
    // undefined here would render an empty path with no error.
    await renderForOs('freebsd');
    expect(await pathForClient(/Claude Desktop/i)).toBe(
      '~/.config/Claude/claude_desktop_config.json'
    );
  });

  test('renders the settings description normally but omits it when embedded', async () => {
    // Line 269 — the Connections surface embeds this panel. Both directions
    // are asserted on purpose: an earlier draft only checked that the
    // description was ABSENT when embedded, which passed even with the
    // `embedded` branch deleted, because the string it looked for was wrong
    // and so never present in either mode. A negative assertion alone cannot
    // tell "correctly hidden" from "never rendered".
    const DESCRIPTION = 'Configure external MCP clients to connect to OpenHuman';

    await renderForOs('macos');
    expect(
      screen.getByText(DESCRIPTION),
      'the settings scaffold renders the panel description'
    ).toBeInTheDocument();

    cleanup();
    vi.resetModules();

    await renderForOs('macos', { embedded: true });
    expect(screen.getByRole('tab', { name: /Claude Desktop/i })).toBeInTheDocument();
    expect(
      screen.queryByText(DESCRIPTION),
      'embedded mode uses PanelPage, which must not repeat the settings subtitle'
    ).not.toBeInTheDocument();
  });
});
