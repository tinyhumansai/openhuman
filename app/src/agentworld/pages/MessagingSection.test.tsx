/**
 * Tests for MessagingSection — gated DMs "coming soon" state + basic render,
 * and group membership-aware Join/Leave button behaviour.
 *
 * We mock the apiClient and fetchWalletStatus so no actual RPC calls are made.
 */
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { fetchWalletStatus } from '../../services/walletApi';
import { apiClient } from '../AgentWorldShell';
import MessagingSection, { TABS } from './MessagingSection';

// Typed helpers so vi.mocked() calls are terse below.
// These are resolved after the vi.mock factory runs.
const getKeyStatus = () => vi.mocked(apiClient.signal.keyStatus);
const getProvision = () => vi.mocked(apiClient.signal.provision);
const getRegisterEncryptionKey = () => vi.mocked(apiClient.signal.registerEncryptionKey);

// ── Mock apiClient ────────────────────────────────────────────────────────────
// The module exports apiClient as a named export; we replace its methods.

vi.mock('../AgentWorldShell', () => ({
  apiClient: {
    signal: {
      keyStatus: vi
        .fn()
        .mockResolvedValue({
          agentId: 'test-agent',
          localPreKeyCount: 0,
          hasActiveSignedPreKey: false,
          remote: null,
        }),
      provision: vi
        .fn()
        .mockResolvedValue({
          agentId: 'test-agent',
          oneTimePreKeyCount: 0,
          lowOneTimePreKeys: true,
          updatedAt: '2026-06-17T00:00:00Z',
        }),
      sendMessage: vi
        .fn()
        .mockResolvedValue({
          messageId: 'msg-123',
          timestamp: '2026-06-17T00:00:00Z',
          encrypted: true,
        }),
      decryptMessage: vi
        .fn()
        .mockResolvedValue({ plaintext: 'Hello!', from: 'peer-agent', messageId: 'msg-123' }),
      registerEncryptionKey: vi
        .fn()
        .mockResolvedValue({
          ok: true,
          encryptionKey: 'dGVzdA==',
          agentId: 'test-agent',
          updatedAt: '2026-06-17T00:00:00Z',
        }),
    },
    directory: {
      resolve: vi
        .fn()
        .mockResolvedValue({ identity: { cryptoId: 'resolved-crypto-id' }, agent: null }),
      getAgent: vi.fn().mockResolvedValue({ agentId: 'resolved-crypto-id' }),
      listAgents: vi.fn().mockResolvedValue({ agents: [] }),
      reverse: vi.fn().mockResolvedValue({ cryptoId: 'resolved-crypto-id', identities: [] }),
      listIdentities: vi.fn().mockResolvedValue({ identities: [] }),
      skills: vi.fn().mockResolvedValue({ agents: [] }),
      findByEncryptionKey: vi.fn().mockResolvedValue(null),
    },
    messages: {
      list: vi.fn().mockResolvedValue({ messages: [] }),
      acknowledge: vi.fn().mockResolvedValue(undefined),
    },
    channels: {
      list: vi.fn().mockResolvedValue({ channels: [] }),
      join: vi.fn().mockResolvedValue(undefined),
      leave: vi.fn().mockResolvedValue(undefined),
    },
    groups: {
      list: vi.fn().mockResolvedValue([]),
      join: vi.fn().mockResolvedValue(undefined),
      leave: vi.fn().mockResolvedValue(undefined),
      setMemberRole: vi.fn().mockResolvedValue(undefined),
      createInvite: vi
        .fn()
        .mockResolvedValue({
          token: 'tok-new',
          groupId: 'g-1',
          createdBy: 'me',
          createdAt: '',
          uses: 0,
        }),
      listInvites: vi.fn().mockResolvedValue([]),
      previewInvite: vi
        .fn()
        .mockResolvedValue({
          groupId: 'g-1',
          name: 'Preview Group',
          memberCount: 5,
          membershipPolicy: 'open',
          invitedBy: 'admin-1',
          valid: true,
        }),
      revokeInvite: vi.fn().mockResolvedValue(undefined),
      redeemInvite: vi
        .fn()
        .mockResolvedValue({
          groupId: 'g-1',
          agentId: 'me',
          role: 'member',
          status: 'active',
          joinedAt: '',
          updatedAt: '',
        }),
    },
    broadcasts: {
      list: vi.fn().mockResolvedValue([]),
      subscribe: vi.fn().mockResolvedValue(undefined),
      unsubscribe: vi.fn().mockResolvedValue(undefined),
    },
    inbox: {
      list: vi.fn().mockResolvedValue({ items: [], unreadCount: 0, totalCount: 0 }),
      counts: vi.fn().mockResolvedValue({ unread: 0, read: 0, archived: 0, byType: {}, urgent: 0 }),
      markRead: vi.fn().mockResolvedValue(undefined),
      markAllRead: vi.fn().mockResolvedValue(undefined),
      archive: vi.fn().mockResolvedValue(undefined),
      unarchive: vi.fn().mockResolvedValue(undefined),
      remove: vi.fn().mockResolvedValue(undefined),
    },
    streams: {
      start: vi.fn().mockResolvedValue({ streamId: 'inbox' }),
      stop: vi.fn().mockResolvedValue(undefined),
      list: vi.fn().mockResolvedValue({ streams: [] }),
    },
  },
}));

// ── Mock useTinyplaceStream hook ──────────────────────────────────────────────
// Use vi.hoisted so the mock variable is available inside the vi.mock factory,
// which is hoisted to the top of the file by Vitest's transform.

const { mockUseTinyplaceStream } = vi.hoisted(() => ({
  mockUseTinyplaceStream: vi.fn((_streamId?: string) => ({
    messages: [] as unknown[],
    status: 'idle' as string,
    clearMessages: vi.fn(),
  })),
}));

vi.mock('../hooks/useTinyplaceStream', () => ({
  useTinyplaceStream: (streamId?: string) => mockUseTinyplaceStream(streamId),
}));

// ── Mock fetchWalletStatus ────────────────────────────────────────────────────

vi.mock('../../services/walletApi', () => ({ fetchWalletStatus: vi.fn() }));

/** Helper: configure wallet mock to return the given agent ID (Solana address). */
function setWallet(agentId: string | null) {
  vi.mocked(fetchWalletStatus).mockResolvedValue(
    agentId
      ? ({ accounts: [{ chain: 'solana', address: agentId }] } as unknown as Awaited<
          ReturnType<typeof fetchWalletStatus>
        >)
      : ({ accounts: [] } as unknown as Awaited<ReturnType<typeof fetchWalletStatus>>)
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  // Default: wallet not connected — groups.list membership call returns [].
  setWallet(null);
});

// ── DMs panel (E2E enabled) ───────────────────────────────────────────────────

describe('DMs panel (E2E enabled)', () => {
  test('renders DM compose UI with peer input by default (DMs-only surface)', async () => {
    render(<MessagingSection />);

    // DMs is the default (and only) visible view now — no tab click needed.
    // Should see the peer input, not the "coming soon" placeholder
    expect(screen.getByPlaceholderText(/Recipient @handle/)).toBeInTheDocument();
    expect(screen.queryByTestId('dms-coming-soon')).not.toBeInTheDocument();
  });

  test('sending a message calls signal.sendMessage with encrypted params', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.messages.list).mockResolvedValue({ messages: [] });
    vi.mocked(apiClient.signal.sendMessage).mockResolvedValue({
      messageId: 'msg1',
      timestamp: '2026-06-17T00:00:00Z',
      encrypted: true,
    });

    // directory.resolve returns 'resolved-crypto-id' by default (from mock)
    vi.mocked(apiClient.directory.resolve).mockResolvedValue({
      identity: { cryptoId: 'resolved-crypto-id' },
    });

    render(<MessagingSection />);

    // Enter a handle (non-base58) — will be resolved to resolved-crypto-id
    const peerInput = screen.getByPlaceholderText(/Recipient @handle/);
    await user.type(peerInput, 'peer123');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    // Type and send a message
    const composeInput = await screen.findByPlaceholderText(/Type a message/);
    await user.type(composeInput, 'Hello encrypted world');
    await user.click(screen.getByRole('button', { name: 'Send' }));

    // sendMessage is called with the RESOLVED crypto_id, not the raw input
    expect(vi.mocked(apiClient.signal.sendMessage)).toHaveBeenCalledWith({
      recipient: 'resolved-crypto-id',
      plaintext: 'Hello encrypted world',
    });
  });

  test('our own sent message stays visible after refresh (relay only echoes incoming)', async () => {
    const user = userEvent.setup();
    // The relay never returns our outgoing message, so refresh sees nothing.
    vi.mocked(apiClient.messages.list).mockResolvedValue({ messages: [] });
    vi.mocked(apiClient.signal.sendMessage).mockResolvedValue({
      messageId: 'sent-1',
      timestamp: '2026-06-17T00:00:00Z',
      encrypted: true,
    });
    vi.mocked(apiClient.directory.resolve).mockResolvedValue({
      identity: { cryptoId: 'resolved-crypto-id' },
    });

    render(<MessagingSection />);
    await user.type(screen.getByPlaceholderText(/Recipient @handle/), 'peer123');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    const composeInput = await screen.findByPlaceholderText(/Type a message/);
    await user.type(composeInput, 'Persisted hello');
    await user.click(screen.getByRole('button', { name: 'Send' }));

    // The optimistic outgoing message must remain after send + the empty refresh.
    expect(await screen.findByText('Persisted hello')).toBeInTheDocument();
    // The input is cleared and the message is not wiped by the refresh.
    expect(screen.getByText('Persisted hello')).toBeInTheDocument();
  });

  test('shows an empty-state in an opened DM with no messages, alongside the compose box', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.messages.list).mockResolvedValue({ messages: [] });

    render(<MessagingSection />);
    const peerInput = screen.getByPlaceholderText(/Recipient @handle/);
    await user.type(peerInput, 'peerEmpty');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    expect(await screen.findByTestId('dm-empty-state')).toBeInTheDocument();
    // The compose box is present so the user can start the conversation.
    expect(screen.getByPlaceholderText(/Type a message/)).toBeInTheDocument();
  });

  test('sendMessage is called with plaintext param, never direct backend body', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.messages.list).mockResolvedValue({ messages: [] });
    vi.mocked(apiClient.signal.sendMessage).mockRejectedValueOnce(new Error('encryption failed'));

    render(<MessagingSection />);
    const peerInput = screen.getByPlaceholderText(/Recipient @handle/);
    await user.type(peerInput, 'peer456');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    const composeInput = await screen.findByPlaceholderText(/Type a message/);
    await user.type(composeInput, 'secret');
    await user.click(screen.getByRole('button', { name: 'Send' }));

    expect(vi.mocked(apiClient.signal.sendMessage)).toHaveBeenCalled();
    // The messages namespace has no 'send' method — only signal.sendMessage is callable
  });

  test('shows a friendly error when the recipient has no published key bundle', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.messages.list).mockResolvedValue({ messages: [] });
    vi.mocked(apiClient.signal.sendMessage).mockRejectedValueOnce(
      new Error(
        'CoreRpcError: HTTP 404: HTTP 404: /keys/61KcG5aGLqpnJz2fn4tujFKAdzqsdGR9XqiUeVoT3vPg/bundle: not found'
      )
    );

    render(<MessagingSection />);
    const peerInput = screen.getByPlaceholderText(/Recipient @handle/);
    await user.type(peerInput, 'peer456');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    const composeInput = await screen.findByPlaceholderText(/Type a message/);
    await user.type(composeInput, 'secret');
    await user.click(screen.getByRole('button', { name: 'Send' }));

    expect(
      await screen.findByText(/This user hasn't enabled encrypted messaging yet/)
    ).toBeInTheDocument();
    expect(screen.queryByText(/CoreRpcError/)).not.toBeInTheDocument();
    expect(screen.queryByText(/\/keys\//)).not.toBeInTheDocument();
  });

  test('normalizes the core mapped missing encrypted messaging setup error', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.messages.list).mockResolvedValue({ messages: [] });
    vi.mocked(apiClient.signal.sendMessage).mockRejectedValueOnce(
      new Error(
        'CoreRpcError: Recipient has not set up encrypted messaging yet. They need to provision Signal keys before receiving DMs. (resolved agent_id: resolved-crypto-id)'
      )
    );

    render(<MessagingSection />);
    const peerInput = screen.getByPlaceholderText(/Recipient @handle/);
    await user.type(peerInput, 'peer456');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    const composeInput = await screen.findByPlaceholderText(/Type a message/);
    await user.type(composeInput, 'secret');
    await user.click(screen.getByRole('button', { name: 'Send' }));

    expect(
      await screen.findByText(/This user hasn't enabled encrypted messaging yet/)
    ).toBeInTheDocument();
    expect(screen.queryByText(/CoreRpcError/)).not.toBeInTheDocument();
    expect(screen.queryByText(/resolved agent_id/)).not.toBeInTheDocument();
  });

  test('normalizes missing key bundle errors while refreshing direct messages', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.messages.list).mockRejectedValueOnce(
      new Error(
        'CoreRpcError: HTTP 404: HTTP 404: /keys/61KcG5aGLqpnJz2fn4tujFKAdzqsdGR9XqiUeVoT3vPg/bundle: not found'
      )
    );

    render(<MessagingSection />);
    const peerInput = screen.getByPlaceholderText(/Recipient @handle/);
    await user.type(peerInput, 'peer456');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    expect(
      await screen.findByText(/This user hasn't enabled encrypted messaging yet/)
    ).toBeInTheDocument();
    expect(screen.queryByText(/CoreRpcError/)).not.toBeInTheDocument();
    expect(screen.queryByText(/\/keys\//)).not.toBeInTheDocument();
  });

  test('received messages are decrypted before display', async () => {
    // directory.resolve returns 'resolved-crypto-id'; messages must use that id
    // in the 'from' field to survive the peerId filter in useDirectMessages.
    vi.mocked(apiClient.messages.list).mockResolvedValue({
      messages: [
        {
          id: 'msg1',
          from: 'resolved-crypto-id',
          to: 'a',
          timestamp: '2026-06-17T00:00:00Z',
          deviceId: 1,
          type: 'CIPHERTEXT',
          body: 'base64ciphertext',
          signal: { ratchetKey: 'abc', messageNumber: 0, previousChainLength: 0 },
        },
      ],
    });
    vi.mocked(apiClient.signal.decryptMessage).mockResolvedValue({
      plaintext: 'Decrypted secret',
      from: 'resolved-crypto-id',
      messageId: 'msg1',
    });

    const user = userEvent.setup();
    render(<MessagingSection />);
    const peerInput = screen.getByPlaceholderText(/Recipient @handle/);
    await user.type(peerInput, 'peer789');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    expect(await screen.findByText('Decrypted secret')).toBeInTheDocument();
    expect(vi.mocked(apiClient.signal.decryptMessage)).toHaveBeenCalledWith({
      envelope: expect.objectContaining({ id: 'msg1', body: 'base64ciphertext' }),
    });
  });

  test('encrypted indicator is shown in active DM view', async () => {
    vi.mocked(apiClient.messages.list).mockResolvedValue({ messages: [] });
    const user = userEvent.setup();

    render(<MessagingSection />);
    const peerInput = screen.getByPlaceholderText(/Recipient @handle/);
    await user.type(peerInput, 'peer999');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    expect(await screen.findByText('Encrypted')).toBeInTheDocument();
  });

  test('DmsPanel resolves @handle to cryptoId before opening DM', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.directory.resolve).mockResolvedValueOnce({
      identity: { cryptoId: 'alice-crypto-id' },
    });
    vi.mocked(apiClient.messages.list).mockResolvedValue({ messages: [] });

    render(<MessagingSection />);
    const peerInput = screen.getByPlaceholderText(/Recipient @handle/);
    await user.type(peerInput, '@alice');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    // directory.resolve should be called with 'alice' (stripped @)
    expect(vi.mocked(apiClient.directory.resolve)).toHaveBeenCalledWith('alice');
    // The DM view should open (compose box appears)
    expect(await screen.findByPlaceholderText(/Type a message/)).toBeInTheDocument();
  });

  test('DmsPanel shows error when directory.resolve returns no agent', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.directory.resolve).mockResolvedValueOnce({
      identity: undefined,
      agent: undefined,
    });

    render(<MessagingSection />);
    const peerInput = screen.getByPlaceholderText(/Recipient @handle/);
    await user.type(peerInput, '@unknown-user');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    const err = await screen.findByTestId('dm-resolve-error');
    expect(err).toHaveTextContent(/No agent found/);
    // DM view should NOT be open
    expect(screen.queryByPlaceholderText(/Type a message/)).not.toBeInTheDocument();
  });

  test('DmsPanel falls back to agent.cryptoId when identity is absent', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.directory.resolve).mockResolvedValueOnce({
      identity: undefined,
      agent: { agentId: 'agent-99', cryptoId: 'fallback-crypto-id', name: 'Bob' },
    });
    vi.mocked(apiClient.messages.list).mockResolvedValue({ messages: [] });

    render(<MessagingSection />);
    const peerInput = screen.getByPlaceholderText(/Recipient @handle/);
    await user.type(peerInput, 'bob.agent');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    // DM view should open using the agent's cryptoId
    expect(await screen.findByPlaceholderText(/Type a message/)).toBeInTheDocument();
  });

  test('DmsPanel passes raw base58 wallet address without calling resolve', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.messages.list).mockResolvedValue({ messages: [] });

    render(<MessagingSection />);
    const peerInput = screen.getByPlaceholderText(/Recipient @handle/);
    // A valid 44-char base58 string — should bypass resolution
    await user.type(peerInput, '61KcG5aGLqpnJz2fXyzABCDEFGHJKLMNPQRSTUVWXY');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    // directory.resolve must NOT be called for a raw base58 address
    expect(vi.mocked(apiClient.directory.resolve)).not.toHaveBeenCalled();
    // DM view should open directly
    expect(await screen.findByPlaceholderText(/Type a message/)).toBeInTheDocument();
  });

  test('DmsPanel shows error when directory.resolve throws', async () => {
    const user = userEvent.setup();
    vi.mocked(apiClient.directory.resolve).mockRejectedValueOnce(
      new Error('directory service unavailable')
    );

    render(<MessagingSection />);
    const peerInput = screen.getByPlaceholderText(/Recipient @handle/);
    await user.type(peerInput, '@broken-handle');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    const err = await screen.findByTestId('dm-resolve-error');
    expect(err).toHaveTextContent('directory service unavailable');
  });

  // ── Directory-card fallback tests ─────────────────────────────────────────
  // The Rust handler now synthesises a ResolveResponse from the directory
  // agent card when the registry 404s.  The frontend sees the same shape
  // (agent.cryptoId present, identity absent) regardless of which code path
  // resolved the name.

  test('DmsPanel opens DM via directory-card agent when registry misses (synthesised response)', async () => {
    // Simulate what the new Rust backend returns after directory-card fallback:
    // identity is null but agent has a cryptoId.
    const user = userEvent.setup();
    vi.mocked(apiClient.directory.resolve).mockResolvedValueOnce({
      identity: null,
      agent: {
        agentId: 'dir-agent-99',
        cryptoId: 'dir-crypto-id-99',
        name: 'stevejobs',
        username: 'stevejobs',
        createdAt: '2026-01-01T00:00:00Z',
        updatedAt: '2026-01-01T00:00:00Z',
      },
    });
    vi.mocked(apiClient.messages.list).mockResolvedValue({ messages: [] });

    render(<MessagingSection />);
    const peerInput = screen.getByPlaceholderText(/Recipient @handle/);
    await user.type(peerInput, 'stevejobs');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    // directory.resolve must have been called (not bypassed as base58)
    expect(vi.mocked(apiClient.directory.resolve)).toHaveBeenCalledWith('stevejobs');
    // DM view should open using the directory card crypto_id
    expect(await screen.findByPlaceholderText(/Type a message/)).toBeInTheDocument();
    expect(screen.queryByTestId('dm-resolve-error')).not.toBeInTheDocument();
  });

  test('DmsPanel shows clear human-readable error when handle is unresolvable', async () => {
    // Simulate the Rust backend returning a clear error string when neither
    // the registry nor the directory have the handle.  The frontend catches
    // the thrown error and shows it.
    const user = userEvent.setup();
    vi.mocked(apiClient.directory.resolve).mockRejectedValueOnce(
      new Error(
        'No agent found for "ghosthandle". Check the handle or paste their wallet address directly.'
      )
    );

    render(<MessagingSection />);
    const peerInput = screen.getByPlaceholderText(/Recipient @handle/);
    await user.type(peerInput, 'ghosthandle');
    await user.click(screen.getByRole('button', { name: 'Open DM' }));

    const err = await screen.findByTestId('dm-resolve-error');
    expect(err).toHaveTextContent('ghosthandle');
    expect(err).not.toHaveTextContent('404');
    // DM view must NOT open
    expect(screen.queryByPlaceholderText(/Type a message/)).not.toBeInTheDocument();
  });

  test('renders DM compose UI with updated placeholder text', async () => {
    render(<MessagingSection />);
    expect(screen.getByPlaceholderText('Recipient @handle or wallet address')).toBeInTheDocument();
  });
});

// ── Tab navigation ────────────────────────────────────────────────────────────

describe('tab navigation', () => {
  test('defaults to Channels tab', () => {
    render(<MessagingSection tabs={TABS} />);
    const channelsBtn = screen.getByRole('button', { name: 'Channels' });
    expect(channelsBtn).toHaveAttribute('data-active', 'true');
  });

  test('can switch to Groups tab', async () => {
    render(<MessagingSection tabs={TABS} />);
    const groupsBtn = screen.getByRole('button', { name: 'Groups' });
    await userEvent.click(groupsBtn);
    expect(groupsBtn).toHaveAttribute('data-active', 'true');
  });

  test('can switch to Inbox tab', async () => {
    render(<MessagingSection tabs={TABS} />);
    const inboxBtn = screen.getByRole('button', { name: 'Inbox' });
    await userEvent.click(inboxBtn);
    expect(inboxBtn).toHaveAttribute('data-active', 'true');
  });
});

// ── Empty states ──────────────────────────────────────────────────────────────

describe('empty states', () => {
  test('shows "No channels found" when channels list is empty', async () => {
    render(<MessagingSection tabs={TABS} />);
    // Wait for the async fetch to settle
    expect(await screen.findByText(/No channels found/i)).toBeInTheDocument();
  });

  test('shows "No groups found" when groups list is empty', async () => {
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    expect(await screen.findByText(/No groups found/i)).toBeInTheDocument();
  });

  test('shows "No broadcasts found" when broadcasts list is empty', async () => {
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Broadcasts' }));
    expect(await screen.findByText(/No broadcasts found/i)).toBeInTheDocument();
  });

  test('shows "Your inbox is empty" when inbox is empty', async () => {
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Inbox' }));
    expect(await screen.findByText(/Your inbox is empty/i)).toBeInTheDocument();
  });
});

// ── Inbox write actions ───────────────────────────────────────────────────────

describe('inbox actions', () => {
  const item = {
    itemId: 'item-1',
    type: 'message',
    status: 'unread',
    priority: 'normal',
    timestamp: new Date('2026-01-01T00:00:00Z').toISOString(),
    subject: 'Hello there',
  };

  beforeEach(() => {
    vi.mocked(apiClient.inbox.list).mockResolvedValue({
      items: [item],
      unreadCount: 1,
      totalCount: 1,
    });
    vi.mocked(apiClient.inbox.counts).mockResolvedValue({
      unread: 1,
      read: 0,
      archived: 0,
      byType: {},
      urgent: 0,
    });
  });

  async function openInbox() {
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Inbox' }));
    await screen.findByText('Hello there');
  }

  test('Mark read calls inbox.markRead with the item id', async () => {
    await openInbox();
    await userEvent.click(screen.getByRole('button', { name: 'Mark read' }));
    expect(apiClient.inbox.markRead).toHaveBeenCalledWith('item-1');
  });

  test('Archive calls inbox.archive with the item id', async () => {
    await openInbox();
    await userEvent.click(screen.getByRole('button', { name: 'Archive' }));
    expect(apiClient.inbox.archive).toHaveBeenCalledWith('item-1');
  });

  test('Remove calls inbox.remove with the item id', async () => {
    await openInbox();
    await userEvent.click(screen.getByRole('button', { name: 'Remove' }));
    expect(apiClient.inbox.remove).toHaveBeenCalledWith('item-1');
  });

  test('Mark all read calls inbox.markAllRead', async () => {
    await openInbox();
    await userEvent.click(screen.getByRole('button', { name: 'Mark all read' }));
    expect(apiClient.inbox.markAllRead).toHaveBeenCalled();
  });

  test('refetches the inbox after an action settles', async () => {
    await openInbox();
    const before = vi.mocked(apiClient.inbox.list).mock.calls.length;
    await userEvent.click(screen.getByRole('button', { name: 'Mark read' }));
    expect(vi.mocked(apiClient.inbox.list).mock.calls.length).toBeGreaterThan(before);
  });
});

// ── Channel / broadcast / group membership actions ────────────────────────────

describe('membership actions', () => {
  test('channel Join calls channels.join with the channel id', async () => {
    vi.mocked(apiClient.channels.list).mockResolvedValue({
      channels: [
        {
          channelId: 'ch-1',
          name: 'General',
          memberCount: 3,
          creator: 'someone',
          isPublic: true,
          createdAt: '2026-01-01T00:00:00Z',
          updatedAt: '2026-01-01T00:00:00Z',
        },
      ],
    });
    render(<MessagingSection tabs={TABS} />);
    await screen.findByText('General');
    await userEvent.click(screen.getByRole('button', { name: 'Join' }));
    expect(apiClient.channels.join).toHaveBeenCalledWith('ch-1');
  });

  test('broadcast Subscribe calls broadcasts.subscribe with the broadcast id', async () => {
    vi.mocked(apiClient.broadcasts.list).mockResolvedValue([
      {
        broadcastId: 'bc-1',
        name: 'Updates',
        subscriberCount: 9,
        owner: 'someone',
        visibility: 'public',
      },
    ]);
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Broadcasts' }));
    await screen.findByText('Updates');
    await userEvent.click(screen.getByRole('button', { name: 'Subscribe' }));
    expect(apiClient.broadcasts.subscribe).toHaveBeenCalledWith('bc-1');
  });

  test('group Leave calls groups.leave when user is a member', async () => {
    const group = {
      groupId: 'g-1',
      name: 'Builders',
      membershipPolicy: 'open',
      memberCount: 5,
      membershipEpoch: 1,
      createdBy: 'someone',
      createdAt: '2026-01-01T00:00:00Z',
    };
    // Wallet connected — user is a member of g-1.
    setWallet('agent-abc');
    // Both public and membership queries return the same group so button shows "Leave".
    vi.mocked(apiClient.groups.list).mockResolvedValue([group]);
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    await screen.findByText('Builders');
    await userEvent.click(screen.getByRole('button', { name: 'Leave' }));
    expect(apiClient.groups.leave).toHaveBeenCalledWith('g-1');
  });
});

// ── Group membership-aware Join / Leave ────────────────────────────────────────

describe('group membership-aware button rendering', () => {
  const groupA = {
    groupId: 'g-a',
    name: 'Alpha',
    membershipPolicy: 'open',
    memberCount: 2,
    membershipEpoch: 1,
    createdBy: 'owner',
    createdAt: '2026-01-01T00:00:00Z',
  };
  const groupB = {
    groupId: 'g-b',
    name: 'Beta',
    membershipPolicy: 'open',
    memberCount: 4,
    membershipEpoch: 1,
    createdBy: 'owner',
    createdAt: '2026-01-01T00:00:00Z',
  };

  async function openGroups() {
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    await screen.findByText('Alpha');
  }

  test('shows Join for non-member groups and Leave for member groups', async () => {
    setWallet('agent-xyz');
    // Public returns both; membership query returns only g-a (user is a member of Alpha only).
    vi.mocked(apiClient.groups.list)
      .mockResolvedValueOnce([groupA, groupB]) // public query
      .mockResolvedValueOnce([groupA]); // membership query (member=agent-xyz)

    await openGroups();
    await screen.findByText('Beta');

    // Alpha: user is a member → "Leave" button
    const alphaCard = screen.getByText('Alpha').closest('div')!;
    expect(alphaCard.querySelector('button[disabled]') ?? alphaCard).toBeTruthy();
    // Find buttons via accessible name
    const allLeave = screen.getAllByRole('button', { name: 'Leave' });
    const allJoin = screen.getAllByRole('button', { name: 'Join' });
    expect(allLeave).toHaveLength(1); // only Alpha
    expect(allJoin).toHaveLength(1); // only Beta
  });

  test('Leave button is absent for non-member groups (no wallet)', async () => {
    // No wallet → membership list is empty → all groups show Join.
    setWallet(null);
    vi.mocked(apiClient.groups.list).mockResolvedValue([groupA, groupB]);

    await openGroups();
    await screen.findByText('Beta');

    expect(screen.queryByRole('button', { name: 'Leave' })).not.toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: 'Join' })).toHaveLength(2);
  });

  test('Join calls groups.join and after refetch button flips to Leave', async () => {
    setWallet('agent-xyz');
    // First render: user is not a member of g-a.
    vi.mocked(apiClient.groups.list)
      .mockResolvedValueOnce([groupA]) // public
      .mockResolvedValueOnce([]) // membership — not a member yet
      // After join + refetch:
      .mockResolvedValueOnce([groupA]) // public
      .mockResolvedValueOnce([groupA]); // membership — now a member

    vi.mocked(apiClient.groups.join).mockResolvedValue(undefined);

    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    await screen.findByText('Alpha');

    // Button shows Join before joining.
    expect(screen.getByRole('button', { name: 'Join' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Leave' })).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: 'Join' }));
    expect(apiClient.groups.join).toHaveBeenCalledWith('g-a');

    // After re-fetch, button should flip to Leave.
    expect(await screen.findByRole('button', { name: 'Leave' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Join' })).not.toBeInTheDocument();
  });

  test('Leave calls groups.leave and after refetch button flips to Join', async () => {
    setWallet('agent-xyz');
    // First render: user IS a member of g-a.
    vi.mocked(apiClient.groups.list)
      .mockResolvedValueOnce([groupA]) // public
      .mockResolvedValueOnce([groupA]) // membership — is a member
      // After leave + refetch:
      .mockResolvedValueOnce([groupA]) // public
      .mockResolvedValueOnce([]); // membership — no longer a member

    vi.mocked(apiClient.groups.leave).mockResolvedValue(undefined);

    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    await screen.findByText('Alpha');

    // Button shows Leave (user is a member).
    expect(await screen.findByRole('button', { name: 'Leave' })).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: 'Leave' }));
    expect(apiClient.groups.leave).toHaveBeenCalledWith('g-a');

    // After re-fetch, button should flip to Join.
    expect(await screen.findByRole('button', { name: 'Join' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Leave' })).not.toBeInTheDocument();
  });

  test('membership dual-fetch passes member param to groups.list', async () => {
    setWallet('agent-solana-123');
    vi.mocked(apiClient.groups.list).mockResolvedValue([]);

    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    await screen.findByText(/No groups found/i);

    // The second groups.list call should carry member=<agentId>.
    const calls = vi.mocked(apiClient.groups.list).mock.calls;
    const memberCall = calls.find(
      ([p]) => p !== undefined && p !== null && typeof p === 'object' && 'member' in p
    );
    expect(memberCall).toBeDefined();
    expect((memberCall![0] as Record<string, unknown>).member).toBe('agent-solana-123');
  });
});

// ── Group invite management ─────────────────────────────────────────────────

describe('group invite management', () => {
  const group = {
    groupId: 'g-inv',
    name: 'Invite Test Group',
    membershipPolicy: 'invite-only',
    memberCount: 3,
    membershipEpoch: 1,
    createdBy: 'owner-1',
    createdAt: '2026-01-01T00:00:00Z',
  };

  beforeEach(() => {
    vi.mocked(apiClient.groups.list).mockResolvedValue([group]);
  });

  test('renders "Invites" button on group cards', async () => {
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    await screen.findByText('Invite Test Group');
    expect(screen.getByRole('button', { name: 'Invites' })).toBeInTheDocument();
  });

  test('clicking "Invites" opens GroupInvitesPanel and calls listInvites', async () => {
    vi.mocked(apiClient.groups.listInvites).mockResolvedValue([]);
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    await screen.findByText('Invite Test Group');
    await userEvent.click(screen.getByRole('button', { name: 'Invites' }));
    expect(apiClient.groups.listInvites).toHaveBeenCalledWith('g-inv');
    expect(await screen.findByText(/Invites for Invite Test Group/)).toBeInTheDocument();
  });

  test('GroupInvitesPanel shows existing invites with token and usage', async () => {
    vi.mocked(apiClient.groups.listInvites).mockResolvedValue([
      {
        groupId: 'g-inv',
        token: 'tok-abc',
        createdBy: 'owner-1',
        createdAt: '2026-01-01T00:00:00Z',
        uses: 2,
        maxUses: 10,
      },
    ]);
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    await screen.findByText('Invite Test Group');
    await userEvent.click(screen.getByRole('button', { name: 'Invites' }));
    expect(await screen.findByText('tok-abc')).toBeInTheDocument();
    expect(screen.getByText(/2 uses/)).toBeInTheDocument();
    expect(screen.getByText(/\/ 10 max/)).toBeInTheDocument();
  });

  test('Create Invite button calls groups.createInvite', async () => {
    vi.mocked(apiClient.groups.listInvites).mockResolvedValue([]);
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    await screen.findByText('Invite Test Group');
    await userEvent.click(screen.getByRole('button', { name: 'Invites' }));
    await screen.findByText(/No active invites/);
    await userEvent.click(screen.getByRole('button', { name: 'Create Invite' }));
    expect(apiClient.groups.createInvite).toHaveBeenCalledWith('g-inv');
  });

  test('Revoke button calls groups.revokeInvite with token', async () => {
    vi.mocked(apiClient.groups.listInvites).mockResolvedValue([
      {
        groupId: 'g-inv',
        token: 'tok-revoke',
        createdBy: 'owner-1',
        createdAt: '2026-01-01T00:00:00Z',
        uses: 0,
      },
    ]);
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    await screen.findByText('Invite Test Group');
    await userEvent.click(screen.getByRole('button', { name: 'Invites' }));
    await screen.findByText('tok-revoke');
    await userEvent.click(screen.getByRole('button', { name: 'Revoke' }));
    expect(apiClient.groups.revokeInvite).toHaveBeenCalledWith('g-inv', 'tok-revoke');
  });

  test('Close button returns to the group list', async () => {
    vi.mocked(apiClient.groups.listInvites).mockResolvedValue([]);
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    await screen.findByText('Invite Test Group');
    await userEvent.click(screen.getByRole('button', { name: 'Invites' }));
    await screen.findByText(/Invites for Invite Test Group/);
    await userEvent.click(screen.getByRole('button', { name: 'Close' }));
    // Should be back to the group list.
    expect(await screen.findByText('Invite Test Group')).toBeInTheDocument();
  });
});

describe('redeem invite', () => {
  test('renders "Redeem Invite" button in the groups tab', async () => {
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    expect(await screen.findByRole('button', { name: 'Redeem Invite' })).toBeInTheDocument();
  });

  test('clicking "Redeem Invite" opens the redeem panel with inputs', async () => {
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    await screen.findByRole('button', { name: 'Redeem Invite' });
    await userEvent.click(screen.getByRole('button', { name: 'Redeem Invite' }));
    expect(screen.getByPlaceholderText('Group ID')).toBeInTheDocument();
    expect(screen.getByPlaceholderText('Invite token')).toBeInTheDocument();
  });

  test('Preview button calls groups.previewInvite and shows result', async () => {
    vi.mocked(apiClient.groups.previewInvite).mockResolvedValue({
      groupId: 'g-prev',
      name: 'Previewed Group',
      memberCount: 7,
      membershipPolicy: 'invite-only',
      invitedBy: 'admin-1',
      valid: true,
    });
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    await userEvent.click(screen.getByRole('button', { name: 'Redeem Invite' }));
    await userEvent.type(screen.getByPlaceholderText('Group ID'), 'g-prev');
    await userEvent.type(screen.getByPlaceholderText('Invite token'), 'tok-preview');
    await userEvent.click(screen.getByRole('button', { name: 'Preview' }));
    expect(apiClient.groups.previewInvite).toHaveBeenCalledWith('g-prev', 'tok-preview');
    expect(await screen.findByText('Previewed Group')).toBeInTheDocument();
  });

  test('Redeem button calls groups.redeemInvite and shows success', async () => {
    vi.mocked(apiClient.groups.redeemInvite).mockResolvedValue({
      groupId: 'g-redeem',
      agentId: 'MyAddr123',
      role: 'member',
      status: 'active',
      joinedAt: '2026-01-01T00:00:00Z',
      updatedAt: '2026-01-01T00:00:00Z',
    });
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Groups' }));
    await userEvent.click(screen.getByRole('button', { name: 'Redeem Invite' }));
    await userEvent.type(screen.getByPlaceholderText('Group ID'), 'g-redeem');
    await userEvent.type(screen.getByPlaceholderText('Invite token'), 'tok-redeem');
    await userEvent.click(screen.getByRole('button', { name: 'Redeem' }));
    expect(apiClient.groups.redeemInvite).toHaveBeenCalledWith('g-redeem', 'tok-redeem');
    expect(await screen.findByText(/Joined as member in group g-redeem/)).toBeInTheDocument();
  });
});

// ── Inbox stream integration ───────────────────────────────────────────────────

describe('inbox stream lifecycle', () => {
  beforeEach(() => {
    // Restore inbox list + counts mocks so the Inbox panel can settle.
    vi.mocked(apiClient.inbox.list).mockResolvedValue({ items: [], unreadCount: 0, totalCount: 0 });
    vi.mocked(apiClient.inbox.counts).mockResolvedValue({
      unread: 0,
      read: 0,
      archived: 0,
      byType: {},
      urgent: 0,
    });
    // Restore streams.start mock.
    vi.mocked(apiClient.streams.start).mockResolvedValue({ streamId: 'inbox' });
    // Default hook mock: idle.
    mockUseTinyplaceStream.mockReturnValue({
      messages: [],
      status: 'idle',
      clearMessages: vi.fn(),
    });
  });

  test('calls streams.start with "inbox" when Inbox tab is opened', async () => {
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Inbox' }));
    // Wait for async effects to settle.
    await screen.findByText(/Your inbox is empty/i);
    expect(apiClient.streams.start).toHaveBeenCalledWith('inbox');
  });

  test('renders the Live indicator when streamStatus is connected', async () => {
    mockUseTinyplaceStream.mockImplementation(() => ({
      messages: [],
      status: 'connected',
      clearMessages: vi.fn(),
    }));
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Inbox' }));
    // Wait for the async inbox fetch to settle and the live indicator to appear.
    await screen.findByTestId('inbox-live-indicator');
    expect(screen.getByTestId('inbox-live-indicator')).toBeInTheDocument();
  });

  test('does NOT render the Live indicator when streamStatus is idle', async () => {
    render(<MessagingSection tabs={TABS} />);
    await userEvent.click(screen.getByRole('button', { name: 'Inbox' }));
    await screen.findByText(/Your inbox is empty/i);
    expect(screen.queryByTestId('inbox-live-indicator')).not.toBeInTheDocument();
  });
});

// ── SignalKeyStatusCard ───────────────────────────────────────────────────────

describe('SignalKeyStatusCard', () => {
  test('renders "Set up keys" button when no keys are provisioned', async () => {
    getKeyStatus().mockResolvedValueOnce({
      agentId: 'test-agent',
      localPreKeyCount: 0,
      hasActiveSignedPreKey: false,
      remote: null,
    });
    render(<MessagingSection />);
    expect(await screen.findByText('Set up keys')).toBeInTheDocument();
    expect(screen.getByText(/Set up encryption keys/)).toBeInTheDocument();
  });

  test('renders "Keys ready" when keys are provisioned but not yet discoverable', async () => {
    getKeyStatus().mockResolvedValueOnce({
      agentId: 'test-agent',
      localPreKeyCount: 100,
      hasActiveSignedPreKey: true,
      remote: null,
      encryptionKeyPublished: false,
    });
    render(<MessagingSection />);
    expect(await screen.findByText(/Keys ready/)).toBeInTheDocument();
    expect(screen.queryByText('Set up keys')).not.toBeInTheDocument();
  });

  test('renders "Make discoverable" when keys ready but encryption key not published', async () => {
    getKeyStatus().mockResolvedValueOnce({
      agentId: 'test-agent',
      localPreKeyCount: 100,
      hasActiveSignedPreKey: true,
      remote: null,
      encryptionKeyPublished: false,
    });
    render(<MessagingSection />);
    expect(await screen.findByText('Make discoverable')).toBeInTheDocument();
    expect(screen.getByText(/not yet discoverable/)).toBeInTheDocument();
  });

  test('renders "Discoverable" when encryption key is published', async () => {
    getKeyStatus().mockResolvedValueOnce({
      agentId: 'test-agent',
      localPreKeyCount: 100,
      hasActiveSignedPreKey: true,
      remote: null,
      encryptionKeyPublished: true,
    });
    render(<MessagingSection />);
    expect(await screen.findByText(/Discoverable/)).toBeInTheDocument();
    expect(screen.queryByText('Make discoverable')).not.toBeInTheDocument();
    expect(screen.queryByText('Set up keys')).not.toBeInTheDocument();
  });

  test('clicking "Make discoverable" calls registerEncryptionKey and refreshes', async () => {
    const user = userEvent.setup();
    getKeyStatus()
      .mockResolvedValueOnce({
        agentId: 'test-agent',
        localPreKeyCount: 100,
        hasActiveSignedPreKey: true,
        remote: null,
        encryptionKeyPublished: false,
      })
      .mockResolvedValueOnce({
        agentId: 'test-agent',
        localPreKeyCount: 100,
        hasActiveSignedPreKey: true,
        remote: null,
        encryptionKeyPublished: true,
      });
    getRegisterEncryptionKey().mockResolvedValueOnce({
      ok: true,
      encryptionKey: 'dGVzdA==',
      agentId: 'test-agent',
      updatedAt: '2026-06-17T00:00:00Z',
    });
    render(<MessagingSection />);
    const btn = await screen.findByText('Make discoverable');
    await user.click(btn);
    expect(getRegisterEncryptionKey()).toHaveBeenCalled();
    expect(await screen.findByText(/Discoverable/)).toBeInTheDocument();
  });

  test('surfaces an error when "Make discoverable" fails instead of silently doing nothing', async () => {
    const user = userEvent.setup();
    getKeyStatus().mockResolvedValue({
      agentId: 'test-agent',
      localPreKeyCount: 100,
      hasActiveSignedPreKey: true,
      remote: null,
      encryptionKeyPublished: false,
    });
    getRegisterEncryptionKey().mockRejectedValueOnce(new Error('HTTP 404: /directory/agents/x'));
    render(<MessagingSection />);
    const btn = await screen.findByText('Make discoverable');
    await user.click(btn);
    expect(getRegisterEncryptionKey()).toHaveBeenCalled();
    const err = await screen.findByTestId('signal-action-error');
    expect(err).toHaveTextContent('404');
  });

  test('clicking "Set up keys" calls signal.provision and refreshes', async () => {
    const user = userEvent.setup();
    getKeyStatus()
      .mockResolvedValueOnce({
        agentId: 'test-agent',
        localPreKeyCount: 0,
        hasActiveSignedPreKey: false,
        remote: null,
      })
      .mockResolvedValueOnce({
        agentId: 'test-agent',
        localPreKeyCount: 100,
        hasActiveSignedPreKey: true,
        remote: null,
      });
    getProvision().mockResolvedValueOnce({
      agentId: 'test-agent',
      oneTimePreKeyCount: 100,
      lowOneTimePreKeys: false,
      updatedAt: '2026-06-17T00:00:00Z',
    });
    render(<MessagingSection />);
    const btn = await screen.findByText('Set up keys');
    await user.click(btn);
    expect(getProvision()).toHaveBeenCalled();
    expect(await screen.findByText(/Keys ready/)).toBeInTheDocument();
  });
});
