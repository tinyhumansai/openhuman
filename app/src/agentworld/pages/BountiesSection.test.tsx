/**
 * Tests for BountiesSection — the Agent World Bounties section (Phase B).
 *
 * Covers loading / error / empty / populated states, BountyStatusBadge colors,
 * reward display formatting, accordion expand/collapse, wallet-gated actions,
 * form validation, and fund/submit/comment/cancel/council flows.
 *
 * apiClient is mocked at module level; no real RPC calls are made.
 * All sample data uses generic placeholder names/IDs per project rules.
 */
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { type Bounty } from '../../lib/agentworld/invokeApiClient';
import { fetchWalletStatus } from '../../services/walletApi';
import { apiClient } from '../AgentWorldShell';
import BountiesSection, { BountyStatusBadge } from './BountiesSection';

vi.mock('../AgentWorldShell', () => ({
  apiClient: {
    bounties: {
      list: vi.fn(),
      get: vi.fn(),
      create: vi.fn(),
      cancel: vi.fn(),
      submit: vi.fn(),
      listSubmissions: vi.fn(),
      comment: vi.fn(),
      listComments: vi.fn(),
      runCouncil: vi.fn(),
      approve: vi.fn(),
    },
  },
}));

vi.mock('../../services/walletApi', () => ({ fetchWalletStatus: vi.fn() }));

const MY_AGENT_ID = 'my-agent-solana-addr-1111';
const OTHER_AGENT_ID = 'other-agent-addr-2222';
const sampleWalletStatus = { accounts: [{ chain: 'solana', address: MY_AGENT_ID }] };

// ── Sample data (generic placeholders) ───────────────────────────────────────

const sampleBounty: Bounty = {
  bountyId: 'bounty-001',
  creator: OTHER_AGENT_ID,
  title: 'Build an integration plugin',
  description: 'Create a TypeScript plugin that connects to our API.',
  reward: { amount: '5000000', asset: 'USDC', network: 'solana-devnet' },
  status: 'open',
  submissionCount: 2,
  commentCount: 3,
  council: undefined,
  deadline: '2026-09-01T00:00:00Z',
  startAt: '2026-06-01T00:00:00Z',
  createdAt: '2026-06-01T12:00:00Z',
  updatedAt: '2026-06-01T12:00:00Z',
};

const sampleOwnBounty: Bounty = {
  ...sampleBounty,
  bountyId: 'bounty-002',
  creator: MY_AGENT_ID,
  title: 'My own draft bounty',
  status: 'draft',
  submissionCount: 0,
  commentCount: 0,
};

const sampleBountyWithCouncil: Bounty = {
  ...sampleBounty,
  bountyId: 'bounty-003',
  title: 'Bounty with council',
  status: 'judging',
  council: {
    status: 'complete',
    winnerSubmissionId: 'sub-winner-001',
    reasoning: 'Best submission by far.',
    votes: [
      {
        model: 'gpt-4o',
        winnerSubmissionId: 'sub-winner-001',
        reasoning: 'Excellent implementation',
      },
    ],
  },
};

const emptyListResponse = { bounties: [] };
const listWithBounties = { bounties: [sampleBounty] };
const listWithOwnBounty = { bounties: [sampleOwnBounty] };

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(apiClient.bounties.list).mockResolvedValue(emptyListResponse);
  vi.mocked(apiClient.bounties.listSubmissions).mockResolvedValue({ submissions: [] });
  vi.mocked(apiClient.bounties.listComments).mockResolvedValue({ comments: [] });
  vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as never);
});

// ── Loading state ─────────────────────────────────────────────────────────────

describe('Loading state', () => {
  test('shows loading spinner before fetch resolves', () => {
    vi.mocked(apiClient.bounties.list).mockReturnValue(new Promise(() => {}));
    render(<BountiesSection />);
    expect(screen.getByText(/loading bounties/i)).toBeInTheDocument();
  });
});

// ── Error state ───────────────────────────────────────────────────────────────

describe('Error state', () => {
  test('shows error message on API failure', async () => {
    vi.mocked(apiClient.bounties.list).mockRejectedValue(new Error('network error'));
    render(<BountiesSection />);
    await waitFor(() => {
      expect(screen.getByText(/failed to load bounties/i)).toBeInTheDocument();
      expect(screen.getByText(/network error/i)).toBeInTheDocument();
    });
  });
});

// ── Empty state ───────────────────────────────────────────────────────────────

describe('Empty state', () => {
  test('shows "No bounties found" when list is empty', async () => {
    vi.mocked(apiClient.bounties.list).mockResolvedValue(emptyListResponse);
    render(<BountiesSection />);
    await waitFor(() => {
      expect(screen.getByText(/no bounties found/i)).toBeInTheDocument();
    });
  });

  test('tolerates missing bounties field and shows empty', async () => {
    vi.mocked(apiClient.bounties.list).mockResolvedValue({} as never);
    render(<BountiesSection />);
    await waitFor(() => {
      expect(screen.getByText(/no bounties found/i)).toBeInTheDocument();
    });
  });
});

// ── Populated list ────────────────────────────────────────────────────────────

describe('Populated list', () => {
  test('renders bounty rows with title, reward, status badge, counts, deadline', async () => {
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithBounties);
    render(<BountiesSection />);
    await waitFor(() => {
      expect(screen.getByText('Build an integration plugin')).toBeInTheDocument();
    });
    // Reward: 5000000 base units USDC = 5 USDC displayed
    expect(screen.getByText(/5.*USDC/)).toBeInTheDocument();
    // Status badge
    expect(screen.getByText('open')).toBeInTheDocument();
    // Submission/comment counts
    expect(screen.getByText(/2 submission/)).toBeInTheDocument();
    expect(screen.getByText(/3 comment/)).toBeInTheDocument();
  });
});

// ── BountyStatusBadge ─────────────────────────────────────────────────────────

describe('BountyStatusBadge colors', () => {
  test('draft → gray (stone)', () => {
    render(<BountyStatusBadge status="draft" />);
    const badge = screen.getByText('draft');
    expect(badge.className).toContain('content-secondary');
  });

  test('open → green', () => {
    render(<BountyStatusBadge status="open" />);
    const badge = screen.getByText('open');
    expect(badge.className).toContain('green');
  });

  test('judging → amber', () => {
    render(<BountyStatusBadge status="judging" />);
    const badge = screen.getByText('judging');
    expect(badge.className).toContain('amber');
  });

  test('review → ocean (blue)', () => {
    render(<BountyStatusBadge status="review" />);
    const badge = screen.getByText('review');
    expect(badge.className).toContain('primary');
  });

  test('awarded → purple', () => {
    render(<BountyStatusBadge status="awarded" />);
    const badge = screen.getByText('awarded');
    expect(badge.className).toContain('purple');
  });

  test('cancelled → gray (stone)', () => {
    render(<BountyStatusBadge status="cancelled" />);
    const badge = screen.getByText('cancelled');
    expect(badge.className).toContain('content-secondary');
  });

  test('refunded → gray (stone)', () => {
    render(<BountyStatusBadge status="refunded" />);
    const badge = screen.getByText('refunded');
    expect(badge.className).toContain('content-secondary');
  });
});

// ── Accordion expand ──────────────────────────────────────────────────────────

describe('Accordion expand', () => {
  test('click expands row to show description and reward detail', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithBounties);
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByText('Build an integration plugin')).toBeInTheDocument();
    });

    // Description not visible before expand
    expect(
      screen.queryByText('Create a TypeScript plugin that connects to our API.')
    ).not.toBeInTheDocument();

    await user.click(screen.getByText('Build an integration plugin'));

    await waitFor(() => {
      expect(
        screen.getByText('Create a TypeScript plugin that connects to our API.')
      ).toBeInTheDocument();
    });
    // Deadline shown in detail
    expect(screen.getByText(/Deadline/)).toBeInTheDocument();
  });

  test('click again collapses row', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithBounties);
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByText('Build an integration plugin')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build an integration plugin'));
    await waitFor(() => {
      expect(
        screen.getByText('Create a TypeScript plugin that connects to our API.')
      ).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build an integration plugin'));
    await waitFor(() => {
      expect(
        screen.queryByText('Create a TypeScript plugin that connects to our API.')
      ).not.toBeInTheDocument();
    });
  });
});

// ── Wallet-gated actions ──────────────────────────────────────────────────────

describe('Wallet-gated Create Bounty button', () => {
  test('Create Bounty button is visible when wallet is unlocked', async () => {
    vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as never);
    render(<BountiesSection />);
    await waitFor(() => {
      expect(screen.getByText('Create Bounty')).toBeInTheDocument();
    });
  });

  test('Create Bounty button is hidden when wallet is locked', async () => {
    vi.mocked(fetchWalletStatus).mockResolvedValue({ accounts: [] } as never);
    render(<BountiesSection />);
    await waitFor(() => {
      expect(screen.queryByText('Create Bounty')).not.toBeInTheDocument();
    });
  });
});

// ── Create Bounty form ────────────────────────────────────────────────────────

describe('Create Bounty form', () => {
  test('opens modal on Create Bounty click and validates required fields', async () => {
    const user = userEvent.setup();
    vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as never);
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByRole('button', { name: /create bounty/i })).toBeInTheDocument();
    });

    await user.click(screen.getByRole('button', { name: /create bounty/i }));

    // Modal opens — find the form by waiting for the title input to appear
    let titleInput: HTMLElement | null = null;
    await waitFor(() => {
      titleInput = document.querySelector('input[placeholder="Bounty title"]') as HTMLElement;
      expect(titleInput).not.toBeNull();
    });

    // Submit with empty form — should show validation error (title field is empty)
    const submitBtn = document.querySelector('button[type="submit"]') as HTMLElement;
    expect(submitBtn).not.toBeNull();
    await user.click(submitBtn);
    await waitFor(() => {
      expect(screen.getByText(/title is required/i)).toBeInTheDocument();
    });
  });

  test('submits create form and calls bounties.create', async () => {
    const user = userEvent.setup();
    // create is now an x402 confirm-before-spend flow; the probe returns the
    // bounty directly when no payment is required (free path).
    vi.mocked(apiClient.bounties.create).mockResolvedValue({
      bounty: { ...sampleOwnBounty, status: 'open' },
    } as never);
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithOwnBounty);
    vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as never);
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByText('Create Bounty')).toBeInTheDocument();
    });

    // Click the outer "Create Bounty" button (type=button)
    const createBtn = screen
      .getAllByRole('button')
      .find(
        btn => btn.textContent?.includes('Create Bounty') && btn.getAttribute('type') === 'button'
      );
    expect(createBtn).toBeDefined();
    await user.click(createBtn!);

    // Fill in the form
    await user.type(screen.getByPlaceholderText(/bounty title/i), 'Test bounty title');
    await user.type(screen.getByPlaceholderText(/describe the bounty/i), 'Test description');
    await user.clear(screen.getByPlaceholderText('5'));
    await user.type(screen.getByPlaceholderText('5'), '10');

    const submitBtn2 = screen
      .getAllByRole('button')
      .find(btn => btn.getAttribute('type') === 'submit');
    expect(submitBtn2).toBeDefined();
    await user.click(submitBtn2!);

    await waitFor(() => {
      expect(apiClient.bounties.create).toHaveBeenCalledWith(
        expect.objectContaining({
          title: 'Test bounty title',
          description: 'Test description',
          amount: '10', // human-decimal amount (SDK BountyCreateRequest.amount)
          asset: 'USDC',
        }),
        { confirmed: false }
      );
    });
  });
});

// ── Submit Work flow ──────────────────────────────────────────────────────────

describe('Submit Work', () => {
  test('Submit Work button visible to non-creator on open bounty', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithBounties);
    // sampleBounty.creator is OTHER_AGENT_ID, not MY_AGENT_ID
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByText('Build an integration plugin')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build an integration plugin'));

    await waitFor(() => {
      expect(screen.getByText('Submit Work')).toBeInTheDocument();
    });
  });

  test('Submit Work modal validates URL required', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithBounties);
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByText('Build an integration plugin')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build an integration plugin'));

    await waitFor(() => {
      expect(screen.getByText('Submit Work')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Submit Work'));

    // Wait for the URL input to appear (modal opened)
    await waitFor(() => {
      expect(document.querySelector('input[placeholder="https://github.com/…"]')).not.toBeNull();
    });

    // Click the submit button (type=submit) without filling in URL
    const submitBtn = document.querySelector('button[type="submit"]') as HTMLElement;
    expect(submitBtn).not.toBeNull();
    await user.click(submitBtn);

    await waitFor(() => {
      expect(screen.getByText(/url is required/i)).toBeInTheDocument();
    });
  });

  test('Submit Work calls bounties.submit with URL', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithBounties);
    vi.mocked(apiClient.bounties.submit).mockResolvedValue({} as never);
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByText('Build an integration plugin')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build an integration plugin'));

    await waitFor(() => {
      expect(screen.getByText('Submit Work')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Submit Work'));

    await waitFor(() => {
      expect(screen.getByPlaceholderText(/https:\/\/github\.com/i)).toBeInTheDocument();
    });

    await user.type(
      screen.getByPlaceholderText(/https:\/\/github\.com/i),
      'https://github.com/example/submission'
    );
    await user.click(screen.getByText('Submit Work', { selector: '[type=submit]' }));

    await waitFor(() => {
      expect(apiClient.bounties.submit).toHaveBeenCalledWith(
        sampleBounty.bountyId,
        'https://github.com/example/submission',
        undefined,
        undefined
      );
    });
  });
});

// ── Comment flow ──────────────────────────────────────────────────────────────

describe('Comment flow', () => {
  test('Comment button calls bounties.comment', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithBounties);
    vi.mocked(apiClient.bounties.comment).mockResolvedValue({} as never);
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByText('Build an integration plugin')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build an integration plugin'));

    await waitFor(() => {
      expect(screen.getByText('Comment')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Comment'));

    await waitFor(() => {
      expect(screen.getByPlaceholderText(/your comment/i)).toBeInTheDocument();
    });

    await user.type(screen.getByPlaceholderText(/your comment/i), 'Great bounty!');
    await user.click(screen.getByText('Post Comment'));

    await waitFor(() => {
      expect(apiClient.bounties.comment).toHaveBeenCalledWith(
        sampleBounty.bountyId,
        'Great bounty!'
      );
    });
  });
});

// ── Cancel flow ───────────────────────────────────────────────────────────────

describe('Cancel flow', () => {
  test('Cancel button visible to creator on draft bounty', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithOwnBounty);
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByText('My own draft bounty')).toBeInTheDocument();
    });

    await user.click(screen.getByText('My own draft bounty'));

    await waitFor(() => {
      expect(screen.getByText('Cancel')).toBeInTheDocument();
    });
  });

  test('Cancel calls bounties.cancel and refetches', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithOwnBounty);
    vi.mocked(apiClient.bounties.cancel).mockResolvedValue({} as never);
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByText('My own draft bounty')).toBeInTheDocument();
    });

    await user.click(screen.getByText('My own draft bounty'));

    await waitFor(() => {
      expect(screen.getByText('Cancel')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Cancel'));

    await waitFor(() => {
      expect(apiClient.bounties.cancel).toHaveBeenCalledWith(sampleOwnBounty.bountyId);
    });
  });
});

// ── Run Council flow ──────────────────────────────────────────────────────────

describe('Run Council flow', () => {
  test('Run Council button visible to creator on open bounty', async () => {
    const user = userEvent.setup();
    const openOwnBounty = { ...sampleOwnBounty, status: 'open' };
    vi.mocked(apiClient.bounties.list).mockResolvedValue({ bounties: [openOwnBounty] });
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByText('My own draft bounty')).toBeInTheDocument();
    });

    await user.click(screen.getByText('My own draft bounty'));

    await waitFor(() => {
      expect(screen.getByText('Run Council')).toBeInTheDocument();
    });
  });

  test('Run Council calls bounties.runCouncil', async () => {
    const user = userEvent.setup();
    const openOwnBounty = { ...sampleOwnBounty, status: 'open' };
    vi.mocked(apiClient.bounties.list).mockResolvedValue({ bounties: [openOwnBounty] });
    vi.mocked(apiClient.bounties.runCouncil).mockResolvedValue({} as never);
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByText('My own draft bounty')).toBeInTheDocument();
    });

    await user.click(screen.getByText('My own draft bounty'));

    await waitFor(() => {
      expect(screen.getByText('Run Council')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Run Council'));

    await waitFor(() => {
      expect(apiClient.bounties.runCouncil).toHaveBeenCalledWith(sampleOwnBounty.bountyId);
    });
  });
});

// ── Wallet-locked state ───────────────────────────────────────────────────────

describe('Wallet-locked state', () => {
  test('shows "Unlock your wallet" when wallet is locked and user expands a row', async () => {
    const user = userEvent.setup();
    vi.mocked(fetchWalletStatus).mockResolvedValue({ accounts: [] } as never);
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithBounties);
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByText('Build an integration plugin')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build an integration plugin'));

    await waitFor(() => {
      expect(screen.getByText(/unlock your wallet/i)).toBeInTheDocument();
    });
  });
});

// ── Reward formatting ─────────────────────────────────────────────────────────

describe('Reward amount display', () => {
  test('formats base-unit USDC (5000000) as "5 USDC"', async () => {
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithBounties);
    render(<BountiesSection />);
    await waitFor(() => {
      expect(screen.getByText('Build an integration plugin')).toBeInTheDocument();
    });
    // Check the reward display (5000000 base units = 5 USDC)
    expect(screen.getByText(/5.*USDC/)).toBeInTheDocument();
  });
});

// ── Council section ───────────────────────────────────────────────────────────

describe('Council section', () => {
  test('renders council section when council is present', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.bounties.list).mockResolvedValue({ bounties: [sampleBountyWithCouncil] });
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByText('Bounty with council')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Bounty with council'));

    await waitFor(() => {
      expect(screen.getByText('Council')).toBeInTheDocument();
      expect(screen.getByText('complete')).toBeInTheDocument();
      expect(screen.getByText(/Best submission by far/)).toBeInTheDocument();
    });
  });

  test('renders council votes when present', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.bounties.list).mockResolvedValue({ bounties: [sampleBountyWithCouncil] });
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByText('Bounty with council')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Bounty with council'));

    await waitFor(() => {
      expect(screen.getByText('Votes')).toBeInTheDocument();
      expect(screen.getByText('gpt-4o')).toBeInTheDocument();
      expect(screen.getByText(/Excellent implementation/)).toBeInTheDocument();
    });
  });
});

// ── Create success confirmation ───────────────────────────────────────────────

/**
 * Helper: open Create Bounty modal, fill the minimum required fields, and
 * click the submit button. Returns after the submit click so callers can
 * assert on the result.
 */
async function openAndSubmitCreateForm(user: ReturnType<typeof userEvent.setup>) {
  await waitFor(() => {
    expect(screen.getByRole('button', { name: /create bounty/i })).toBeInTheDocument();
  });

  const createBtn = screen
    .getAllByRole('button')
    .find(
      btn => btn.textContent?.includes('Create Bounty') && btn.getAttribute('type') === 'button'
    );
  expect(createBtn).toBeDefined();
  await user.click(createBtn!);

  await waitFor(() => {
    expect(document.querySelector('input[placeholder="Bounty title"]')).not.toBeNull();
  });

  await user.type(screen.getByPlaceholderText(/bounty title/i), 'My new bounty');
  await user.type(screen.getByPlaceholderText(/describe the bounty/i), 'Some description');
  // Amount field is required and must be positive
  await user.clear(screen.getByPlaceholderText('5'));
  await user.type(screen.getByPlaceholderText('5'), '10');

  const submitBtn = screen
    .getAllByRole('button')
    .find(btn => btn.getAttribute('type') === 'submit');
  expect(submitBtn).toBeDefined();
  await user.click(submitBtn!);
}

describe('Create success confirmation', () => {
  test('shows success toast with title after bounty creation (free/open path)', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.bounties.create).mockResolvedValue({
      bounty: { ...sampleOwnBounty, status: 'open', title: 'My new bounty' },
    } as never);
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithOwnBounty);
    vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as never);

    render(<BountiesSection />);
    await openAndSubmitCreateForm(user);

    await waitFor(() => {
      expect(screen.getByText('Bounty created')).toBeInTheDocument();
      expect(screen.getByText('My new bounty')).toBeInTheDocument();
      expect(screen.getByText('View')).toBeInTheDocument();
    });
  });

  test('View action in toast expands the created bounty row', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.bounties.create).mockResolvedValue({
      bounty: { ...sampleOwnBounty, status: 'open', title: 'My new bounty' },
    } as never);
    // List returns the created bounty so the row exists to expand
    vi.mocked(apiClient.bounties.list).mockResolvedValue({
      bounties: [{ ...sampleOwnBounty, status: 'open', title: 'My new bounty' }],
    });
    vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as never);

    render(<BountiesSection />);
    await openAndSubmitCreateForm(user);

    // Wait for the toast to appear
    await waitFor(() => {
      expect(screen.getByText('View')).toBeInTheDocument();
    });

    // Click "View" — should expand the bounty row (description becomes visible)
    await user.click(screen.getByText('View'));

    await waitFor(() => {
      expect(
        screen.getByText('Create a TypeScript plugin that connects to our API.')
      ).toBeInTheDocument();
    });
  });

  test('list refetches after successful creation', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.bounties.create).mockResolvedValue({
      bounty: { ...sampleOwnBounty, status: 'open', title: 'My new bounty' },
    } as never);
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithOwnBounty);
    vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as never);

    render(<BountiesSection />);

    // list is called once on mount
    await waitFor(() => {
      expect(apiClient.bounties.list).toHaveBeenCalledTimes(1);
    });

    await openAndSubmitCreateForm(user);

    // list should be called a second time after create succeeds
    await waitFor(() => {
      expect(apiClient.bounties.list).toHaveBeenCalledTimes(2);
    });
  });

  test('toast can be dismissed', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.bounties.create).mockResolvedValue({
      bounty: { ...sampleOwnBounty, status: 'open', title: 'My new bounty' },
    } as never);
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithOwnBounty);
    vi.mocked(fetchWalletStatus).mockResolvedValue(sampleWalletStatus as never);

    render(<BountiesSection />);
    await openAndSubmitCreateForm(user);

    await waitFor(() => {
      expect(screen.getByText('Bounty created')).toBeInTheDocument();
    });

    // Find the close button inside the toast (svg close button, aria unlabelled)
    // The Toast component renders a close <button> after the action button.
    // We locate it by finding all buttons visible and clicking the one without
    // meaningful text content that follows the "View" action button.
    const closeBtn = screen.getAllByRole('button').find(btn => btn.textContent?.trim() === '');
    expect(closeBtn).toBeDefined();
    await user.click(closeBtn!);

    // After dismiss animation (200ms) the toast is removed; but in jsdom
    // setTimeout fires synchronously via fake timers — use waitFor instead.
    await waitFor(
      () => {
        expect(screen.queryByText('Bounty created')).not.toBeInTheDocument();
      },
      { timeout: 1000 }
    );
  });
});

// ── Submissions section ───────────────────────────────────────────────────────

describe('Submissions section', () => {
  test('renders submissions list when loaded on expand', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.bounties.list).mockResolvedValue(listWithBounties);
    vi.mocked(apiClient.bounties.listSubmissions).mockResolvedValue({
      submissions: [
        {
          submissionId: 'sub-001',
          bountyId: 'bounty-001',
          submitter: 'submitter-addr-aabb',
          url: 'https://github.com/example/work',
          title: 'My plugin implementation',
          status: 'submitted',
          createdAt: '2026-06-02T10:00:00Z',
          updatedAt: '2026-06-02T10:00:00Z',
        },
      ],
    });
    render(<BountiesSection />);

    await waitFor(() => {
      expect(screen.getByText('Build an integration plugin')).toBeInTheDocument();
    });

    await user.click(screen.getByText('Build an integration plugin'));

    await waitFor(() => {
      expect(screen.getByText('My plugin implementation')).toBeInTheDocument();
      expect(screen.getByText('https://github.com/example/work')).toBeInTheDocument();
    });
  });
});
