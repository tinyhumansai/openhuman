/**
 * Tests for the "Preview" composio filter pill introduced in issue #2283.
 *
 * Covers:
 *  - Default grid hides non-agent-ready toolkits that have no connection.
 *  - Toolkits with an existing connection always appear in the default grid.
 *  - The "Preview" pill reveals non-agent-ready toolkits and hides curated ones.
 *  - While agent-ready data is loading, all toolkits are shown (no blank flash).
 *  - On agent-ready fetch error, all toolkits are shown (graceful degradation).
 *  - Search works correctly in both default and Preview modes.
 *  - The "Preview" pill is absent when every toolkit is agent-ready.
 */
import { fireEvent, screen, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import '../../test/mockDefaultSkillStatusHooks';
import { renderWithProviders } from '../../test/test-utils';
import Skills from '../Skills';

// ── Mutable state shared across tests ─────────────────────────────────────────

let composioToolkits: string[] = [];
let composioConnectionByToolkit = new Map<
  string,
  { id: string; toolkit: string; status: string }
>();
let agentReadyState: { agentReady: Set<string>; loading: boolean; error: string | null } = {
  agentReady: new Set<string>(),
  loading: false,
  error: null,
};
let composioModeStatus = { result: { mode: 'backend', api_key_set: true }, logs: [] };
let sessionToken = 'jwt-abc';

// ── Mocks (module-level, hoisted by Vitest) ────────────────────────────────────

vi.mock('../../hooks/useChannelDefinitions', () => ({
  useChannelDefinitions: () => ({ definitions: [], loading: false, error: null }),
}));

vi.mock('../../lib/skills/skillsApi', () => ({
  installSkill: vi.fn().mockResolvedValue(undefined),
}));

vi.mock('../../lib/skills/hooks', () => ({
  useAvailableSkills: () => ({ skills: [], loading: false, refresh: vi.fn() }),
}));

vi.mock('../../lib/composio/hooks', () => ({
  useComposioIntegrations: () => ({
    toolkits: composioToolkits,
    connectionByToolkit: composioConnectionByToolkit,
    refresh: vi.fn(),
    loading: false,
    error: null,
  }),
  useAgentReadyComposioToolkits: () => agentReadyState,
}));

vi.mock('../../lib/coreState/store', async () => {
  const actual = await vi.importActual<typeof import('../../lib/coreState/store')>(
    '../../lib/coreState/store'
  );
  return { ...actual, getCoreStateSnapshot: () => ({ snapshot: { sessionToken } }) };
});

vi.mock('../../utils/tauriCommands', async () => {
  const actual = await vi.importActual<typeof import('../../utils/tauriCommands')>(
    '../../utils/tauriCommands'
  );
  return {
    ...actual,
    openhumanComposioGetMode: vi.fn(async () => composioModeStatus),
    subconsciousEscalationsDismiss: vi.fn(),
  };
});

// ── Helpers ────────────────────────────────────────────────────────────────────

/** Returns the integrations section container element. */
function getIntegrationsSection(): HTMLElement {
  const heading = screen.getByRole('heading', { name: 'Composio Integrations' });
  const section = heading.closest('.rounded-2xl');
  expect(section).not.toBeNull();
  return section as HTMLElement;
}

/** Queries all `data-testid="skill-row-composio-<slug>"` within an element. */
function getComposioSlugs(container: HTMLElement): string[] {
  return Array.from(container.querySelectorAll('[data-testid^="skill-row-composio-"]')).map(el =>
    (el as HTMLElement).dataset.testid!.replace('skill-row-composio-', '')
  );
}

// ── Setup ──────────────────────────────────────────────────────────────────────

describe('Skills page — Preview filter pill', () => {
  beforeEach(() => {
    composioToolkits = [];
    composioConnectionByToolkit = new Map();
    composioModeStatus = { result: { mode: 'backend', api_key_set: true }, logs: [] };
    sessionToken = 'jwt-abc';
    // Resolved state with two agent-ready toolkits and two preview-only ones.
    agentReadyState = { agentReady: new Set(['gmail', 'github']), loading: false, error: null };
  });

  // ── Test 1 ──────────────────────────────────────────────────────────────────
  it('default grid shows agent-ready toolkits and hides non-agent-ready unconnected ones', () => {
    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });
    const section = getIntegrationsSection();
    const slugs = getComposioSlugs(section);

    expect(slugs).toContain('gmail');
    expect(slugs).toContain('github');
    // airtable is in KNOWN_COMPOSIO_TOOLKITS but NOT agent-ready and has no
    // connection — it must be hidden from the default view.
    expect(slugs).not.toContain('airtable');
  });

  // ── Test 2 ──────────────────────────────────────────────────────────────────
  it('default grid always shows a connected toolkit even if it is not agent-ready', () => {
    // notion is NOT in the agent-ready set but has an active connection.
    composioConnectionByToolkit = new Map([
      ['notion', { id: 'ca_notion', toolkit: 'notion', status: 'ACTIVE' }],
    ]);

    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });
    const section = getIntegrationsSection();
    const slugs = getComposioSlugs(section);

    expect(slugs).toContain('notion');
    expect(slugs).not.toContain('airtable');
  });

  // ── Test 3 ──────────────────────────────────────────────────────────────────
  it('Preview pill appears and shows only non-agent-ready toolkits when selected', () => {
    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    // The Preview pill must be visible.
    const previewTab = screen.getByRole('tab', { name: /Preview/i });
    expect(previewTab).toBeInTheDocument();

    fireEvent.click(previewTab);

    const section = getIntegrationsSection();
    const slugs = getComposioSlugs(section);

    // Agent-ready toolkits must not appear in Preview mode.
    expect(slugs).not.toContain('gmail');
    expect(slugs).not.toContain('github');
    // A non-agent-ready toolkit (airtable) must appear.
    expect(slugs).toContain('airtable');
  });

  // ── Test 4 ──────────────────────────────────────────────────────────────────
  it('while agent-ready data is loading, all toolkits are shown in the default view', () => {
    agentReadyState = { agentReady: new Set(), loading: true, error: null };

    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });
    const section = getIntegrationsSection();
    const slugs = getComposioSlugs(section);

    // Non-agent-ready toolkit must still be visible (no flash of empty grid).
    expect(slugs).toContain('airtable');
    expect(slugs).toContain('gmail');
  });

  // ── Test 5 ──────────────────────────────────────────────────────────────────
  it('while agent-ready data is loading, Preview pill is hidden and integrations grid remains populated', () => {
    agentReadyState = { agentReady: new Set(), loading: true, error: null };

    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    // Preview pill must not appear until agent-ready data resolves.
    expect(screen.queryByRole('tab', { name: /Preview/i })).not.toBeInTheDocument();
    // Default grid must remain non-empty (no flash of blank grid while loading).
    const section = getIntegrationsSection();
    expect(getComposioSlugs(section).length).toBeGreaterThan(0);
  });

  // ── Test 6 ──────────────────────────────────────────────────────────────────
  it('on agent-ready fetch error, all toolkits are shown (graceful degradation)', () => {
    agentReadyState = { agentReady: new Set(), loading: false, error: 'rpc unavailable' };

    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });
    const section = getIntegrationsSection();
    const slugs = getComposioSlugs(section);

    expect(slugs).toContain('airtable');
    expect(slugs).toContain('gmail');
    // No Preview badges — we cannot tell which toolkits are non-agent-ready.
    expect(within(section).queryAllByTestId(/composio-preview-badge-/)).toHaveLength(0);
  });

  // ── Test 7 ──────────────────────────────────────────────────────────────────
  it('search query filters results in default mode', () => {
    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    const searchInput = screen.getByPlaceholderText('Search skills…');
    fireEvent.change(searchInput, { target: { value: 'gmail' } });

    const section = getIntegrationsSection();
    const slugs = getComposioSlugs(section);

    expect(slugs).toContain('gmail');
    expect(slugs).not.toContain('github');
  });

  // ── Test 8 ──────────────────────────────────────────────────────────────────
  it('search query filters results in Preview mode', () => {
    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    const previewTab = screen.getByRole('tab', { name: /Preview/i });
    fireEvent.click(previewTab);

    const searchInput = screen.getByPlaceholderText('Search skills…');
    fireEvent.change(searchInput, { target: { value: 'airtable' } });

    const section = getIntegrationsSection();
    const slugs = getComposioSlugs(section);

    expect(slugs).toContain('airtable');
    // Other non-agent-ready toolkits filtered out by search.
    expect(slugs).not.toContain('notion');
  });

  // ── Test 9 ──────────────────────────────────────────────────────────────────
  it('Preview pill does not appear when every catalog toolkit is agent-ready', () => {
    // Mark every KNOWN toolkit as agent-ready by providing a wildcard check.
    // We achieve this by setting agentReady to a very large set — in practice
    // we make loading=false, error=null, and every toolkit appear in agentReady
    // by stubbing it with a custom has() implementation.
    const allAgentReady = {
      has: () => true,
      *[Symbol.iterator]() {},
      size: 999,
    } as unknown as Set<string>;
    agentReadyState = { agentReady: allAgentReady, loading: false, error: null };

    renderWithProviders(<Skills />, { initialEntries: ['/skills'] });

    expect(screen.queryByRole('tab', { name: /Preview/i })).not.toBeInTheDocument();
  });
});
