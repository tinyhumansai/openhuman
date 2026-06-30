/**
 * Tests for JobsSection — the Agent World Jobs section (Phase 3, GraphQL).
 *
 * Covers loading / error / empty / populated states, JobStatusBadge colors,
 * client profile rendering, budget/skills display, and inline expand/collapse.
 * Optional fields: dispute and on-chain sections are covered with presence/absence tests.
 *
 * apiClient is mocked at module level; no real RPC calls are made.
 * All sample data uses generic placeholder names/IDs per project rules.
 */
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import {
  type FeedAuthor,
  type GqlJobPosting,
  type Proposal,
} from '../../lib/agentworld/invokeApiClient';
import { fetchWalletStatus } from '../../services/walletApi';
import { apiClient } from '../AgentWorldShell';
import JobsSection, { JobStatusBadge } from './JobsSection';

vi.mock('../AgentWorldShell', () => ({
  apiClient: {
    graphql: { jobs: vi.fn(), job: vi.fn() },
    jobsWrite: {
      create: vi.fn(),
      cancel: vi.fn(),
      apply: vi.fn(),
      listProposals: vi.fn(),
      getProposal: vi.fn(),
      shortlistProposal: vi.fn(),
      withdrawProposal: vi.fn(),
      select: vi.fn(),
      openDispute: vi.fn(),
      adjudicateDispute: vi.fn(),
    },
  },
}));

vi.mock('../../services/walletApi', () => ({ fetchWalletStatus: vi.fn() }));

const MY_AGENT_ID = 'my-agent-addr-111';
const sampleWalletStatus = { accounts: [{ chain: 'solana', address: MY_AGENT_ID }] };

// ── Sample data (generic placeholders) ───────────────────────────────────────

const sampleClientProfile: FeedAuthor = {
  handle: 'client-alpha',
  cryptoId: 'crypto-client-1',
  displayName: 'Client Alpha',
  avatarUrl: 'https://example.com/avatar.png',
  verified: true,
};

const sampleJob: GqlJobPosting = {
  jobId: 'job-001',
  client: 'crypto-client-1',
  title: 'Build a dashboard widget',
  description: 'Create a React component that displays analytics data.',
  category: 'development',
  skills: ['React', 'TypeScript', 'Tailwind'],
  budget: { amount: '500', asset: 'USDC' },
  status: 'OPEN',
  proposalCount: 3,
  contractEscrowId: 'escrow-001',
  dispute: undefined,
  onChain: undefined,
  proposalDeadline: '2026-07-01T00:00:00Z',
  createdAt: '2026-06-01T12:00:00Z',
  updatedAt: '2026-06-01T12:00:00Z',
  clientProfile: sampleClientProfile,
};

const sampleJobWithDispute: GqlJobPosting = {
  ...sampleJob,
  jobId: 'job-002',
  title: 'Disputed job posting',
  status: 'DISPUTED',
  dispute: {
    reason: 'Work not delivered on time',
    openedBy: 'crypto-client-1',
    openedAt: '2026-06-10T12:00:00Z',
    status: 'OPEN',
  },
};

const sampleJobWithOnChain: GqlJobPosting = {
  ...sampleJob,
  jobId: 'job-003',
  title: 'On-chain job posting',
  onChain: {
    vault: 'vault-address-1234',
    jobPdaCommit: 'pda-commit-5678',
    fundingTxSig: 'funding-tx-sig-9abc',
  },
};

const sampleProposal: Proposal = {
  proposalId: 'proposal-001',
  jobId: 'job-001',
  candidate: 'candidate-addr-999',
  coverLetter: 'I am very qualified for this job.',
  bidAmount: '450 USDC',
  status: 'PENDING',
  createdAt: '2026-06-02T10:00:00Z',
  updatedAt: '2026-06-02T10:00:00Z',
};

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [], count: 0 });
  vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as any);
});

// ── Jobs list ─────────────────────────────────────────────────────────────────

describe('Jobs list', () => {
  test('shows loading state before fetch resolves', () => {
    vi.mocked(apiClient.graphql.jobs).mockReturnValue(new Promise(() => {}));
    render(<JobsSection />);
    expect(screen.getByText(/loading jobs/i)).toBeInTheDocument();
  });

  test('shows empty state when jobs board has no postings', async () => {
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [], count: 0 });
    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText(/no jobs found/i)).toBeInTheDocument();
    });
  });

  test('renders job list with title, client name, budget, status, skills', async () => {
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });
    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });
    expect(screen.getByText('Client Alpha')).toBeInTheDocument();
    expect(screen.getByText('500 USDC')).toBeInTheDocument();
    expect(screen.getByText('OPEN')).toBeInTheDocument();
    expect(screen.getByText('React')).toBeInTheDocument();
    expect(screen.getByText('TypeScript')).toBeInTheDocument();
  });

  test('shows generic error on rejection', async () => {
    vi.mocked(apiClient.graphql.jobs).mockRejectedValue(new Error('network failure'));
    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText(/failed to load jobs/i)).toBeInTheDocument();
      expect(screen.getByText(/network failure/i)).toBeInTheDocument();
    });
  });

  test('tolerates response missing jobs field and shows empty', async () => {
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({} as any);
    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText(/no jobs found/i)).toBeInTheDocument();
    });
  });
});

// ── JobStatusBadge ────────────────────────────────────────────────────────────

describe('JobStatusBadge colors', () => {
  test('status badge renders correct color for OPEN', () => {
    render(<JobStatusBadge status="OPEN" />);
    const badge = screen.getByText('OPEN');
    expect(badge.className).toContain('green');
  });

  test('status badge renders correct color for DISPUTED', () => {
    render(<JobStatusBadge status="DISPUTED" />);
    const badge = screen.getByText('DISPUTED');
    expect(badge.className).toContain('red');
  });

  test('status badge renders correct color for IN_PROGRESS', () => {
    render(<JobStatusBadge status="IN_PROGRESS" />);
    const badge = screen.getByText('IN_PROGRESS');
    expect(badge.className).toContain('primary');
  });
});

// ── Inline expand ─────────────────────────────────────────────────────────────

describe('Inline expand', () => {
  test('click expands job to show full description', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });
    render(<JobsSection />);

    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });

    // Before expansion: full description not visible
    expect(
      screen.queryByText('Create a React component that displays analytics data.')
    ).not.toBeInTheDocument();

    // Click the row title to expand
    await user.click(screen.getByText('Build a dashboard widget'));

    await waitFor(() => {
      expect(
        screen.getByText('Create a React component that displays analytics data.')
      ).toBeInTheDocument();
    });
  });

  test('expanded job shows skills chips', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });
    render(<JobsSection />);

    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build a dashboard widget'));

    await waitFor(() => {
      // All three skills should be visible (not truncated in expanded view)
      const tailwindElements = screen.getAllByText('Tailwind');
      expect(tailwindElements.length).toBeGreaterThan(0);
    });
  });

  test('expanded job shows budget amount and asset', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });
    render(<JobsSection />);

    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build a dashboard widget'));

    await waitFor(() => {
      // Budget is shown in the summary row
      expect(screen.getByText('500 USDC')).toBeInTheDocument();
    });
  });

  test('expanded job shows dispute info when present', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJobWithDispute], count: 1 });
    render(<JobsSection />);

    await waitFor(() => {
      expect(screen.getByText('Disputed job posting')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Disputed job posting'));

    await waitFor(() => {
      expect(screen.getByText('Work not delivered on time')).toBeInTheDocument();
      expect(screen.getByText('Dispute')).toBeInTheDocument();
    });
  });

  test('expanded job shows on-chain info when present', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJobWithOnChain], count: 1 });
    render(<JobsSection />);

    await waitFor(() => {
      expect(screen.getByText('On-chain job posting')).toBeInTheDocument();
    });

    await user.click(screen.getByText('On-chain job posting'));

    await waitFor(() => {
      expect(screen.getByText('vault-address-1234')).toBeInTheDocument();
      expect(screen.getByText('On-chain')).toBeInTheDocument();
    });
  });

  test('expanded job hides dispute/on-chain when absent', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });
    render(<JobsSection />);

    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build a dashboard widget'));

    await waitFor(() => {
      // Description visible
      expect(
        screen.getByText('Create a React component that displays analytics data.')
      ).toBeInTheDocument();
    });

    // Dispute and on-chain sections should not be present
    expect(screen.queryByText('Dispute')).not.toBeInTheDocument();
    expect(screen.queryByText('On-chain')).not.toBeInTheDocument();
  });
});

// ── Client profile ────────────────────────────────────────────────────────────

describe('Client profile', () => {
  test('renders client profile avatar and display name', async () => {
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });
    render(<JobsSection />);

    await waitFor(() => {
      expect(screen.getByText('Client Alpha')).toBeInTheDocument();
    });

    // Avatar image should be present with alt text
    const avatar = screen.getByAltText('Client Alpha');
    expect(avatar).toBeInTheDocument();
  });
});

// ── Null skills handling ──────────────────────────────────────────────────────

describe('Null skills handling', () => {
  test('handles job with no skills (skills is null)', async () => {
    const jobNoSkills: GqlJobPosting = {
      ...sampleJob,
      jobId: 'job-no-skills',
      title: 'Job with no skills',
      skills: undefined,
    };
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [jobNoSkills], count: 1 });
    render(<JobsSection />);

    await waitFor(() => {
      expect(screen.getByText('Job with no skills')).toBeInTheDocument();
    });

    // Should not throw; no skill chips rendered for missing skills
    expect(screen.queryByText('+1 more')).not.toBeInTheDocument();
  });
});

// ── Jobs write actions ────────────────────────────────────────────────────────

describe('Jobs write actions', () => {
  test('Post Job button hidden when wallet locked', async () => {
    vi.mocked(fetchWalletStatus).mockRejectedValue(new Error('wallet locked'));
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });
    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });
    expect(screen.queryByRole('button', { name: /post a job/i })).not.toBeInTheDocument();
  });

  test('Post Job button visible when wallet unlocked', async () => {
    vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as any);
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });
    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: /post a job/i })).toBeInTheDocument();
    });
  });

  test('Post Job form submit calls create', async () => {
    const user = userEvent.setup();
    vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as any);
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });
    vi.mocked(apiClient.jobsWrite.create).mockResolvedValue(sampleJob as any);

    render(<JobsSection />);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /post a job/i })).toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: /post a job/i }));

    // Fill the form
    const titleInput = screen.getByPlaceholderText(/build a solana/i);
    await user.type(titleInput, 'My New Job');

    const budgetInput = screen.getByPlaceholderText('500');
    await user.type(budgetInput, '200');

    await user.click(screen.getByRole('button', { name: /post job/i }));

    await waitFor(() => {
      expect(vi.mocked(apiClient.jobsWrite.create)).toHaveBeenCalledWith(
        expect.objectContaining({ title: 'My New Job', budgetAmount: '200' })
      );
    });
  });

  test('Apply button visible for non-client on OPEN job', async () => {
    const user = userEvent.setup();
    // MY_AGENT_ID !== sampleJob.client ('crypto-client-1')
    vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as any);
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });

    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build a dashboard widget'));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^apply$/i })).toBeInTheDocument();
    });
  });

  test('Apply button hidden on own job (client === myAgentId)', async () => {
    const user = userEvent.setup();
    // wallet returns the same address as sampleJob.client
    vi.mocked(fetchWalletStatus).mockResolvedValue({
      accounts: [{ chain: 'solana', address: 'crypto-client-1' }],
    } as any);
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });

    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build a dashboard widget'));

    await waitFor(() => {
      // Description should be visible (expanded)
      expect(
        screen.getByText('Create a React component that displays analytics data.')
      ).toBeInTheDocument();
    });

    expect(screen.queryByRole('button', { name: /^apply$/i })).not.toBeInTheDocument();
  });

  test('Apply form calls jobsWrite.apply', async () => {
    const user = userEvent.setup();
    vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as any);
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });
    vi.mocked(apiClient.jobsWrite.apply).mockResolvedValue(sampleProposal as any);

    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build a dashboard widget'));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^apply$/i })).toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: /^apply$/i }));

    // Fill cover letter
    const coverLetterArea = screen.getByPlaceholderText(/describe your experience/i);
    await user.type(coverLetterArea, 'I am experienced in React');

    await user.click(screen.getByRole('button', { name: /submit application/i }));

    await waitFor(() => {
      expect(vi.mocked(apiClient.jobsWrite.apply)).toHaveBeenCalledWith(
        sampleJob.jobId,
        expect.objectContaining({ coverLetter: 'I am experienced in React' })
      );
    });
  });

  test('Apply success shows success state and triggers refetch', async () => {
    const user = userEvent.setup();
    vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as any);
    // First call: initial load; second call should happen after successful apply.
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });
    vi.mocked(apiClient.jobsWrite.apply).mockResolvedValue(sampleProposal as any);

    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build a dashboard widget'));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^apply$/i })).toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: /^apply$/i }));

    await user.click(screen.getByRole('button', { name: /submit application/i }));

    // After success, the jobs list should be refetched (called a second time).
    await waitFor(() => {
      expect(vi.mocked(apiClient.graphql.jobs)).toHaveBeenCalledTimes(2);
    });
  });

  test('Apply form shows error on failure', async () => {
    const user = userEvent.setup();
    vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as any);
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });
    vi.mocked(apiClient.jobsWrite.apply).mockRejectedValue(new Error('Network error'));

    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build a dashboard widget'));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^apply$/i })).toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: /^apply$/i }));

    await user.click(screen.getByRole('button', { name: /submit application/i }));

    await waitFor(() => {
      // The error message should appear in the modal.
      expect(screen.getByRole('alert')).toBeInTheDocument();
      expect(screen.getByRole('alert')).toHaveTextContent(/network error/i);
    });

    // Modal stays open so user can retry.
    expect(screen.getByRole('button', { name: /submit application/i })).toBeInTheDocument();
    // Refetch should NOT have been called on failure.
    expect(vi.mocked(apiClient.graphql.jobs)).toHaveBeenCalledTimes(1);
  });

  test('Apply button shows loading state while submitting', async () => {
    const user = userEvent.setup();
    vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as any);
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });

    let resolveApply: (v: unknown) => void;
    vi.mocked(apiClient.jobsWrite.apply).mockReturnValue(
      new Promise(resolve => {
        resolveApply = resolve;
      }) as any
    );

    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build a dashboard widget'));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^apply$/i })).toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: /^apply$/i }));

    await user.click(screen.getByRole('button', { name: /submit application/i }));

    // While the request is in-flight, the button shows "Applying..." and is disabled.
    await waitFor(() => {
      const btn = screen.getByRole('button', { name: /applying/i });
      expect(btn).toBeInTheDocument();
      expect(btn).toBeDisabled();
    });

    // Clean up the pending promise.
    resolveApply!(sampleProposal);
  });

  test('Own job shows Cancel Job and View Proposals buttons', async () => {
    const user = userEvent.setup();
    vi.mocked(fetchWalletStatus).mockResolvedValue({
      accounts: [{ chain: 'solana', address: 'crypto-client-1' }],
    } as any);
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });

    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build a dashboard widget'));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /cancel job/i })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: /view proposals/i })).toBeInTheDocument();
    });
  });

  test('Cancel Job calls jobsWrite.cancel and refetches', async () => {
    const user = userEvent.setup();
    vi.mocked(fetchWalletStatus).mockResolvedValue({
      accounts: [{ chain: 'solana', address: 'crypto-client-1' }],
    } as any);
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });
    vi.mocked(apiClient.jobsWrite.cancel).mockResolvedValue(sampleJob as any);

    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build a dashboard widget'));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /cancel job/i })).toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: /cancel job/i }));

    await waitFor(() => {
      expect(vi.mocked(apiClient.jobsWrite.cancel)).toHaveBeenCalledWith(sampleJob.jobId);
    });
    expect(vi.mocked(apiClient.graphql.jobs)).toHaveBeenCalledTimes(2);
  });

  test('Select shows confirm dialog and calls select when confirmed', async () => {
    const user = userEvent.setup();
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    vi.mocked(fetchWalletStatus).mockResolvedValue({
      accounts: [{ chain: 'solana', address: 'crypto-client-1' }],
    } as any);
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });
    vi.mocked(apiClient.jobsWrite.listProposals).mockResolvedValue({
      proposals: [sampleProposal],
    } as any);
    vi.mocked(apiClient.jobsWrite.select).mockResolvedValue({
      job: sampleJob,
      contractEscrowId: 'escrow-xyz',
    } as any);

    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build a dashboard widget'));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /view proposals/i })).toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: /view proposals/i }));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^select$/i })).toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: /^select$/i }));

    await waitFor(() => {
      expect(window.confirm).toHaveBeenCalled();
      expect(vi.mocked(apiClient.jobsWrite.select)).toHaveBeenCalledWith(
        sampleJob.jobId,
        sampleProposal.proposalId
      );
    });

    vi.restoreAllMocks();
  });

  test('Select aborted when confirm returns false', async () => {
    const user = userEvent.setup();
    vi.spyOn(window, 'confirm').mockReturnValue(false);
    vi.mocked(fetchWalletStatus).mockResolvedValue({
      accounts: [{ chain: 'solana', address: 'crypto-client-1' }],
    } as any);
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });
    vi.mocked(apiClient.jobsWrite.listProposals).mockResolvedValue({
      proposals: [sampleProposal],
    } as any);

    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build a dashboard widget'));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /view proposals/i })).toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: /view proposals/i }));

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /^select$/i })).toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: /^select$/i }));

    await waitFor(() => {
      expect(window.confirm).toHaveBeenCalled();
    });

    expect(vi.mocked(apiClient.jobsWrite.select)).not.toHaveBeenCalled();

    vi.restoreAllMocks();
  });

  test('Wallet-locked message shown when myAgentId is null', async () => {
    const user = userEvent.setup();
    vi.mocked(fetchWalletStatus).mockRejectedValue(new Error('wallet locked'));
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [sampleJob], count: 1 });

    render(<JobsSection />);
    await waitFor(() => {
      expect(screen.getByText('Build a dashboard widget')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build a dashboard widget'));

    await waitFor(() => {
      expect(screen.getByText(/unlock your wallet/i)).toBeInTheDocument();
    });
  });
});

// ── Proposal deadline normalization ───────────────────────────────────────────

describe('Proposal deadline normalization', () => {
  // Helper: open the Post Job modal and fill required fields.
  async function openPostJobModal(user: ReturnType<typeof userEvent.setup>) {
    vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as any);
    vi.mocked(apiClient.graphql.jobs).mockResolvedValue({ jobs: [], count: 0 });

    render(<JobsSection />);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /post a job/i })).toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: /post a job/i }));

    // Fill required fields: title and budget amount.
    await user.type(screen.getByPlaceholderText(/build a solana/i), 'Test Job Title');
    await user.type(screen.getByPlaceholderText('500'), '100');
  }

  test('date-only input is normalized to RFC3339 end-of-day UTC before submit', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.jobsWrite.create).mockResolvedValue(sampleJob as any);
    await openPostJobModal(user);

    // Use fireEvent to set the date input — userEvent doesn't drive date pickers.
    // The modal has exactly one input[type="date"] (Proposal Deadline).
    const { fireEvent } = await import('@testing-library/react');
    const dateInput = document.querySelector('input[type="date"]') as HTMLInputElement;
    fireEvent.change(dateInput, { target: { value: '2099-12-31' } });

    await user.click(screen.getByRole('button', { name: /post job/i }));

    await waitFor(() => {
      expect(vi.mocked(apiClient.jobsWrite.create)).toHaveBeenCalledWith(
        expect.objectContaining({ proposalDeadline: '2099-12-31T23:59:59Z' })
      );
    });
  });

  test('empty deadline is omitted (sent as undefined)', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.jobsWrite.create).mockResolvedValue(sampleJob as any);
    await openPostJobModal(user);

    // Leave the deadline blank and submit.
    await user.click(screen.getByRole('button', { name: /post job/i }));

    await waitFor(() => {
      expect(vi.mocked(apiClient.jobsWrite.create)).toHaveBeenCalledWith(
        expect.objectContaining({ proposalDeadline: undefined })
      );
    });
  });

  test('past deadline shows validation error and does not call create', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.jobsWrite.create).mockResolvedValue(sampleJob as any);
    await openPostJobModal(user);

    const { fireEvent } = await import('@testing-library/react');
    const dateInput = document.querySelector('input[type="date"]') as HTMLInputElement;
    fireEvent.change(dateInput, { target: { value: '2020-01-01' } });

    await user.click(screen.getByRole('button', { name: /post job/i }));

    await waitFor(() => {
      expect(screen.getByText(/proposal deadline must be in the future/i)).toBeInTheDocument();
    });
    expect(vi.mocked(apiClient.jobsWrite.create)).not.toHaveBeenCalled();
  });

  test('current-day deadline shows validation error and does not call create', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.jobsWrite.create).mockResolvedValue(sampleJob as any);
    await openPostJobModal(user);

    // Build today's date in YYYY-MM-DD using local calendar (same logic as
    // the component) to ensure the guard fires regardless of UTC offset.
    const now = new Date();
    const today = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;

    const { fireEvent } = await import('@testing-library/react');
    const dateInput = document.querySelector('input[type="date"]') as HTMLInputElement;
    fireEvent.change(dateInput, { target: { value: today } });

    await user.click(screen.getByRole('button', { name: /post job/i }));

    await waitFor(() => {
      expect(screen.getByText(/proposal deadline must be in the future/i)).toBeInTheDocument();
    });
    expect(vi.mocked(apiClient.jobsWrite.create)).not.toHaveBeenCalled();
  });
});
