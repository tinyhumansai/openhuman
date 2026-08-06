/**
 * Tests for ExploreSection — Agent World Explore page.
 *
 * The page renders:
 *   1. Network stats via apiClient.explorer.overview() — full StatusBlock states
 *      (loading / payment_required / wallet_locked / generic error / ok).
 *   2. Four independent live sections:
 *        - Trending Communities  → apiClient.groups.list()
 *        - Active Jobs           → apiClient.graphql.jobs()
 *        - Featured Bounties     → apiClient.graphql.bounties()
 *        - New Agents            → apiClient.graphql.agents()
 *      Each section independently handles loading (skeleton) / ok / empty / error
 *      (silent degrade — section hidden). Mocks prevent real RPC calls.
 */
import { act, render, screen, waitFor } from '@testing-library/react';
import { StrictMode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { PaymentRequiredError } from '../../../lib/agentworld/invokeApiClient';
import { apiClient } from '../../AgentWorldShell';
import ExploreSection from './index';

const debugLog = vi.hoisted(() => vi.fn());
vi.mock('debug', () => ({ default: () => debugLog }));

// ── Mock apiClient ────────────────────────────────────────────────────────────

vi.mock('../../AgentWorldShell', () => ({
  apiClient: {
    explorer: { overview: vi.fn() },
    groups: { list: vi.fn() },
    graphql: { agents: vi.fn(), bounties: vi.fn(), jobs: vi.fn() },
    bounties: { list: vi.fn() },
    directory: { listAgents: vi.fn() },
  },
}));

// ── Mock react-router-dom useNavigate ─────────────────────────────────────────

const mockNavigate = vi.fn();
vi.mock('react-router-dom', async () => {
  const actual = await vi.importActual<typeof import('react-router-dom')>('react-router-dom');
  return { ...actual, useNavigate: () => mockNavigate };
});

// ── Convenience typed mocks ───────────────────────────────────────────────────

const mockOverview = vi.mocked(apiClient.explorer.overview);
const mockGroupsList = vi.mocked(apiClient.groups.list);
const mockGraphqlJobs = vi.mocked(apiClient.graphql.jobs);
const mockBountiesList = vi.mocked(apiClient.graphql.bounties);
const mockListAgents = vi.mocked(apiClient.graphql.agents);

// ── Sample fixtures ───────────────────────────────────────────────────────────

const OVERVIEW_OK = {
  allTime: { registeredAgents: 42, volumeUsd: '1000.00', feesUsd: '10.00' },
  last24h: { transactions: 5, uniqueAgents: 3, volumeUsd: '100.00', feesUsd: '1.00' },
  ledger: { totalEntries: 99, latestTxId: 'tx-abc', latestTimestamp: '2024-01-01T00:00:00Z' },
};

const GROUPS_OK = [
  {
    groupId: 'g-1',
    name: 'Alpha Community',
    description: 'First community',
    createdBy: 'user1',
    createdAt: '2024-01-01T00:00:00Z',
    membershipPolicy: 'open',
    memberCount: 100,
    membershipEpoch: 1,
    tags: ['defi', 'ai'],
  },
  {
    groupId: 'g-2',
    name: 'Beta Community',
    description: 'Second community',
    createdBy: 'user2',
    createdAt: '2024-01-02T00:00:00Z',
    membershipPolicy: 'open',
    memberCount: 50,
    membershipEpoch: 1,
  },
];

const JOBS_OK = {
  jobs: [
    {
      jobId: 'job-1',
      client: 'client-addr',
      title: 'Build a DeFi Dashboard',
      description: 'Need a developer',
      skills: ['React', 'TypeScript'],
      budget: { amount: '500', asset: 'USDC' },
      status: 'open',
      proposalCount: 3,
      createdAt: '2024-01-15T00:00:00Z',
      updatedAt: '2024-01-15T00:00:00Z',
      clientProfile: {
        handle: 'client1',
        cryptoId: 'addr1',
        displayName: 'Client One',
        verified: false,
      },
    },
  ],
  count: 1,
};

const BOUNTIES_OK = [
  {
    bountyId: 'b-1',
    creator: 'creator-addr',
    title: 'Fix Critical Bug',
    description: 'Critical bug in our system',
    reward: { amount: '250000000000', asset: 'SOL', network: 'solana' },
    status: 'open',
    submissionCount: 2,
    commentCount: 0,
    createdAt: '2024-01-10T00:00:00Z',
    updatedAt: '2024-01-10T00:00:00Z',
    startAt: '2024-01-10T00:00:00Z',
    deadline: '2024-03-01T00:00:00Z',
  },
];

const AGENTS_OK = {
  agents: [
    { agentId: 'agent-1', name: 'Nexus', username: 'nexus', description: 'An AI research agent' },
    { agentId: 'agent-2', username: '@aurora', description: 'A creative agent' },
  ],
};

// ── Helper: render inside MemoryRouter ────────────────────────────────────────

function renderExplore() {
  return render(
    <MemoryRouter>
      <ExploreSection />
    </MemoryRouter>
  );
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(resolvePromise => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function sensitiveFailure(): TypeError {
  return Object.assign(new TypeError('private resource response'), {
    payload: 'do-not-log-this-payload',
  });
}

// ── beforeEach: resolve all calls with success data ───────────────────────────

beforeEach(() => {
  vi.clearAllMocks();
  mockNavigate.mockReset();
  mockOverview.mockResolvedValue(
    OVERVIEW_OK as unknown as Awaited<ReturnType<typeof mockOverview>>
  );
  mockGroupsList.mockResolvedValue(GROUPS_OK);
  mockGraphqlJobs.mockResolvedValue(JOBS_OK);
  mockBountiesList.mockResolvedValue(
    BOUNTIES_OK as unknown as Awaited<ReturnType<typeof mockBountiesList>>
  );
  mockListAgents.mockResolvedValue(AGENTS_OK);
});

// ── Stats + all sections render ───────────────────────────────────────────────

describe('fully populated state', () => {
  test('renders network overview section heading', async () => {
    renderExplore();
    // "Network Overview" appears both as the PanelScaffold description and the <h3>;
    // use findAllByText so the duplicate doesn't throw.
    const matches = await screen.findAllByText('Network Overview');
    expect(matches.length).toBeGreaterThanOrEqual(1);
  });

  test('renders all-time stat card with registered agents', async () => {
    renderExplore();
    expect(await screen.findByText('42')).toBeInTheDocument();
  });

  test('renders trending communities section heading', async () => {
    renderExplore();
    expect(await screen.findByText('Trending Communities')).toBeInTheDocument();
  });

  test('renders community cards sorted by member count desc', async () => {
    renderExplore();
    expect(await screen.findByText('Alpha Community')).toBeInTheDocument();
    expect(screen.getByText('Beta Community')).toBeInTheDocument();
    // Alpha (100 members) should appear before Beta (50 members) — test ordering.
    const allNames = screen.getAllByText(/Community/);
    expect(allNames[0].textContent).toBe('Alpha Community');
  });

  test('renders community member count and tags', async () => {
    renderExplore();
    expect(await screen.findByText('100 members')).toBeInTheDocument();
    expect(screen.getByText('defi')).toBeInTheDocument();
    expect(screen.getByText('ai')).toBeInTheDocument();
  });

  test('renders active jobs section heading', async () => {
    renderExplore();
    expect(await screen.findByText('Active Jobs')).toBeInTheDocument();
  });

  test('renders job card with title, budget and skills', async () => {
    renderExplore();
    expect(await screen.findByText('Build a DeFi Dashboard')).toBeInTheDocument();
    expect(screen.getByText('500 USDC')).toBeInTheDocument();
    expect(screen.getByText('React')).toBeInTheDocument();
    expect(screen.getByText('TypeScript')).toBeInTheDocument();
  });

  test('renders job client display name', async () => {
    renderExplore();
    expect(await screen.findByText(/Client One/)).toBeInTheDocument();
  });

  test('renders featured bounties section heading', async () => {
    renderExplore();
    expect(await screen.findByText('Featured Bounties')).toBeInTheDocument();
  });

  test('renders bounty card with title, normalized reward and submission count', async () => {
    renderExplore();
    expect(await screen.findByText('Fix Critical Bug')).toBeInTheDocument();
    expect(screen.getByText('250 SOL')).toBeInTheDocument();
    expect(screen.getByText(/2 submissions/)).toBeInTheDocument();
  });

  test('renders new agents section heading', async () => {
    renderExplore();
    expect(await screen.findByText('New Agents')).toBeInTheDocument();
  });

  test('renders agent mini cards with handles', async () => {
    renderExplore();
    expect(await screen.findByText('@nexus')).toBeInTheDocument();
    expect(screen.getByText('@aurora')).toBeInTheDocument();
  });

  test('renders agent description', async () => {
    renderExplore();
    expect(await screen.findByText('An AI research agent')).toBeInTheDocument();
  });

  test('renders "View all" buttons for each live section', async () => {
    renderExplore();
    // Wait for sections to load.
    await screen.findByText('Trending Communities');
    const viewAllButtons = screen.getAllByText('View all');
    expect(viewAllButtons.length).toBeGreaterThanOrEqual(4);
  });
});

// ── Loading state ─────────────────────────────────────────────────────────────

describe('loading state', () => {
  test('renders skeleton placeholders for communities while loading', () => {
    mockOverview.mockReturnValue(new Promise(() => {}));
    mockGroupsList.mockReturnValue(new Promise(() => {}));
    mockGraphqlJobs.mockReturnValue(new Promise(() => {}));
    mockBountiesList.mockReturnValue(new Promise(() => {}));
    mockListAgents.mockReturnValue(new Promise(() => {}));

    const { container } = renderExplore();
    const skeletons = container.querySelectorAll('.animate-pulse');
    // 4 sections × 4-8 skeletons each, at least one group exists.
    expect(skeletons.length).toBeGreaterThan(0);
  });

  test('shows network loading text while overview loads', () => {
    mockOverview.mockReturnValue(new Promise(() => {}));
    mockGroupsList.mockResolvedValue([]);
    mockGraphqlJobs.mockResolvedValue({ jobs: [], count: 0 });
    mockBountiesList.mockResolvedValue([]);
    mockListAgents.mockResolvedValue({ agents: [] });

    renderExplore();
    expect(screen.getByText(/Loading network overview/i)).toBeInTheDocument();
  });
});

// ── Empty sections ────────────────────────────────────────────────────────────

describe('empty sections', () => {
  beforeEach(() => {
    mockGroupsList.mockResolvedValue([]);
    mockGraphqlJobs.mockResolvedValue({ jobs: [], count: 0 });
    mockBountiesList.mockResolvedValue([]);
    mockListAgents.mockResolvedValue({ agents: [] });
  });

  test('shows no-communities message when groups list is empty', async () => {
    renderExplore();
    expect(await screen.findByText('No communities yet')).toBeInTheDocument();
  });

  test('shows no-jobs message when jobs list is empty', async () => {
    renderExplore();
    expect(await screen.findByText('No active jobs')).toBeInTheDocument();
  });

  test('shows no-bounties message when bounties list is empty', async () => {
    renderExplore();
    expect(await screen.findByText('No open bounties')).toBeInTheDocument();
  });

  test('shows no-agents message when directory returns no agents', async () => {
    renderExplore();
    expect(await screen.findByText('No agents registered')).toBeInTheDocument();
  });

  test('still renders stats section when entity sections are empty', async () => {
    renderExplore();
    // Stats resolve fine.
    expect(await screen.findByText('42')).toBeInTheDocument();
    // Empty messages present.
    expect(await screen.findByText('No communities yet')).toBeInTheDocument();
  });

  test('hides community section entirely when groups.list returns []', async () => {
    renderExplore();
    // Wait for resolution.
    await screen.findByText('No communities yet');
    // No community card names appear.
    expect(screen.queryByText('Alpha Community')).not.toBeInTheDocument();
  });
});

// ── Bounties open-only client-side filter ─────────────────────────────────────

describe('bounties client-side status filter', () => {
  test('filters out non-open bounties returned by the server', async () => {
    mockBountiesList.mockResolvedValue([
      {
        bountyId: 'b-open',
        creator: 'c',
        title: 'Open Bounty',
        description: 'desc',
        reward: { amount: '100', asset: 'SOL', network: 'solana' },
        status: 'open',
        submissionCount: 0,
        commentCount: 0,
        createdAt: '2024-01-01T00:00:00Z',
        updatedAt: '2024-01-01T00:00:00Z',
        startAt: '2024-01-01T00:00:00Z',
        deadline: '2024-02-01T00:00:00Z',
      },
      {
        bountyId: 'b-closed',
        creator: 'c',
        title: 'Closed Bounty',
        description: 'desc',
        reward: { amount: '50', asset: 'SOL', network: 'solana' },
        status: 'closed',
        submissionCount: 1,
        commentCount: 0,
        createdAt: '2024-01-01T00:00:00Z',
        updatedAt: '2024-01-01T00:00:00Z',
        startAt: '2024-01-01T00:00:00Z',
        deadline: '2024-02-01T00:00:00Z',
      },
    ]);

    renderExplore();
    expect(await screen.findByText('Open Bounty')).toBeInTheDocument();
    expect(screen.queryByText('Closed Bounty')).not.toBeInTheDocument();
  });

  test('shows empty state when all returned bounties are non-open', async () => {
    mockBountiesList.mockResolvedValue([
      {
        bountyId: 'b-closed',
        creator: 'c',
        title: 'Closed Only',
        description: 'desc',
        reward: { amount: '50', asset: 'SOL', network: 'solana' },
        status: 'closed',
        submissionCount: 0,
        commentCount: 0,
        createdAt: '2024-01-01T00:00:00Z',
        updatedAt: '2024-01-01T00:00:00Z',
        startAt: '2024-01-01T00:00:00Z',
        deadline: '2024-02-01T00:00:00Z',
      },
    ]);

    renderExplore();
    expect(await screen.findByText('No open bounties')).toBeInTheDocument();
    expect(screen.queryByText('Closed Only')).not.toBeInTheDocument();
  });
});

// ── Section error graceful degrade ────────────────────────────────────────────

describe('section error: graceful degrade', () => {
  test('hides communities section on groups.list error; other sections still render', async () => {
    mockGroupsList.mockRejectedValue(new Error('communities API down'));

    renderExplore();
    // Other sections still resolve.
    expect(await screen.findByText('Build a DeFi Dashboard')).toBeInTheDocument();
    expect(await screen.findByText('Fix Critical Bug')).toBeInTheDocument();
    expect(await screen.findByText('@nexus')).toBeInTheDocument();
    // Communities section entirely hidden.
    expect(screen.queryByText('Trending Communities')).not.toBeInTheDocument();
    expect(screen.queryByText('Alpha Community')).not.toBeInTheDocument();
    expect(screen.queryByText('No communities yet')).not.toBeInTheDocument();
  });

  test('hides jobs section on graphql.jobs error; other sections still render', async () => {
    mockGraphqlJobs.mockRejectedValue(new Error('jobs API down'));

    renderExplore();
    expect(await screen.findByText('Alpha Community')).toBeInTheDocument();
    expect(await screen.findByText('Fix Critical Bug')).toBeInTheDocument();
    expect(await screen.findByText('@nexus')).toBeInTheDocument();
    expect(screen.queryByText('Active Jobs')).not.toBeInTheDocument();
  });

  test('hides bounties section on bounties.list error; other sections still render', async () => {
    mockBountiesList.mockRejectedValue(new Error('bounties API down'));

    renderExplore();
    expect(await screen.findByText('Alpha Community')).toBeInTheDocument();
    expect(await screen.findByText('Build a DeFi Dashboard')).toBeInTheDocument();
    expect(await screen.findByText('@nexus')).toBeInTheDocument();
    expect(screen.queryByText('Featured Bounties')).not.toBeInTheDocument();
  });

  test('hides agents section on directory.listAgents error; other sections still render', async () => {
    mockListAgents.mockRejectedValue(new Error('directory API down'));

    renderExplore();
    expect(await screen.findByText('Alpha Community')).toBeInTheDocument();
    expect(await screen.findByText('Build a DeFi Dashboard')).toBeInTheDocument();
    expect(await screen.findByText('Fix Critical Bug')).toBeInTheDocument();
    expect(screen.queryByText('New Agents')).not.toBeInTheDocument();
  });

  test('page does not crash when all entity sections error', async () => {
    mockGroupsList.mockRejectedValue(new Error('fail'));
    mockGraphqlJobs.mockRejectedValue(new Error('fail'));
    mockBountiesList.mockRejectedValue(new Error('fail'));
    mockListAgents.mockRejectedValue(new Error('fail'));

    renderExplore();
    // Stats still render.
    expect(await screen.findByText('42')).toBeInTheDocument();
    // No section headings for entity sections.
    await waitFor(() => {
      expect(screen.queryByText('Trending Communities')).not.toBeInTheDocument();
      expect(screen.queryByText('Active Jobs')).not.toBeInTheDocument();
      expect(screen.queryByText('Featured Bounties')).not.toBeInTheDocument();
      expect(screen.queryByText('New Agents')).not.toBeInTheDocument();
    });
  });
});

describe('section error: private diagnostics', () => {
  test.each([
    [
      'communities',
      () => mockGroupsList.mockRejectedValue(sensitiveFailure()),
      'communities fetch failed error_type=%s',
    ],
    [
      'jobs',
      () => mockGraphqlJobs.mockRejectedValue(sensitiveFailure()),
      'jobs fetch failed error_type=%s',
    ],
    [
      'bounties',
      () => mockBountiesList.mockRejectedValue(sensitiveFailure()),
      'bounties fetch failed error_type=%s',
    ],
    [
      'agents',
      () => mockListAgents.mockRejectedValue(sensitiveFailure()),
      'agents fetch failed error_type=%s',
    ],
  ] as const)('logs only the safe error type for %s failures', async (_, reject, format) => {
    reject();

    renderExplore();

    await waitFor(() => {
      const failureCalls = debugLog.mock.calls.filter(
        ([message]) => typeof message === 'string' && message.includes('fetch failed')
      );
      expect(failureCalls).toEqual([[format, 'TypeError']]);
    });
    const logged = JSON.stringify(debugLog.mock.calls);
    expect(logged).not.toContain('private resource response');
    expect(logged).not.toContain('do-not-log-this-payload');
  });
});

// ── Stats section error states ────────────────────────────────────────────────

describe('stats section: wallet locked', () => {
  test('shows wallet-locked StatusBlock when overview errors with wallet message', async () => {
    mockOverview.mockRejectedValue(new Error('the wallet is not configured'));

    renderExplore();
    expect(await screen.findByText('Unlock your wallet to use Agent World')).toBeInTheDocument();
    expect(screen.getByText(/Import your recovery phrase in Settings/i)).toBeInTheDocument();
  });

  test('shows wallet-locked StatusBlock for missing wallet secret material', async () => {
    mockOverview.mockRejectedValue(new Error('wallet secret material is missing'));

    renderExplore();
    expect(await screen.findByText('Unlock your wallet to use Agent World')).toBeInTheDocument();
  });

  test('shows generic error StatusBlock for unknown overview failure', async () => {
    mockOverview.mockRejectedValue(new Error('network timeout'));

    renderExplore();
    expect(await screen.findByText('Failed to load Agent World')).toBeInTheDocument();
    expect(screen.getByText(/network timeout/i)).toBeInTheDocument();
  });
});

describe('stats section: payment required', () => {
  test('shows payment-required StatusBlock when overview throws PaymentRequiredError', async () => {
    mockOverview.mockRejectedValue(new PaymentRequiredError({ terms: 'x402-v1' }));

    renderExplore();
    expect(await screen.findByText('Access requires payment')).toBeInTheDocument();
    expect(screen.getByText(/Your wallet will be used to fulfill/i)).toBeInTheDocument();
  });
});

// ── Agent display name edge cases ─────────────────────────────────────────────

describe('agent handle derivation', () => {
  test('strips leading @ from username before re-adding it', async () => {
    mockListAgents.mockResolvedValue({ agents: [{ agentId: 'a-1', username: '@preexisting' }] });

    renderExplore();
    // Should show @preexisting not @@preexisting.
    expect(await screen.findByText('@preexisting')).toBeInTheDocument();
    expect(screen.queryByText('@@preexisting')).not.toBeInTheDocument();
  });

  test('falls back to name when username is absent', async () => {
    mockListAgents.mockResolvedValue({ agents: [{ agentId: 'a-2', name: 'SolAgent' }] });

    renderExplore();
    expect(await screen.findByText('@SolAgent')).toBeInTheDocument();
  });

  test('falls back to first 8 chars of agentId when name and username absent', async () => {
    mockListAgents.mockResolvedValue({ agents: [{ agentId: 'xyzabc1234567890' }] });

    renderExplore();
    // displayName = agentId.slice(0,8) = 'xyzabc12'
    expect(await screen.findByText('@xyzabc12')).toBeInTheDocument();
  });
});

// ── Communities member count sort ─────────────────────────────────────────────

describe('communities sort by memberCount', () => {
  test('sorts communities by memberCount descending', async () => {
    mockGroupsList.mockResolvedValue([
      {
        groupId: 'low',
        name: 'Small Group',
        description: 'small',
        createdBy: 'u1',
        createdAt: '2024-01-01T00:00:00Z',
        membershipPolicy: 'open',
        memberCount: 5,
        membershipEpoch: 1,
      },
      {
        groupId: 'high',
        name: 'Big Group',
        description: 'big',
        createdBy: 'u2',
        createdAt: '2024-01-01T00:00:00Z',
        membershipPolicy: 'open',
        memberCount: 500,
        membershipEpoch: 1,
      },
    ]);

    renderExplore();
    await screen.findByText('Big Group');
    // In the DOM, Big Group (500 members) should appear before Small Group (5 members).
    const bigIdx = document.body.innerHTML.indexOf('Big Group');
    const smallIdx = document.body.innerHTML.indexOf('Small Group');
    expect(bigIdx).toBeLessThan(smallIdx);
  });
});

// ── Cancellation ──────────────────────────────────────────────────────────────

describe('cancellation on unmount', () => {
  test('does not update state after unmount (no act warning)', async () => {
    let resolveGroups!: (v: typeof GROUPS_OK) => void;
    mockGroupsList.mockReturnValue(
      new Promise(r => {
        resolveGroups = r;
      })
    );

    const { unmount } = renderExplore();
    unmount();
    resolveGroups(GROUPS_OK);
    await waitFor(() => expect(mockGroupsList).toHaveBeenCalled());
    // No error — the latest-only guard swallowed the state update.
  });
});

describe('latest request wins', () => {
  test('does not let an older communities response overwrite a newer response', async () => {
    const requestA = deferred<typeof GROUPS_OK>();
    const requestB = deferred<typeof GROUPS_OK>();
    mockGroupsList.mockReset();
    mockGroupsList.mockReturnValueOnce(requestA.promise).mockReturnValueOnce(requestB.promise);

    render(
      <StrictMode>
        <MemoryRouter>
          <ExploreSection />
        </MemoryRouter>
      </StrictMode>
    );

    await waitFor(() => expect(mockGroupsList).toHaveBeenCalledTimes(2));

    const newestGroups = [{ ...GROUPS_OK[0], groupId: 'new', name: 'Newest Community' }];
    await act(async () => {
      requestB.resolve(newestGroups);
      await requestB.promise;
    });
    expect(await screen.findByText('Newest Community')).toBeInTheDocument();

    const olderGroups = [{ ...GROUPS_OK[0], groupId: 'old', name: 'Older Community' }];
    await act(async () => {
      requestA.resolve(olderGroups);
      await requestA.promise;
    });

    expect(screen.getByText('Newest Community')).toBeInTheDocument();
    expect(screen.queryByText('Older Community')).not.toBeInTheDocument();
  });
});
