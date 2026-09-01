/**
 * `?tab=` resolution on the Connections page (`Skills.tsx:517-542`).
 *
 * `Skills.intelligence-tabs.test.tsx` covers six canonical values
 * (llm / voice / embeddings / search / usage / composio-key). This file covers
 * the rest of that `useMemo`, which had none:
 *
 *   - the four LEGACY aliases (`apps`, `messaging`, `tools`, `explorer`), whose
 *     own source comment says they exist "so that e.g. `/skills?tab=composio`
 *     still works after the redirect";
 *   - the default landing tab when no `?tab=` is supplied — the branch every
 *     first visit takes;
 *   - an unrecognised value, which must fall back rather than render nothing.
 *
 * Worth knowing while reading: the aliases are currently unreachable by the
 * route they were written for. `/skills` redirects with
 * `<Navigate to="/connections" replace />`, a bare path string, so the query is
 * dropped before the page ever sees it — see the pinned case in
 * `AppRoutes.connections-flows.test.tsx`. They ARE reachable by a direct
 * `/connections?tab=apps`, which is what this file exercises, so the branches
 * are live code and not dead weight.
 */
import { screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import '../../test/mockDefaultSkillStatusHooks';
import { renderWithProviders } from '../../test/test-utils';
import type { ChannelDefinition } from '../../types/channels';
import Skills from '../Skills';

// A non-empty definition list is what makes the channels group render at all.
//
// Declared through `vi.hoisted` rather than as a plain top-level `const`:
// `vi.mock` is hoisted above every declaration in the file, so a factory that
// closes over an ordinary const reads it before initialisation if the mocked
// module is pulled in during the import phase. It happens to work here — the
// hook is only called during render, inside the test body — but it is the
// documented hazard and `vi.hoisted` is the supported way to share a value
// with a factory (#5883, CodeRabbit).
const { webDef } = vi.hoisted(() => ({
  webDef: {
    id: 'web',
    display_name: 'Web',
    description: 'Chat via the built-in web UI.',
    icon: 'web',
    auth_modes: [],
    capabilities: [],
  } as ChannelDefinition,
}));

vi.mock('../../hooks/useChannelDefinitions', () => ({
  useChannelDefinitions: () => ({ definitions: [webDef], loading: false, error: null }),
}));

// Stub the two heavy tab bodies so the branch is observable without its tree.
vi.mock('../../components/skills/SkillsExplorerTab', () => ({
  default: () => <div data-testid="tab-body-skills-explorer" />,
}));
vi.mock('../../components/channels/mcp/McpServersTab', () => ({
  default: () => <div data-testid="tab-body-mcp-servers" />,
}));

vi.mock('../../lib/skills/skillsApi', () => ({
  installSkill: vi.fn().mockResolvedValue(undefined),
}));
vi.mock('../../lib/skills/hooks', () => ({
  useAvailableSkills: () => ({ skills: [], loading: false, refresh: vi.fn() }),
}));
vi.mock('../../lib/composio/hooks', () => ({
  useComposioIntegrations: () => ({
    toolkits: [],
    connectionByToolkit: new Map(),
    connectionsByToolkit: new Map(),
    refresh: vi.fn(),
    loading: false,
    error: null,
  }),
  useAgentReadyComposioToolkits: () => ({
    agentReady: new Set<string>(),
    loading: true,
    error: null,
  }),
}));
vi.mock('../../lib/coreState/store', async () => {
  const actual = await vi.importActual<typeof import('../../lib/coreState/store')>(
    '../../lib/coreState/store'
  );
  return { ...actual, getCoreStateSnapshot: () => ({ snapshot: { sessionToken: 'jwt-abc' } }) };
});
vi.mock('../../utils/tauriCommands', async () => {
  const actual = await vi.importActual<typeof import('../../utils/tauriCommands')>(
    '../../utils/tauriCommands'
  );
  return {
    ...actual,
    openhumanComposioGetMode: vi.fn(async () => ({
      result: { mode: 'backend', api_key_set: true },
      logs: [],
    })),
  };
});

const renderAt = (search: string) =>
  renderWithProviders(<Skills />, { initialEntries: [`/connections${search}`] });

/**
 * Which tab the page resolved to, read off the two-pane nav.
 *
 * `TwoPaneNav` marks exactly one row `aria-current="page"`
 * (`components/layout/TwoPaneNav.tsx:97-98`), which is a direct read of
 * `activeTab` and does not depend on whether that tab's body has data to show.
 * Asserting the body instead would couple every case to whatever mocks its
 * panel happens to need.
 */
async function selectedTab(): Promise<string> {
  const row = await waitFor(() => {
    const current = document.querySelector('[data-testid^="two-pane-nav-"][aria-current="page"]');
    expect(current).not.toBeNull();
    return current as HTMLElement;
  });
  return (row.dataset.testid ?? '').replace('two-pane-nav-', '');
}

describe('Connections ?tab= resolution — legacy aliases', () => {
  // Each of these four has a canonical successor. `Skills.tsx:537-540` maps
  // them, per its own comment, "so that e.g. `/skills?tab=composio` still works
  // after the redirect". None had a test.
  it.each([
    ['apps', 'composio'],
    ['messaging', 'channels'],
    ['tools', 'mcp'],
    ['explorer', 'skills'],
  ])('?tab=%s resolves to the %s tab', async (alias, canonical) => {
    renderAt(`?tab=${alias}`);
    expect(await selectedTab()).toBe(canonical);
  });

  it('a legacy alias does NOT silently fall through to Welcome', async () => {
    // The failure this guards is invisible: drop the alias table and every one
    // of the four still renders a perfectly good page — the wrong one.
    renderAt('?tab=apps');
    expect(await selectedTab()).toBe('composio');
    expect(screen.queryByTestId('connections-welcome')).not.toBeInTheDocument();
  });

  it('the Composio apps tab actually renders its body, not just the nav state', async () => {
    renderAt('?tab=apps');
    await waitFor(() => {
      expect(screen.getByTestId('composio-integrations-card')).toBeInTheDocument();
    });
  });
});

describe('Connections ?tab= resolution — canonical values', () => {
  it.each([['composio'], ['channels'], ['mcp'], ['skills'], ['wallet']])(
    '?tab=%s passes through unchanged',
    async tab => {
      renderAt(`?tab=${tab}`);
      expect(await selectedTab()).toBe(tab);
    }
  );

  it('?tab=welcome renders the landing overview', async () => {
    // `welcome` is the one value with no nav row of its own -- it is the
    // page's landing overview, rendered by `PageWelcome` at Skills.tsx:1037.
    renderAt('?tab=welcome');
    await waitFor(() => {
      expect(screen.getByTestId('connections-welcome')).toBeInTheDocument();
    });
  });
});

describe('Connections ?tab= resolution — default and fallback', () => {
  it.each([
    ['', 'no ?tab= at all'],
    ['?tab=definitely-not-a-tab', 'an unrecognised value'],
    ['?tab=', 'an empty value'],
    ['?tab=Composio', 'a value with the wrong case'],
  ])('%s lands on Welcome', async search => {
    renderAt(search);
    await waitFor(() => {
      expect(screen.getByTestId('connections-welcome')).toBeInTheDocument();
    });
    // ...and specifically NOT on some other tab that happened to also render.
    const current = document.querySelector('[data-testid^="two-pane-nav-"][aria-current="page"]');
    expect(current).toBeNull();
  });
});
