/**
 * MessagingSection — Agent World Messages tab.
 *
 * Renders public metadata for Channels, Groups, Broadcasts, and Inbox.
 * Encrypted DM compose/read is gated behind E2E_MESSAGING_ENABLED (currently
 * true) using the Signal protocol for end-to-end encryption.
 *
 * Signal key provisioning is always accessible when a wallet is connected —
 * the `SignalKeyStatusCard` appears above the tab content regardless of the
 * E2E_MESSAGING_ENABLED gate so users can set up keys before 0C ships.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import ChipTabs from '../../components/layout/ChipTabs';
import Button from '../../components/ui/Button';
import {
  type BroadcastChannel,
  type BroadcastQueryParams,
  type Channel,
  type ChannelQueryParams,
  type GroupInvite,
  type GroupInvitePreview,
  type GroupMember,
  type GroupMetadata,
  type GroupQueryParams,
  type InboxItem,
  type InboxQueryParams,
  PaymentRequiredError,
  type SignalKeyStatus,
} from '../../lib/agentworld/invokeApiClient';
import { useT } from '../../lib/i18n/I18nContext';
import { fetchWalletStatus } from '../../services/walletApi';
import { apiClient } from '../AgentWorldShell';
import { useTinyplaceStream } from '../hooks/useTinyplaceStream';

const log = debug('openhuman:messaging');

// ── Feature gate ──────────────────────────────────────────────────────────────

/**
 * Signal-protocol encrypted DMs are deferred. When this flag is true the DMs
 * tab will render the real compose UI; until then it renders the "coming soon"
 * placeholder. Do NOT wire this to Rust Config — it's a UI-only fence.
 */
const E2E_MESSAGING_ENABLED = true;

// ── Tab definition ────────────────────────────────────────────────────────────

export const TABS = ['channels', 'groups', 'broadcasts', 'inbox', 'dms'] as const;
type Tab = (typeof TABS)[number];

const TAB_LABELS: Record<Tab, string> = {
  channels: 'Channels',
  groups: 'Groups',
  broadcasts: 'Broadcasts',
  inbox: 'Inbox',
  dms: 'DMs',
};

// ── Generic async-state shape ─────────────────────────────────────────────────

type AsyncState<T> =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'payment_required'; challenge: unknown }
  | { status: 'error'; message: string }
  | { status: 'ok'; data: T };

function useAsyncCall<T>(fetcher: () => Promise<T>, deps: unknown[]): AsyncState<T> {
  const [state, setState] = useState<AsyncState<T>>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;
    setState({ status: 'loading' });

    void fetcher()
      .then(data => {
        if (!cancelled) setState({ status: 'ok', data });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        if (err instanceof PaymentRequiredError) {
          setState({ status: 'payment_required', challenge: err.challenge });
        } else {
          setState({ status: 'error', message: String(err) });
        }
      });

    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  return state;
}

// ── My agent ID (wallet address) ─────────────────────────────────────────────

/**
 * Returns the Solana agent ID of the connected wallet, or null when the wallet
 * is not connected / still loading.  Mirrors the pattern in DirectorySection.
 */
function useMyAgentId(): string | null {
  const [agentId, setAgentId] = useState<string | null>(null);
  useEffect(() => {
    void fetchWalletStatus()
      .then(status => {
        const solana = (status.accounts ?? []).find(
          (a: { chain: string; address?: string }) => a.chain === 'solana'
        );
        if (solana?.address) setAgentId(solana.address);
      })
      .catch(() => {});
  }, []);
  return agentId;
}

// ── Sub-panels ────────────────────────────────────────────────────────────────

function LoadingPane() {
  return (
    <div className="flex items-center justify-center py-12 text-content-muted">
      <span className="animate-pulse text-sm">Loading…</span>
    </div>
  );
}

function ErrorPane({ message }: { message: string }) {
  const isWalletLocked =
    message.includes('wallet is not configured') ||
    message.includes('wallet secret material is missing');

  if (isWalletLocked) {
    return (
      <div className="flex flex-col items-center justify-center py-12 gap-2 text-content-muted">
        <p className="font-medium">Unlock your wallet to use Agent World</p>
        <p className="text-sm">Import your recovery phrase in Settings to continue.</p>
      </div>
    );
  }

  return (
    <div className="flex flex-col items-center justify-center py-12 gap-2 text-red-400">
      <p className="font-medium text-sm">Failed to load</p>
      <p className="text-xs text-content-faint">{message}</p>
    </div>
  );
}

function PaymentRequiredPane() {
  return (
    <div className="flex flex-col items-center justify-center py-12 gap-2 text-amber-400">
      <p className="font-medium">Access requires payment</p>
      <p className="text-sm text-content-muted">
        Your wallet will be used to fulfill the x402 payment challenge.
      </p>
    </div>
  );
}

// ── Signal key status ─────────────────────────────────────────────────────────

function useSignalKeyStatus() {
  const [status, setStatus] = useState<SignalKeyStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await apiClient.signal.keyStatus();
      setStatus(result);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { status, loading, error, refresh };
}

function SignalKeyStatusCard() {
  const { status, loading, error, refresh } = useSignalKeyStatus();
  const [provisioning, setProvisioning] = useState(false);
  const [publishing, setPublishing] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const handleProvision = useCallback(async () => {
    setProvisioning(true);
    setActionError(null);
    try {
      await apiClient.signal.provision();
      await refresh();
    } catch (err) {
      log('provision error: %s', String(err));
      setActionError(String(err));
    } finally {
      setProvisioning(false);
    }
  }, [refresh]);

  const handlePublish = useCallback(async () => {
    setPublishing(true);
    setActionError(null);
    try {
      await apiClient.signal.registerEncryptionKey();
      await refresh();
    } catch (err) {
      log('register encryption key error: %s', String(err));
      setActionError(String(err));
    } finally {
      setPublishing(false);
    }
  }, [refresh]);

  if (loading && !status) return null; // silent initial load
  if (error && !status) return null; // degrade: hide if status unavailable

  const keysReady = status?.hasActiveSignedPreKey && (status?.localPreKeyCount ?? 0) > 0;
  const discoverable = status?.encryptionKeyPublished === true;

  return (
    <div className="mx-4 mb-3 rounded-lg border border-line bg-surface-muted p-3">
      <div className="flex items-center justify-between">
        <div>
          <p className="text-sm font-medium text-content">Encrypted messaging</p>
          <p className="mt-0.5 text-xs text-content-muted">
            {!keysReady
              ? 'Set up encryption keys to enable direct messages'
              : discoverable
                ? `Discoverable (${status!.localPreKeyCount} pre-keys)`
                : `Keys ready (${status!.localPreKeyCount} pre-keys) -- not yet discoverable`}
          </p>
        </div>
        {!keysReady && (
          <Button
            variant="primary"
            size="sm"
            className="ml-3 flex-shrink-0"
            disabled={provisioning}
            onClick={() => void handleProvision()}>
            {provisioning ? 'Setting up...' : 'Set up keys'}
          </Button>
        )}
        {keysReady && !discoverable && (
          <Button
            variant="primary"
            size="sm"
            className="ml-3 flex-shrink-0"
            disabled={publishing}
            onClick={() => void handlePublish()}>
            {publishing ? 'Publishing...' : 'Make discoverable'}
          </Button>
        )}
      </div>
      {actionError && (
        <p className="mt-2 text-xs text-coral-500" data-testid="signal-action-error">
          {actionError}
        </p>
      )}
    </div>
  );
}

// ── Channels panel ────────────────────────────────────────────────────────────

function ChannelsPanel() {
  const params: ChannelQueryParams = { limit: 20 };
  const { version, busyKey, error: actionError, run } = useRowActions();
  const state = useAsyncCall(() => apiClient.channels.list(params), [version]);

  if (state.status === 'loading') return <LoadingPane />;
  if (state.status === 'payment_required') return <PaymentRequiredPane />;
  if (state.status === 'error') return <ErrorPane message={state.message} />;

  const channels: Channel[] = state.status === 'ok' ? state.data.channels : [];

  if (channels.length === 0) {
    return (
      <div className="flex items-center justify-center py-12 text-content-faint text-sm">
        No channels found
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {actionError ? <ActionErrorBanner message={actionError} /> : null}
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        {channels.map(ch => {
          const busy = busyKey === ch.channelId;
          return (
            <div
              key={ch.channelId}
              className="rounded-lg border border-line bg-surface-muted p-3 text-sm">
              <div className="flex items-center justify-between gap-2">
                <span className="font-medium text-content truncate">{ch.name}</span>
                <span className="shrink-0 text-xs text-content-faint">
                  {ch.memberCount} members
                </span>
              </div>
              {ch.description ? (
                <p className="mt-1 text-xs text-content-muted truncate">{ch.description}</p>
              ) : null}
              {ch.tags && ch.tags.length > 0 ? (
                <div className="mt-2 flex flex-wrap gap-1">
                  {ch.tags.map(tag => (
                    <span
                      key={tag}
                      className="rounded-full bg-surface-subtle px-2 py-0.5 text-[10px] text-content-muted">
                      {tag}
                    </span>
                  ))}
                </div>
              ) : null}
              <div className="mt-2 flex gap-1">
                <RowAction
                  label="Join"
                  disabled={busy}
                  onClick={() => run(ch.channelId, () => apiClient.channels.join(ch.channelId))}
                />
                <RowAction
                  label="Leave"
                  disabled={busy}
                  onClick={() => run(ch.channelId, () => apiClient.channels.leave(ch.channelId))}
                />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ── Groups panel ──────────────────────────────────────────────────────────────

function GroupsPanel() {
  const myAgentId = useMyAgentId();
  const params: GroupQueryParams = { limit: 20 };
  const { version, busyKey, error: actionError, run } = useRowActions();
  const [invitesGroupId, setInvitesGroupId] = useState<string | null>(null);
  const [invitesGroupName, setInvitesGroupName] = useState<string>('');
  const [showRedeem, setShowRedeem] = useState(false);

  // Fetch all public groups.
  const publicState = useAsyncCall(() => apiClient.groups.list(params), [version]);

  // Fetch groups the user belongs to (uses the `member` query param).
  // Falls back to empty list when wallet is not yet connected.
  const myGroupsState = useAsyncCall(
    () =>
      myAgentId
        ? apiClient.groups.list({ member: myAgentId })
        : Promise.resolve([] as GroupMetadata[]),
    [version, myAgentId]
  );

  if (publicState.status === 'loading') return <LoadingPane />;
  if (publicState.status === 'payment_required') return <PaymentRequiredPane />;
  if (publicState.status === 'error') return <ErrorPane message={publicState.message} />;

  const groups: GroupMetadata[] = publicState.status === 'ok' ? publicState.data : [];

  // Build a Set of group IDs the user is a member of so we can flip Join↔Leave.
  const myGroupIds = new Set<string>(
    (myGroupsState.status === 'ok'
      ? Array.isArray(myGroupsState.data)
        ? myGroupsState.data
        : []
      : []
    ).map((g: GroupMetadata) => g.groupId)
  );

  log('[groups] public=%d my=%d agent=%s', groups.length, myGroupIds.size, myAgentId ?? 'none');

  // Show the invites sub-panel for a specific group.
  if (invitesGroupId) {
    return (
      <GroupInvitesPanel
        groupId={invitesGroupId}
        groupName={invitesGroupName}
        onClose={() => setInvitesGroupId(null)}
      />
    );
  }

  // Show the redeem invite panel.
  if (showRedeem) {
    return <RedeemInvitePanel onClose={() => setShowRedeem(false)} />;
  }

  if (groups.length === 0) {
    return (
      <div className="space-y-2">
        <div className="flex justify-end">
          <Button variant="secondary" size="xs" onClick={() => setShowRedeem(true)}>
            Redeem Invite
          </Button>
        </div>
        <div className="flex items-center justify-center py-12 text-content-faint text-sm">
          No groups found
        </div>
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <div className="flex justify-end">
        <button
          type="button"
          className="rounded bg-surface-strong px-2 py-1 text-xs text-content-secondary hover:bg-stone-300 dark:hover:bg-neutral-600"
          onClick={() => setShowRedeem(true)}>
          Redeem Invite
        </button>
      </div>
      {actionError ? <ActionErrorBanner message={actionError} /> : null}
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        {groups.map(group => {
          const busy = busyKey === group.groupId;
          const isMember = myGroupIds.has(group.groupId);
          return (
            <div
              key={group.groupId}
              className="rounded-lg border border-line bg-surface-muted p-3 text-sm">
              <div className="flex items-center justify-between gap-2">
                <span className="font-medium text-content truncate">{group.name}</span>
                <span className="shrink-0 rounded-full bg-green-500/10 px-1.5 py-0.5 text-[8px] text-green-500">
                  Encrypted
                </span>
              </div>
              {group.description ? (
                <p className="mt-1 text-xs text-content-muted truncate">{group.description}</p>
              ) : null}
              <div className="mt-2 flex items-center gap-3 text-[10px] text-content-faint">
                <span>{group.memberCount} members</span>
                <span>{group.membershipPolicy}</span>
              </div>
              <div className="mt-2 flex gap-1">
                {isMember ? (
                  <RowAction
                    label="Leave"
                    disabled={busy}
                    onClick={() => {
                      log('[groups] leaving group %s', group.groupId);
                      run(group.groupId, () => apiClient.groups.leave(group.groupId));
                    }}
                  />
                ) : (
                  <RowAction
                    label="Join"
                    disabled={busy}
                    onClick={() => {
                      log('[groups] joining group %s', group.groupId);
                      run(group.groupId, () => apiClient.groups.join(group.groupId));
                    }}
                  />
                )}
                <RowAction
                  label="Invites"
                  disabled={busy}
                  onClick={() => {
                    setInvitesGroupId(group.groupId);
                    setInvitesGroupName(group.name);
                  }}
                />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ── Group invites sub-panel ──────────────────────────────────────────────────

function GroupInvitesPanel({
  groupId,
  groupName,
  onClose,
}: {
  groupId: string;
  groupName: string;
  onClose: () => void;
}) {
  const [invites, setInvites] = useState<GroupInvite[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [revoking, setRevoking] = useState<string | null>(null);

  const fetchInvites = useCallback(() => {
    setLoading(true);
    setError(null);
    void apiClient.groups
      .listInvites(groupId)
      .then(list => {
        setInvites(list);
        setLoading(false);
      })
      .catch((err: unknown) => {
        setError(String(err));
        setLoading(false);
      });
  }, [groupId]);

  useEffect(() => {
    fetchInvites();
  }, [fetchInvites]);

  const handleCreate = useCallback(async () => {
    setCreating(true);
    try {
      await apiClient.groups.createInvite(groupId);
      fetchInvites();
    } catch (err) {
      setError(String(err));
    } finally {
      setCreating(false);
    }
  }, [groupId, fetchInvites]);

  const handleRevoke = useCallback(
    async (token: string) => {
      setRevoking(token);
      try {
        await apiClient.groups.revokeInvite(groupId, token);
        fetchInvites();
      } catch (err) {
        setError(String(err));
      } finally {
        setRevoking(null);
      }
    },
    [groupId, fetchInvites]
  );

  return (
    <div className="rounded-lg border border-line bg-surface-muted p-3">
      <div className="flex items-center justify-between mb-2">
        <span className="text-sm font-medium text-content">Invites for {groupName}</span>
        <Button variant="tertiary" size="xs" onClick={onClose}>
          Close
        </Button>
      </div>
      {error ? <p className="mb-2 text-xs text-red-500">{error}</p> : null}
      {loading ? (
        <p className="text-xs text-content-faint animate-pulse">Loading...</p>
      ) : (
        <>
          {invites.length === 0 ? (
            <p className="text-xs text-content-faint">No active invites</p>
          ) : (
            <div className="space-y-1 mb-2">
              {invites.map(inv => (
                <div
                  key={inv.token}
                  className="flex items-center justify-between rounded bg-surface-subtle px-2 py-1 text-xs">
                  <div className="min-w-0 flex-1">
                    <span className="font-mono text-content-secondary truncate block">
                      {inv.token}
                    </span>
                    <span className="text-content-faint">
                      {inv.uses} uses
                      {inv.maxUses != null ? ` / ${inv.maxUses} max` : ''}
                      {inv.revoked ? ' (revoked)' : ''}
                    </span>
                  </div>
                  {!inv.revoked ? (
                    <button
                      type="button"
                      className="ml-2 shrink-0 rounded px-1.5 py-0.5 text-[10px] bg-red-100 text-red-600 hover:bg-red-200 dark:bg-red-900/30 dark:text-red-400 dark:hover:bg-red-900/50"
                      disabled={revoking === inv.token}
                      onClick={() => void handleRevoke(inv.token)}>
                      {revoking === inv.token ? '...' : 'Revoke'}
                    </button>
                  ) : null}
                </div>
              ))}
            </div>
          )}
          <Button
            variant="primary"
            size="xs"
            disabled={creating}
            onClick={() => void handleCreate()}>
            {creating ? 'Creating...' : 'Create Invite'}
          </Button>
        </>
      )}
    </div>
  );
}

// ── Redeem invite sub-panel ─────────────────────────────────────────────────

function RedeemInvitePanel({ onClose }: { onClose: () => void }) {
  const [groupId, setGroupId] = useState('');
  const [token, setToken] = useState('');
  const [preview, setPreview] = useState<GroupInvitePreview | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [redeeming, setRedeeming] = useState(false);
  const [result, setResult] = useState<GroupMember | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handlePreview = useCallback(async () => {
    if (!groupId.trim() || !token.trim()) return;
    setPreviewLoading(true);
    setError(null);
    setPreview(null);
    try {
      const p = await apiClient.groups.previewInvite(groupId.trim(), token.trim());
      setPreview(p);
    } catch (err) {
      setError(String(err));
    } finally {
      setPreviewLoading(false);
    }
  }, [groupId, token]);

  const handleRedeem = useCallback(async () => {
    if (!groupId.trim() || !token.trim()) return;
    setRedeeming(true);
    setError(null);
    try {
      const member = await apiClient.groups.redeemInvite(groupId.trim(), token.trim());
      setResult(member);
    } catch (err) {
      setError(String(err));
    } finally {
      setRedeeming(false);
    }
  }, [groupId, token]);

  return (
    <div className="rounded-lg border border-line bg-surface-muted p-3">
      <div className="flex items-center justify-between mb-2">
        <span className="text-sm font-medium text-content">Redeem Invite</span>
        <Button variant="tertiary" size="xs" onClick={onClose}>
          Close
        </Button>
      </div>
      {error ? <p className="mb-2 text-xs text-red-500">{error}</p> : null}
      {result ? (
        <p className="text-xs text-green-600 dark:text-green-400">
          Joined as {result.role} in group {result.groupId}
        </p>
      ) : (
        <>
          <div className="space-y-1.5 mb-2">
            <input
              type="text"
              placeholder="Group ID"
              value={groupId}
              onChange={e => setGroupId(e.target.value)}
              className="w-full rounded border border-line-strong bg-surface px-2 py-1 text-xs text-content placeholder:text-stone-400"
            />
            <input
              type="text"
              placeholder="Invite token"
              value={token}
              onChange={e => setToken(e.target.value)}
              className="w-full rounded border border-line-strong bg-surface px-2 py-1 text-xs text-content placeholder:text-stone-400"
            />
          </div>
          <div className="flex gap-1 mb-2">
            <Button
              variant="secondary"
              size="xs"
              disabled={previewLoading || !groupId.trim() || !token.trim()}
              onClick={() => void handlePreview()}>
              {previewLoading ? '...' : 'Preview'}
            </Button>
            <Button
              variant="primary"
              size="xs"
              disabled={redeeming || !groupId.trim() || !token.trim()}
              onClick={() => void handleRedeem()}>
              {redeeming ? '...' : 'Redeem'}
            </Button>
          </div>
          {preview ? (
            <div className="rounded bg-surface-subtle p-2 text-xs text-content-secondary">
              <p className="font-medium">{preview.name}</p>
              {preview.description ? (
                <p className="mt-0.5 text-content-faint">{preview.description}</p>
              ) : null}
              <p className="mt-0.5">
                {preview.memberCount} members / {preview.membershipPolicy} / invited by{' '}
                {preview.invitedBy}
              </p>
              {!preview.valid ? (
                <p className="mt-0.5 text-red-500">This invite is no longer valid</p>
              ) : null}
            </div>
          ) : null}
        </>
      )}
    </div>
  );
}

// ── Broadcasts panel ──────────────────────────────────────────────────────────

function BroadcastsPanel() {
  const params: BroadcastQueryParams = { limit: 20 };
  const { version, busyKey, error: actionError, run } = useRowActions();
  const state = useAsyncCall(() => apiClient.broadcasts.list(params), [version]);

  if (state.status === 'loading') return <LoadingPane />;
  if (state.status === 'payment_required') return <PaymentRequiredPane />;
  if (state.status === 'error') return <ErrorPane message={state.message} />;

  const broadcasts: BroadcastChannel[] = state.status === 'ok' ? state.data : [];

  if (broadcasts.length === 0) {
    return (
      <div className="flex items-center justify-center py-12 text-content-faint text-sm">
        No broadcasts found
      </div>
    );
  }

  return (
    <div className="space-y-2">
      {actionError ? <ActionErrorBanner message={actionError} /> : null}
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
        {broadcasts.map(bc => {
          const busy = busyKey === bc.broadcastId;
          return (
            <div
              key={bc.broadcastId}
              className="rounded-lg border border-line bg-surface-muted p-3 text-sm">
              <div className="flex items-center justify-between gap-2">
                <span className="font-medium text-content truncate">{bc.name}</span>
                <span className="shrink-0 text-xs text-content-faint">
                  {bc.subscriberCount} subs
                </span>
              </div>
              {bc.description ? (
                <p className="mt-1 text-xs text-content-muted truncate">{bc.description}</p>
              ) : null}
              <p className="mt-1 text-[10px] text-content-faint truncate">by {bc.owner}</p>
              <div className="mt-2 flex gap-1">
                <RowAction
                  label="Subscribe"
                  disabled={busy}
                  onClick={() =>
                    run(bc.broadcastId, () => apiClient.broadcasts.subscribe(bc.broadcastId))
                  }
                />
                <RowAction
                  label="Unsubscribe"
                  disabled={busy}
                  onClick={() =>
                    run(bc.broadcastId, () => apiClient.broadcasts.unsubscribe(bc.broadcastId))
                  }
                />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

// ── Inbox panel ───────────────────────────────────────────────────────────────

const TYPE_DOT_COLORS: Record<string, string> = {
  TASK_REQUEST: 'bg-blue-500',
  TASK_UPDATE: 'bg-blue-400',
  PAYMENT_RECEIVED: 'bg-green-500',
  PAYMENT_REQUIRED: 'bg-green-400',
  GROUP_INVITE: 'bg-purple-500',
  GROUP_MESSAGE: 'bg-purple-400',
  ARTIFACT_SHARED: 'bg-cyan-500',
  IDENTITY_TRANSFER: 'bg-orange-500',
  OFFER_RECEIVED: 'bg-teal-500',
  SUBSCRIPTION_EVENT: 'bg-indigo-500',
  SYSTEM: 'bg-yellow-500',
};

function formatTs(ts: string): string {
  const d = new Date(ts);
  const now = Date.now();
  const diff = now - d.getTime();
  const mins = Math.floor(diff / 60_000);
  const hours = Math.floor(diff / 3_600_000);
  const days = Math.floor(diff / 86_400_000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  if (hours < 24) return `${hours}h ago`;
  return `${days}d ago`;
}

/** Small row action button (shared across inbox / channels / groups / broadcasts). */
function RowAction({
  label,
  onClick,
  disabled,
}: {
  label: string;
  onClick: () => void;
  disabled: boolean;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      className="rounded border border-line px-1.5 py-0.5 text-[10px] font-medium text-content-secondary hover:bg-surface-hover disabled:opacity-40">
      {label}
    </button>
  );
}

/** Inline error banner for a failed row action. */
function ActionErrorBanner({ message }: { message: string }) {
  return (
    <div className="rounded border border-red-200 bg-red-50 px-3 py-2 text-[11px] text-red-700 dark:border-red-900/50 dark:bg-red-950/30 dark:text-red-300">
      {message}
    </div>
  );
}

/**
 * Shared write-action runner for list panels: tracks a refetch `version`, the
 * in-flight `busyKey`, and an `error`. `run(key, fn)` disables the row, awaits
 * the action, then bumps `version` to refetch; PaymentRequiredError surfaces a
 * clear message.
 */
function useRowActions() {
  const [version, setVersion] = useState(0);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function run(key: string, fn: () => Promise<unknown>) {
    setBusyKey(key);
    setError(null);
    try {
      await fn();
      setVersion(v => v + 1);
    } catch (err) {
      setError(
        err instanceof PaymentRequiredError ? 'Payment required for this action.' : String(err)
      );
    } finally {
      setBusyKey(null);
    }
  }

  return { version, setVersion, busyKey, error, run };
}

function InboxPanel() {
  const params: InboxQueryParams = { limit: 30 };
  const { version, busyKey, error: actionError, run: runAction, setVersion } = useRowActions();
  const itemsState = useAsyncCall(() => apiClient.inbox.list(params), [version]);
  const countsState = useAsyncCall(() => apiClient.inbox.counts(), [version]);

  // Start the inbox stream on mount, stop on unmount.
  const streamRef = useRef<string | null>(null);
  const { messages: streamMessages, status: streamStatus } = useTinyplaceStream('inbox');

  useEffect(() => {
    let cancelled = false;
    void apiClient.streams
      .start('inbox')
      .then(res => {
        if (!cancelled) streamRef.current = res.streamId;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      if (streamRef.current !== null) {
        void apiClient.streams.stop(streamRef.current).catch(() => {});
      }
    };
  }, []);

  // Bump version when a stream message arrives to trigger re-fetch.
  useEffect(() => {
    if (streamMessages.length > 0) {
      setVersion(v => v + 1);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [streamMessages.length]);

  if (itemsState.status === 'loading') return <LoadingPane />;
  if (itemsState.status === 'payment_required') return <PaymentRequiredPane />;
  if (itemsState.status === 'error') return <ErrorPane message={itemsState.message} />;

  const items: InboxItem[] = itemsState.status === 'ok' ? itemsState.data.items : [];
  const unread: number = countsState.status === 'ok' ? countsState.data.unread : 0;
  const anyBusy = busyKey !== null;

  return (
    <div className="flex flex-col overflow-hidden rounded-lg border border-line">
      <div className="flex items-center justify-between border-b border-line px-4 py-2">
        <span className="text-sm font-medium text-content">
          Inbox
          {unread > 0 ? (
            <span className="ml-2 inline-flex items-center justify-center rounded-full bg-blue-500 px-1.5 py-0.5 text-[10px] font-semibold text-white">
              {unread}
            </span>
          ) : null}
          {streamStatus === 'connected' ? (
            <span
              data-testid="inbox-live-indicator"
              className="ml-2 inline-flex items-center gap-1 text-xs text-green-600 dark:text-green-400">
              <span className="h-1.5 w-1.5 rounded-full bg-green-500 animate-pulse" />
              Live
            </span>
          ) : null}
        </span>
        {unread > 0 ? (
          <RowAction
            label="Mark all read"
            disabled={anyBusy}
            onClick={() => runAction('__all__', () => apiClient.inbox.markAllRead())}
          />
        ) : null}
      </div>
      {actionError ? (
        <div className="border-b border-red-200 bg-red-50 px-4 py-2 text-[11px] text-red-700 dark:border-red-900/50 dark:bg-red-950/30 dark:text-red-300">
          {actionError}
        </div>
      ) : null}
      {items.length === 0 ? (
        <div className="flex items-center justify-center py-12 text-content-faint text-sm">
          Your inbox is empty
        </div>
      ) : (
        <div className="divide-y divide-line dark:divide-neutral-800/50">
          {items.map(item => {
            const busy = busyKey === item.itemId;
            const archived = item.status === 'archived';
            return (
              <div key={item.itemId} className="flex items-start gap-3 px-4 py-3">
                <div
                  className={`mt-1.5 h-2 w-2 shrink-0 rounded-full ${TYPE_DOT_COLORS[item.type] ?? 'bg-stone-400 dark:bg-neutral-500'}`}
                />
                <div className="min-w-0 flex-1">
                  <p className="text-xs font-medium text-content">{item.subject}</p>
                  {item.summary ? (
                    <p className="text-[10px] text-content-muted">{item.summary}</p>
                  ) : null}
                  <p className="mt-1 text-[10px] text-content-faint">{formatTs(item.timestamp)}</p>
                </div>
                <div className="flex shrink-0 items-center gap-1">
                  {item.status === 'unread' ? (
                    <RowAction
                      label="Mark read"
                      disabled={busy || anyBusy}
                      onClick={() =>
                        runAction(item.itemId, () => apiClient.inbox.markRead(item.itemId))
                      }
                    />
                  ) : null}
                  {archived ? (
                    <RowAction
                      label="Unarchive"
                      disabled={busy || anyBusy}
                      onClick={() =>
                        runAction(item.itemId, () => apiClient.inbox.unarchive(item.itemId))
                      }
                    />
                  ) : (
                    <RowAction
                      label="Archive"
                      disabled={busy || anyBusy}
                      onClick={() =>
                        runAction(item.itemId, () => apiClient.inbox.archive(item.itemId))
                      }
                    />
                  )}
                  <RowAction
                    label="Remove"
                    disabled={busy || anyBusy}
                    onClick={() =>
                      runAction(item.itemId, () => apiClient.inbox.remove(item.itemId))
                    }
                  />
                </div>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}

// ── DMs panel (gated) ─────────────────────────────────────────────────────────

interface DecryptedMessage {
  messageId: string;
  from: string;
  plaintext: string;
  timestamp: string;
  encrypted: true;
  /**
   * True for messages WE sent. The relay only returns incoming envelopes
   * (and a sender can't decrypt their own outgoing ciphertext anyway), so we
   * keep the plaintext locally and merge it into the thread on refresh.
   */
  outgoing?: boolean;
}

function formatDirectMessageError(err: unknown, missingBundleMessage: string): string {
  const message = String(err);
  const rawBundle404 = /http\s*404/i.test(message) && /\/keys\/[^/\s]+\/bundle\b/i.test(message);
  const mappedMissingBundle =
    /recipient/i.test(message) &&
    /not\s+set\s+up/i.test(message) &&
    /encrypted\s+messaging/i.test(message);
  if (rawBundle404 || mappedMissingBundle) {
    log('[agentworld:dm] recipient key bundle missing');
    return missingBundleMessage;
  }
  return message;
}

function useDirectMessages(peerId: string, missingBundleMessage: string) {
  const [messages, setMessages] = useState<DecryptedMessage[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const response = await apiClient.messages.list({ limit: 50 });
      const fromPeer = response.messages.filter(m => m.from === peerId);
      const decrypted: DecryptedMessage[] = [];
      for (const env of fromPeer) {
        try {
          const result = await apiClient.signal.decryptMessage({ envelope: env });
          decrypted.push({
            messageId: result.messageId,
            from: result.from,
            plaintext: result.plaintext,
            timestamp: env.timestamp,
            encrypted: true,
          });
        } catch (decryptErr) {
          log('failed to decrypt message %s: %s', env.id, String(decryptErr));
        }
      }
      // Merge fresh incoming with the outgoing messages we sent this session —
      // the relay only echoes incoming, so replacing would wipe our own sent
      // messages. De-dupe by messageId, keep chronological order.
      setMessages(prev => {
        const outgoing = prev.filter(m => m.outgoing);
        const seen = new Set(decrypted.map(m => m.messageId));
        const merged = [...decrypted, ...outgoing.filter(m => !seen.has(m.messageId))];
        return merged.sort((a, b) => a.timestamp.localeCompare(b.timestamp));
      });
    } catch (err) {
      setError(formatDirectMessageError(err, missingBundleMessage));
    } finally {
      setLoading(false);
    }
  }, [missingBundleMessage, peerId]);

  const send = useCallback(
    async (plaintext: string) => {
      setSending(true);
      setError(null);
      try {
        const result = await apiClient.signal.sendMessage({ recipient: peerId, plaintext });
        // Optimistically show our own message: the relay won't return it and we
        // can't decrypt our own ciphertext, so keep the plaintext locally.
        const sentMessage: DecryptedMessage = {
          messageId: result?.messageId ?? `local-${peerId}-${plaintext.length}-${Date.now()}`,
          from: 'You',
          plaintext,
          timestamp: new Date().toISOString(),
          encrypted: true,
          outgoing: true,
        };
        setMessages(prev =>
          [...prev, sentMessage].sort((a, b) => a.timestamp.localeCompare(b.timestamp))
        );
        await refresh();
      } catch (err) {
        setError(formatDirectMessageError(err, missingBundleMessage));
      } finally {
        setSending(false);
      }
    },
    [missingBundleMessage, peerId, refresh]
  );

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { messages, loading, error, sending, send, refresh };
}

function ActiveDmView({
  peerId,
  onBack,
  composeText,
  setComposeText,
}: {
  peerId: string;
  onBack: () => void;
  composeText: string;
  setComposeText: (v: string) => void;
}) {
  const { t } = useT();
  const missingBundleMessage = t(
    'agentworld.messaging.missingSignalBundle',
    "This user hasn't enabled encrypted messaging yet. Ask them to open Agent World and enable secure DMs before sending a message."
  );
  const { messages, loading, error, sending, send } = useDirectMessages(
    peerId,
    missingBundleMessage
  );

  const handleSend = useCallback(async () => {
    if (!composeText.trim()) return;
    await send(composeText.trim());
    setComposeText('');
  }, [composeText, send, setComposeText]);

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div className="flex items-center gap-2 border-b border-line px-3 py-2">
        <Button variant="tertiary" size="xs" onClick={onBack}>
          Back
        </Button>
        <span className="text-sm font-medium text-content truncate">{peerId}</span>
        <span className="ml-auto flex items-center gap-1 text-[10px] text-green-600 dark:text-green-400">
          <svg
            className="h-3 w-3"
            fill="none"
            stroke="currentColor"
            strokeWidth={2}
            viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              d="M16.5 10.5V6.75a4.5 4.5 0 10-9 0v3.75m-.75 11.25h10.5a2.25 2.25 0 002.25-2.25v-6.75a2.25 2.25 0 00-2.25-2.25H6.75a2.25 2.25 0 00-2.25 2.25v6.75a2.25 2.25 0 002.25 2.25z"
            />
          </svg>
          Encrypted
        </span>
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-auto p-3 space-y-2">
        {loading && messages.length === 0 ? (
          <p className="text-xs text-content-faint animate-pulse">Loading encrypted messages...</p>
        ) : null}
        {error ? <p className="text-xs text-red-500">{error}</p> : null}
        {!loading && !error && messages.length === 0 ? (
          <div
            data-testid="dm-empty-state"
            className="flex h-full flex-col items-center justify-center gap-1 text-center">
            <p className="text-sm font-medium text-content-muted">No messages yet</p>
            <p className="text-xs text-content-faint">
              Send the first end-to-end encrypted message below to start the conversation.
            </p>
          </div>
        ) : null}
        {messages.map(msg => (
          <div
            key={msg.messageId}
            className={`max-w-[80%] rounded-lg px-3 py-2 text-sm ${
              msg.outgoing
                ? 'ml-auto bg-primary-500/15 dark:bg-primary-500/20'
                : 'mr-auto bg-surface-subtle'
            }`}>
            <p className="text-content">{msg.plaintext}</p>
            <p className="mt-1 text-[10px] text-content-faint">
              {msg.from} &middot; {msg.timestamp}
            </p>
          </div>
        ))}
      </div>

      {/* Compose */}
      <div className="border-t border-line p-3 flex gap-2">
        <input
          type="text"
          value={composeText}
          onChange={e => setComposeText(e.target.value)}
          onKeyDown={e => {
            if (e.key === 'Enter' && !e.shiftKey) void handleSend();
          }}
          placeholder="Type a message..."
          className="flex-1 rounded border border-line-strong bg-surface px-3 py-2 text-sm text-content placeholder:text-stone-400"
        />
        <Button
          variant="primary"
          size="md"
          disabled={sending || !composeText.trim()}
          onClick={() => void handleSend()}>
          {sending ? 'Sending...' : 'Send'}
        </Button>
      </div>
    </div>
  );
}

/** Quick heuristic matching Rust `looks_like_base58_pubkey`: 32–44 chars of
 *  the Bitcoin base58 alphabet [1-9A-HJ-NP-Za-km-z]. */
function looksLikeBase58Pubkey(s: string): boolean {
  return s.length >= 32 && s.length <= 44 && /^[1-9A-HJ-NP-Za-km-z]+$/.test(s);
}

function DmsPanel() {
  const [peerId, setPeerId] = useState('');
  const [activePeer, setActivePeer] = useState<string | null>(null);
  const [composeText, setComposeText] = useState('');
  const [resolving, setResolving] = useState(false);
  const [resolveError, setResolveError] = useState<string | null>(null);

  if (!E2E_MESSAGING_ENABLED) {
    return (
      <div
        data-testid="dms-coming-soon"
        className="flex flex-col items-center justify-center gap-3 rounded-lg border border-line bg-surface p-12 text-center">
        <div className="flex h-10 w-10 items-center justify-center rounded-full bg-surface-subtle">
          <svg
            aria-hidden="true"
            className="h-5 w-5 text-content-muted"
            fill="none"
            stroke="currentColor"
            strokeWidth={1.5}
            viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              d="M16.5 10.5V6.75A4.5 4.5 0 0 0 12 2.25 4.5 4.5 0 0 0 7.5 6.75v3.75m-2.25 0h13.5c.621 0 1.125.504 1.125 1.125v7.5c0 .621-.504 1.125-1.125 1.125H5.25A1.125 1.125 0 0 1 4.125 19.125v-7.5c0-.621.504-1.125 1.125-1.125Z"
            />
          </svg>
        </div>
        <div>
          <p className="text-sm font-medium text-content">Secure direct messages — coming soon</p>
          <p className="mt-1 text-xs text-content-faint">
            End-to-end encrypted DMs use the Signal protocol. Full support is in progress.
          </p>
        </div>
      </div>
    );
  }

  if (activePeer) {
    return (
      <ActiveDmView
        peerId={activePeer}
        onBack={() => setActivePeer(null)}
        composeText={composeText}
        setComposeText={setComposeText}
      />
    );
  }

  const handleOpenDm = async () => {
    const input = peerId.trim();
    if (!input) return;

    setResolving(true);
    setResolveError(null);

    try {
      // If the input looks like a raw crypto_id (base58 wallet address) we can
      // open the DM directly — no resolution round-trip needed. The Rust handler
      // will do the same check and use it verbatim.
      if (looksLikeBase58Pubkey(input)) {
        setActivePeer(input);
        return;
      }

      // Handle or bare name: resolve via directory first for immediate UX feedback.
      const name = input.startsWith('@') ? input.slice(1) : input;
      const resolved = await apiClient.directory.resolve(name);
      const identity = resolved?.identity as
        | { cryptoId?: string; [key: string]: unknown }
        | null
        | undefined;
      const agent = resolved?.agent as { cryptoId?: string; [key: string]: unknown } | null;

      const cryptoId = identity?.cryptoId ?? agent?.cryptoId;
      if (cryptoId) {
        setActivePeer(cryptoId);
      } else {
        setResolveError(`No agent found for "${input}"`);
      }
    } catch (err) {
      setResolveError(String(err));
    } finally {
      setResolving(false);
    }
  };

  return (
    <div className="space-y-3">
      <div className="flex gap-2">
        <input
          type="text"
          value={peerId}
          onChange={e => {
            setPeerId(e.target.value);
            setResolveError(null);
          }}
          placeholder="Recipient @handle or wallet address"
          className="flex-1 rounded border border-line-strong bg-surface px-3 py-2 text-sm text-content placeholder:text-stone-400"
        />
        <Button
          variant="primary"
          size="md"
          disabled={!peerId.trim() || resolving}
          onClick={() => void handleOpenDm()}>
          {resolving ? 'Resolving...' : 'Open DM'}
        </Button>
      </div>
      {resolveError && (
        <p data-testid="dm-resolve-error" className="text-xs text-red-500 dark:text-red-400">
          {resolveError}
        </p>
      )}
    </div>
  );
}

// ── Messaging section root ────────────────────────────────────────────────────

// Messaging is currently focused on DMs only. The channels / groups /
// broadcasts / inbox panels (and their tab bar) are intentionally hidden — add
// their slugs back here to restore the full tab bar; the panels below stay
// wired up (hidden, not removed). With a single visible tab the bar is hidden
// entirely so the surface goes straight to DMs.
const VISIBLE_TABS: readonly Tab[] = ['dms'];

interface MessagingSectionProps {
  /**
   * Which tabs to surface. Defaults to DMs-only (the tab bar hides itself when a
   * single tab is visible). Overridable so the hidden panels stay testable.
   */
  tabs?: readonly Tab[];
}

export default function MessagingSection({ tabs = VISIBLE_TABS }: MessagingSectionProps = {}) {
  const [activeTab, setActiveTab] = useState<Tab>(tabs[0]);
  const showTabBar = tabs.length > 1;

  return (
    <div className="flex flex-col h-full">
      {/* Tab chips — only shown when more than one tab is enabled. */}
      {showTabBar && (
        <ChipTabs<Tab>
          as="tab"
          ariaLabel="Messaging sections"
          className="flex gap-1 px-4 py-3 border-b border-line overflow-x-auto shrink-0"
          items={tabs.map(tab => ({ id: tab, label: TAB_LABELS[tab] }))}
          value={activeTab}
          onChange={setActiveTab}
        />
      )}

      {/* Signal key status — always visible when wallet is connected */}
      <SignalKeyStatusCard />

      {/* Active panel */}
      <div className="flex-1 overflow-auto p-4">
        {activeTab === 'channels' && <ChannelsPanel />}
        {activeTab === 'groups' && <GroupsPanel />}
        {activeTab === 'broadcasts' && <BroadcastsPanel />}
        {activeTab === 'inbox' && <InboxPanel />}
        {activeTab === 'dms' && <DmsPanel />}
      </div>
    </div>
  );
}
