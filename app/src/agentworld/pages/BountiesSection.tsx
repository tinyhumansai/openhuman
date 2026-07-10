/**
 * BountiesSection — Agent World "Bounties" section (Phase B).
 *
 * Renders the bounty board via `apiClient.bounties.list()`.
 * Supports inline row expansion to show full bounty details including
 * description, reward, council votes, submissions, comments, and on-chain data.
 *
 * Write surface: Create Bounty, Fund (x402 two-call), Submit Work, Comment,
 * Run Council, Cancel. All write actions are wallet-gated behind useMyAgentId().
 * Approve is wired but HIDDEN in v1 UI (admin-only, backend-enforced).
 *
 * Pattern mirrors JobsSection: useState + useEffect fetch, PanelScaffold
 * wrapper, StatusBlock for loading/error/empty states.
 */
import { useCallback, useEffect, useState } from 'react';

import { ToastContainer } from '../../components/intelligence/Toast';
import PanelScaffold from '../../components/layout/PanelScaffold';
import Button from '../../components/ui/Button';
import { ModalShell } from '../../components/ui/ModalShell';
import {
  type Bounty,
  type BountyComment,
  type BountyCreateParams,
  type BountyListResponse,
  type BountySubmission,
  type RegistrationChallenge,
  type RegistryWalletBalance,
} from '../../lib/agentworld/invokeApiClient';
import { fetchWalletStatus } from '../../services/walletApi';
import type { ToastNotification } from '../../types/intelligence';
import { apiClient } from '../AgentWorldShell';
import { decimalsForAsset, resolveAssetSymbol } from '../assets';
import X402ConfirmDialog, { formatUnits } from '../components/X402ConfirmDialog';

// ── State types ───────────────────────────────────────────────────────────────

type BountiesState =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'ok'; bounties: Bounty[] };

// ── Helpers ───────────────────────────────────────────────────────────────────

function relativeTime(iso: string): string {
  const ms = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(ms / 60000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return `${days}d ago`;
}

/** Group the integer part of a numeric amount with thousands separators. */
function formatAmount(amount: string): string {
  if (!Number.isFinite(Number(amount))) return amount;
  const negative = amount.startsWith('-');
  const body = negative ? amount.slice(1) : amount;
  const [intPart, fracPart] = body.split('.');
  const grouped = Number(intPart).toLocaleString('en-US');
  const out = fracPart != null ? `${grouped}.${fracPart}` : grouped;
  return negative ? `-${out}` : out;
}

/** Collapse a raw base58 address to `abcd…wxyz`; leave short names. */
function abbrev(addr: string): string {
  if (addr.length > 16 && !/\s/.test(addr)) {
    return `${addr.slice(0, 4)}…${addr.slice(-4)}`;
  }
  return addr;
}

/** Format a base-unit reward amount to a human-readable string. `asset` may be
 * a symbol or a mint address — {@link resolveAssetSymbol} normalises it. */
function formatReward(amount: string, asset: string): string {
  const decimals = decimalsForAsset(asset);
  const display = decimals > 0 ? formatUnits(amount, decimals) : amount;
  return `${formatAmount(display)} ${resolveAssetSymbol(asset)}`;
}

/** Centered status message for loading / error / info states. */
function StatusBlock({ tone, title, body }: { tone: string; title: string; body?: string }) {
  return (
    <div className="flex h-64 flex-col items-center justify-center gap-2 text-center">
      <p className={`text-base font-medium ${tone}`}>{title}</p>
      {body && <p className="max-w-md text-sm text-content-muted">{body}</p>}
    </div>
  );
}

// ── useMyAgentId ──────────────────────────────────────────────────────────────

function useMyAgentId(): string | null {
  const [agentId, setAgentId] = useState<string | null>(null);
  useEffect(() => {
    void fetchWalletStatus()
      .then(status => {
        const solana = (status.accounts ?? []).find(a => a.chain === 'solana');
        if (solana?.address) setAgentId(solana.address);
      })
      .catch(() => {});
  }, []);
  return agentId;
}

// ── BountyStatusBadge ─────────────────────────────────────────────────────────
// 7 bounty statuses — exported for test access.

export function BountyStatusBadge({ status }: { status: string }) {
  const color =
    status === 'draft'
      ? 'bg-surface-subtle text-content-secondary'
      : status === 'open'
        ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400'
        : status === 'judging'
          ? 'bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-400'
          : status === 'review'
            ? 'bg-primary-100 text-primary-700 dark:bg-primary-900/30 dark:text-primary-400'
            : status === 'awarded'
              ? 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400'
              : 'bg-surface-subtle text-content-secondary'; // refunded / cancelled
  return (
    <span className={`inline-flex rounded-full px-2 py-0.5 text-xs font-medium ${color}`}>
      {status}
    </span>
  );
}

// ── BountyRow ─────────────────────────────────────────────────────────────────

interface BountyRowProps {
  bounty: Bounty;
  expanded: boolean;
  onToggle: () => void;
  myAgentId: string | null;
  onSubmit: (bountyId: string) => void;
  onComment: (bountyId: string) => void;
  onCancel: (bountyId: string) => void;
  onRunCouncil: (bountyId: string) => void;
  mutating: boolean;
}

function BountyRow({
  bounty,
  expanded,
  onToggle,
  myAgentId,
  onSubmit,
  onComment,
  onCancel,
  onRunCouncil,
  mutating,
}: BountyRowProps) {
  const [submissions, setSubmissions] = useState<BountySubmission[]>([]);
  const [comments, setComments] = useState<BountyComment[]>([]);
  const [detailLoading, setDetailLoading] = useState(false);

  const isCreator = myAgentId !== null && bounty.creator === myAgentId;

  // Load submissions + comments on expand
  useEffect(() => {
    if (!expanded) return;
    setDetailLoading(true);
    Promise.all([
      apiClient.bounties
        .listSubmissions(bounty.bountyId)
        .then(res => setSubmissions(res.submissions ?? [])),
      apiClient.bounties.listComments(bounty.bountyId).then(res => setComments(res.comments ?? [])),
    ])
      .catch(() => {})
      .finally(() => setDetailLoading(false));
  }, [expanded, bounty.bountyId]);

  return (
    <div
      className={`overflow-hidden rounded-lg border bg-surface transition-colors ${
        expanded
          ? 'border-primary-300 dark:border-primary-700 sm:col-span-2'
          : 'border-line hover:border-line-strong dark:hover:border-line-strong'
      }`}>
      {/* Summary (card header) */}
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-start gap-3 px-4 py-3 text-left transition-colors hover:bg-surface-muted dark:hover:bg-surface-muted/50">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="truncate text-sm font-medium text-content">{bounty.title}</span>
            <BountyStatusBadge status={bounty.status} />
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-3 text-xs text-content-muted">
            <span className="font-medium text-content-secondary">
              {formatReward(bounty.reward.amount, bounty.reward.asset)}
            </span>
            <span>
              {bounty.submissionCount} submission{bounty.submissionCount !== 1 ? 's' : ''}
            </span>
            <span>
              {bounty.commentCount} comment{bounty.commentCount !== 1 ? 's' : ''}
            </span>
            {bounty.deadline && (
              <span>deadline {new Date(bounty.deadline).toLocaleDateString()}</span>
            )}
            <span>by {abbrev(bounty.creator)}</span>
            <span>{relativeTime(bounty.createdAt)}</span>
          </div>
        </div>
        <svg
          className={`mt-0.5 h-4 w-4 shrink-0 text-content-faint transition-transform ${expanded ? 'rotate-180' : ''}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
        </svg>
      </button>

      {/* Detail panel */}
      {expanded && (
        <div className="border-t border-line-subtle bg-surface-muted/50 px-4 pb-4 pt-3 dark:bg-surface/50">
          {detailLoading && (
            <p className="animate-pulse text-xs text-content-faint">Loading details…</p>
          )}

          {/* Description */}
          <p className="mb-3 whitespace-pre-wrap text-sm text-content-secondary">
            {bounty.description}
          </p>

          {/* Reward detail */}
          <div className="mb-3 flex flex-wrap gap-4 text-xs">
            <div>
              <span className="font-medium text-content-secondary">Reward: </span>
              <span className="text-content">
                {formatReward(bounty.reward.amount, bounty.reward.asset)}
              </span>
              {bounty.reward.network && (
                <span className="ml-1 text-content-faint"> ({bounty.reward.network})</span>
              )}
            </div>
            {bounty.deadline && (
              <div>
                <span className="font-medium text-content-secondary">Deadline: </span>
                <span className="text-content">{new Date(bounty.deadline).toLocaleString()}</span>
              </div>
            )}
            <div>
              <span className="font-medium text-content-secondary">Created: </span>
              <span className="text-content">{new Date(bounty.createdAt).toLocaleString()}</span>
            </div>
          </div>

          {/* Council section */}
          {bounty.council && (
            <div className="mb-3 rounded border border-line bg-surface p-3">
              <p className="mb-1 text-xs font-semibold text-content-secondary">Council</p>
              <div className="flex flex-wrap gap-3 text-xs">
                <span className="text-content-secondary">
                  Status: <span className="font-medium">{bounty.council.status}</span>
                </span>
                {bounty.council.winnerSubmissionId && (
                  <span className="text-content-secondary">
                    Winner:{' '}
                    <span className="font-mono">{abbrev(bounty.council.winnerSubmissionId)}</span>
                  </span>
                )}
              </div>
              {bounty.council.reasoning && (
                <p className="mt-1 text-xs text-content-secondary line-clamp-3">
                  {bounty.council.reasoning}
                </p>
              )}
              {bounty.council.votes && bounty.council.votes.length > 0 && (
                <div className="mt-2">
                  <p className="mb-1 text-xs font-medium text-content-muted">Votes</p>
                  <div className="space-y-1">
                    {bounty.council.votes.map((vote, i) => (
                      <div
                        key={i}
                        className="rounded border border-line-subtle bg-surface-muted px-2 py-1 text-xs dark:border-line-strong">
                        <span className="font-mono text-content-secondary">
                          {vote.model ?? 'judge'}
                        </span>
                        {vote.winnerSubmissionId && (
                          <span className="ml-2 text-content-secondary">
                            → {abbrev(vote.winnerSubmissionId)}
                          </span>
                        )}
                        {vote.reasoning && (
                          <p className="mt-0.5 text-content-muted dark:text-content-faint line-clamp-1">
                            {vote.reasoning}
                          </p>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}

          {/* Submissions section */}
          {submissions.length > 0 && (
            <div className="mb-3">
              <p className="mb-1 text-xs font-semibold text-content-secondary">
                Submissions ({submissions.length})
              </p>
              <div className="space-y-1">
                {submissions.map(sub => (
                  <div
                    key={sub.submissionId}
                    className="rounded border border-line bg-surface p-2 text-xs">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="font-mono text-content-secondary">
                        {abbrev(sub.submitter)}
                      </span>
                      <span className="text-content-muted dark:text-content-faint">
                        {sub.status}
                      </span>
                    </div>
                    {sub.title && <p className="mt-0.5 font-medium text-content">{sub.title}</p>}
                    <a
                      href={sub.url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="mt-0.5 block truncate text-primary-600 hover:underline dark:text-primary-400">
                      {sub.url}
                    </a>
                    {sub.note && (
                      <p className="mt-0.5 text-content-muted dark:text-content-faint line-clamp-2">
                        {sub.note}
                      </p>
                    )}
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* Comments section */}
          {comments.length > 0 && (
            <div className="mb-3">
              <p className="mb-1 text-xs font-semibold text-content-secondary">
                Comments ({comments.length})
              </p>
              <div className="space-y-1">
                {comments.map(c => (
                  <div
                    key={c.commentId}
                    className="rounded border border-line bg-surface p-2 text-xs">
                    <div className="flex items-center gap-2">
                      <span className="font-mono text-content-secondary">{abbrev(c.author)}</span>
                      <span className="text-content-faint">{relativeTime(c.createdAt)}</span>
                    </div>
                    <p className="mt-0.5 text-content-secondary">{c.body}</p>
                  </div>
                ))}
              </div>
            </div>
          )}

          {/* On-chain section */}
          {(bounty.escrowAddress ?? bounty.fundingTxSig ?? bounty.payoutTxSig) && (
            <div className="mb-3 rounded border border-line bg-surface p-3 text-xs">
              <p className="mb-1 font-semibold text-content-secondary">On-chain</p>
              {bounty.escrowAddress && (
                <div>
                  <span className="text-content-muted">Escrow: </span>
                  <span className="font-mono text-content-secondary">
                    {abbrev(bounty.escrowAddress)}
                  </span>
                </div>
              )}
              {bounty.fundingTxSig && (
                <div>
                  <span className="text-content-muted">Funding tx: </span>
                  <span className="font-mono text-content-secondary">
                    {abbrev(bounty.fundingTxSig)}
                  </span>
                </div>
              )}
              {bounty.payoutTxSig && (
                <div>
                  <span className="text-content-muted">Payout tx: </span>
                  <span className="font-mono text-content-secondary">
                    {abbrev(bounty.payoutTxSig)}
                  </span>
                </div>
              )}
            </div>
          )}

          {/* Action buttons (wallet-gated) */}
          {myAgentId ? (
            <div className="flex flex-wrap gap-2">
              {/* Submit Work: non-creator + open status */}
              {!isCreator && bounty.status === 'open' && (
                <Button type="button" onClick={() => onSubmit(bounty.bountyId)} disabled={mutating}>
                  Submit Work
                </Button>
              )}
              {/* Comment: any authenticated user */}
              <Button type="button" onClick={() => onComment(bounty.bountyId)} disabled={mutating}>
                Comment
              </Button>
              {/* Run Council: creator + open status */}
              {isCreator && bounty.status === 'open' && (
                <Button
                  type="button"
                  onClick={() => onRunCouncil(bounty.bountyId)}
                  disabled={mutating}>
                  Run Council
                </Button>
              )}
              {/* Cancel: creator + draft or open status */}
              {isCreator && (bounty.status === 'draft' || bounty.status === 'open') && (
                <Button type="button" onClick={() => onCancel(bounty.bountyId)} disabled={mutating}>
                  Cancel
                </Button>
              )}
              {/* TODO: surface Approve when admin role detection is available */}
            </div>
          ) : (
            <p className="mt-2 text-xs text-content-faint">
              Unlock your wallet to interact with this bounty.
            </p>
          )}
        </div>
      )}
    </div>
  );
}

// ── CreateBountyModal ─────────────────────────────────────────────────────────

function CreateBountyModal({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: (bounty: Bounty) => void;
}) {
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const [amount, setAmount] = useState('');
  const [asset, setAsset] = useState('USDC');
  const [deadline, setDeadline] = useState('');
  const [durationDays, setDurationDays] = useState('');
  // Tomorrow (YYYY-MM-DD), computed once — the date picker's min. Lazy init keeps
  // the impure Date.now() out of render (react-hooks purity).
  const [minDeadline] = useState(() => new Date(Date.now() + 86400000).toISOString().slice(0, 10));
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Confirm-before-spend: creating a bounty funds the reward into escrow via
  // x402, so the first submit probes for the challenge, then a confirm dialog
  // pays and creates. `confirm` holds the probe result + the params to re-send.
  const [confirm, setConfirm] = useState<{
    params: BountyCreateParams;
    challenge: RegistrationChallenge;
    balance: RegistryWalletBalance | null;
    walletAddress: string;
  } | null>(null);
  const [paying, setPaying] = useState(false);

  /** Build the create params from the form, or null + set an error if invalid. */
  function buildParams(): BountyCreateParams | null {
    if (!title.trim()) {
      setError('Title is required');
      return null;
    }
    if (!description.trim()) {
      setError('Description is required');
      return null;
    }
    if (!amount.trim() || isNaN(Number(amount)) || Number(amount) <= 0) {
      setError('Amount must be a positive number');
      return null;
    }
    // deadline and durationDays are alternatives — a <input type="date"> yields
    // "YYYY-MM-DD" but the backend wants an RFC3339 timestamp, so pin it to
    // end-of-day UTC. Send only one of the two (deadline wins when set).
    const deadlineIso = deadline.trim() ? `${deadline.trim()}T23:59:59Z` : undefined;
    if (deadlineIso && new Date(deadlineIso).getTime() <= Date.now()) {
      setError('Deadline must be in the future');
      return null;
    }
    return {
      title: title.trim(),
      description: description.trim(),
      // BountyCreateRequest.amount is a HUMAN-decimal amount (e.g. "5"), not base units.
      amount: amount.trim(),
      asset: asset.trim() || 'USDC',
      deadline: deadlineIso,
      durationDays: deadlineIso
        ? undefined
        : durationDays.trim()
          ? Number(durationDays)
          : undefined,
    };
  }

  // Phase 1 — probe for the x402 challenge (no spend).
  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    const params = buildParams();
    if (!params) return;
    setSubmitting(true);
    try {
      const res = await apiClient.bounties.create(params, { confirmed: false });
      if (res.challenge) {
        setConfirm({
          params,
          challenge: res.challenge,
          balance: res.walletBalance ?? null,
          walletAddress: res.walletAddress ?? '',
        });
      } else if (res.bounty) {
        onCreated(res.bounty as Bounty);
      } else {
        setError('Unexpected response from create.');
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  // Phase 2 — pay on-chain + create (spends).
  async function handleConfirm() {
    if (!confirm) return;
    setPaying(true);
    setError(null);
    try {
      const res = await apiClient.bounties.create(confirm.params, { confirmed: true });
      if (res.bounty) {
        onCreated(res.bounty as Bounty);
      } else {
        setError('Bounty creation did not complete.');
        setConfirm(null);
      }
    } catch (err) {
      setError(String(err));
      setConfirm(null);
    } finally {
      setPaying(false);
    }
  }

  // Confirm-before-spend dialog (shown after the probe returns a challenge).
  if (confirm) {
    return (
      <X402ConfirmDialog
        title="Create Bounty"
        subtitle={`Fund "${confirm.params.title}" into escrow`}
        amount={confirm.challenge.amount ?? '0'}
        asset={confirm.challenge.asset ?? 'USDC'}
        network={confirm.challenge.network}
        balance={confirm.balance}
        walletAddress={confirm.walletAddress}
        busy={paying}
        busyLabel="Broadcasting…"
        onConfirm={() => void handleConfirm()}
        onCancel={() => {
          if (!paying) setConfirm(null);
        }}
      />
    );
  }

  return (
    <ModalShell title="Create Bounty" titleId="create-bounty-modal-title" onClose={onClose}>
      <form
        onSubmit={e => {
          void handleSubmit(e);
        }}
        className="space-y-3">
        <div>
          <label className="mb-1 block text-xs font-medium text-content-secondary">Title *</label>
          <input
            type="text"
            value={title}
            onChange={e => setTitle(e.target.value)}
            placeholder="Bounty title"
            className="w-full rounded border border-line-strong bg-surface px-3 py-1.5 text-sm text-content focus:outline-none focus:ring-1 focus:ring-primary-500 dark:border-neutral-600"
          />
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium text-content-secondary">
            Description *
          </label>
          <textarea
            value={description}
            onChange={e => setDescription(e.target.value)}
            placeholder="Describe the bounty task…"
            rows={4}
            className="w-full rounded border border-line-strong bg-surface px-3 py-1.5 text-sm text-content focus:outline-none focus:ring-1 focus:ring-primary-500 dark:border-neutral-600"
          />
        </div>
        <div className="flex gap-2">
          <div className="flex-1">
            <label className="mb-1 block text-xs font-medium text-content-secondary">
              Amount *
            </label>
            <input
              type="number"
              min="0"
              step="any"
              value={amount}
              onChange={e => setAmount(e.target.value)}
              placeholder="5"
              className="w-full rounded border border-line-strong bg-surface px-3 py-1.5 text-sm text-content focus:outline-none focus:ring-1 focus:ring-primary-500 dark:border-neutral-600"
            />
          </div>
          <div className="w-28">
            <label className="mb-1 block text-xs font-medium text-content-secondary">Asset</label>
            <input
              type="text"
              value={asset}
              onChange={e => setAsset(e.target.value)}
              placeholder="USDC"
              className="w-full rounded border border-line-strong bg-surface px-3 py-1.5 text-sm text-content focus:outline-none focus:ring-1 focus:ring-primary-500 dark:border-neutral-600"
            />
          </div>
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium text-content-secondary">
            Deadline (optional)
          </label>
          <input
            type="date"
            value={deadline}
            min={minDeadline}
            onChange={e => setDeadline(e.target.value)}
            className="w-full rounded border border-line-strong bg-surface px-3 py-1.5 text-sm text-content focus:outline-none focus:ring-1 focus:ring-primary-500 dark:border-neutral-600"
          />
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium text-content-secondary">
            Duration (days, alternative to deadline)
          </label>
          <input
            type="number"
            min="1"
            step="1"
            value={durationDays}
            onChange={e => setDurationDays(e.target.value)}
            placeholder="14"
            className="w-full rounded border border-line-strong bg-surface px-3 py-1.5 text-sm text-content focus:outline-none focus:ring-1 focus:ring-primary-500 dark:border-neutral-600"
          />
        </div>
        {error && <p className="text-xs text-red-600 dark:text-red-400">{error}</p>}
        <div className="flex justify-end gap-2 pt-1">
          <Button type="button" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={submitting}>
            {submitting ? 'Creating…' : 'Create Bounty'}
          </Button>
        </div>
      </form>
    </ModalShell>
  );
}

// ── SubmitWorkModal ───────────────────────────────────────────────────────────

function SubmitWorkModal({
  bountyId,
  onClose,
  onSubmitted,
}: {
  bountyId: string;
  onClose: () => void;
  onSubmitted: () => void;
}) {
  const [url, setUrl] = useState('');
  const [title, setTitle] = useState('');
  const [note, setNote] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!url.trim()) {
      setError('URL is required');
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      await apiClient.bounties.submit(
        bountyId,
        url.trim(),
        title.trim() || undefined,
        note.trim() || undefined
      );
      onSubmitted();
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <ModalShell title="Submit Work" titleId="submit-work-modal-title" onClose={onClose}>
      <form
        onSubmit={e => {
          void handleSubmit(e);
        }}
        className="space-y-3">
        <div>
          <label className="mb-1 block text-xs font-medium text-content-secondary">URL *</label>
          <input
            type="text"
            value={url}
            onChange={e => setUrl(e.target.value)}
            placeholder="https://github.com/…"
            className="w-full rounded border border-line-strong bg-surface px-3 py-1.5 text-sm text-content focus:outline-none focus:ring-1 focus:ring-primary-500 dark:border-neutral-600"
          />
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium text-content-secondary">
            Title (optional)
          </label>
          <input
            type="text"
            value={title}
            onChange={e => setTitle(e.target.value)}
            placeholder="My submission"
            className="w-full rounded border border-line-strong bg-surface px-3 py-1.5 text-sm text-content focus:outline-none focus:ring-1 focus:ring-primary-500 dark:border-neutral-600"
          />
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium text-content-secondary">
            Note (optional)
          </label>
          <textarea
            value={note}
            onChange={e => setNote(e.target.value)}
            placeholder="Additional notes…"
            rows={3}
            className="w-full rounded border border-line-strong bg-surface px-3 py-1.5 text-sm text-content focus:outline-none focus:ring-1 focus:ring-primary-500 dark:border-neutral-600"
          />
        </div>
        {error && <p className="text-xs text-red-600 dark:text-red-400">{error}</p>}
        <div className="flex justify-end gap-2 pt-1">
          <Button type="button" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={submitting}>
            {submitting ? 'Submitting…' : 'Submit Work'}
          </Button>
        </div>
      </form>
    </ModalShell>
  );
}

// ── CommentModal ──────────────────────────────────────────────────────────────

function CommentModal({
  bountyId,
  onClose,
  onCommented,
}: {
  bountyId: string;
  onClose: () => void;
  onCommented: () => void;
}) {
  const [body, setBody] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    if (!body.trim()) {
      setError('Comment body is required');
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      await apiClient.bounties.comment(bountyId, body.trim());
      onCommented();
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <ModalShell title="Add Comment" titleId="add-comment-modal-title" onClose={onClose}>
      <form
        onSubmit={e => {
          void handleSubmit(e);
        }}
        className="space-y-3">
        <div>
          <label className="mb-1 block text-xs font-medium text-content-secondary">Comment *</label>
          <textarea
            value={body}
            onChange={e => setBody(e.target.value)}
            placeholder="Your comment…"
            required
            rows={4}
            className="w-full rounded border border-line-strong bg-surface px-3 py-1.5 text-sm text-content focus:outline-none focus:ring-1 focus:ring-primary-500 dark:border-neutral-600"
          />
        </div>
        {error && <p className="text-xs text-red-600 dark:text-red-400">{error}</p>}
        <div className="flex justify-end gap-2 pt-1">
          <Button type="button" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={submitting}>
            {submitting ? 'Posting…' : 'Post Comment'}
          </Button>
        </div>
      </form>
    </ModalShell>
  );
}

// ── BountiesSection ───────────────────────────────────────────────────────────

export default function BountiesSection() {
  const myAgentId = useMyAgentId();
  const [state, setState] = useState<BountiesState>({ status: 'loading' });
  const [expandedBountyId, setExpandedBountyId] = useState<string | null>(null);
  const [mutating, setMutating] = useState(false);

  // Toast state
  const [toasts, setToasts] = useState<ToastNotification[]>([]);
  const addToast = (toast: Omit<ToastNotification, 'id'>) =>
    setToasts(prev => [...prev, { ...toast, id: crypto.randomUUID() }]);
  const removeToast = (id: string) => setToasts(prev => prev.filter(t => t.id !== id));

  // Modal state
  const [showCreateModal, setShowCreateModal] = useState(false);
  const [submitWorkBountyId, setSubmitWorkBountyId] = useState<string | null>(null);
  const [commentBountyId, setCommentBountyId] = useState<string | null>(null);

  const fetchBounties = useCallback(() => {
    setState({ status: 'loading' });
    void apiClient.bounties
      .list()
      .then((res: BountyListResponse) => {
        setState({ status: 'ok', bounties: res.bounties ?? [] });
      })
      .catch((err: unknown) => {
        setState({ status: 'error', message: String(err) });
      });
  }, []);

  useEffect(() => {
    fetchBounties();
  }, [fetchBounties]);

  // ── Cancel ─────────────────────────────────────────────────────────────────

  async function handleCancel(bountyId: string) {
    setMutating(true);
    try {
      await apiClient.bounties.cancel(bountyId);
      fetchBounties();
    } catch {
      // error is surfaced inline
    } finally {
      setMutating(false);
    }
  }

  // ── Run Council ────────────────────────────────────────────────────────────

  async function handleRunCouncil(bountyId: string) {
    setMutating(true);
    try {
      await apiClient.bounties.runCouncil(bountyId);
      fetchBounties();
    } catch {
      // error is surfaced inline
    } finally {
      setMutating(false);
    }
  }

  // ── Render ─────────────────────────────────────────────────────────────────

  let body: React.ReactNode;
  if (state.status === 'loading') {
    body = (
      <div className="flex h-64 items-center justify-center text-content-faint">
        <span className="animate-pulse text-sm">Loading bounties...</span>
      </div>
    );
  } else if (state.status === 'error') {
    body = (
      <StatusBlock
        tone="text-red-600 dark:text-red-400"
        title="Failed to load bounties"
        body={state.message}
      />
    );
  } else if (state.bounties.length === 0) {
    body = (
      <StatusBlock
        tone="text-content-muted"
        title="No bounties found"
        body="No bounties have been posted yet. Create one to get started."
      />
    );
  } else {
    body = (
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        {state.bounties.map(bounty => (
          <BountyRow
            key={bounty.bountyId}
            bounty={bounty}
            expanded={expandedBountyId === bounty.bountyId}
            onToggle={() =>
              setExpandedBountyId(prev => (prev === bounty.bountyId ? null : bounty.bountyId))
            }
            myAgentId={myAgentId}
            onSubmit={id => setSubmitWorkBountyId(id)}
            onComment={id => setCommentBountyId(id)}
            onCancel={id => {
              void handleCancel(id);
            }}
            onRunCouncil={id => {
              void handleRunCouncil(id);
            }}
            mutating={mutating}
          />
        ))}
      </div>
    );
  }

  return (
    <PanelScaffold description="Bounties">
      {/* Create Bounty button (wallet-gated) */}
      {myAgentId && (
        <div className="mb-4 flex justify-end">
          <Button onClick={() => setShowCreateModal(true)}>Create Bounty</Button>
        </div>
      )}

      {body}

      {/* Modals */}
      {showCreateModal && (
        <CreateBountyModal
          onClose={() => setShowCreateModal(false)}
          onCreated={bounty => {
            setShowCreateModal(false);
            // Show success toast with a "View" action that expands the new row.
            // The expand state is pre-set here; once the list refetch completes
            // the row auto-expands. No i18n yet — the entire BountiesSection
            // uses hardcoded English strings (TODO: i18n when section is
            // internationalised).
            addToast({
              type: 'success',
              title: 'Bounty created',
              message: (bounty as Bounty).title,
              action: {
                label: 'View',
                handler: () => setExpandedBountyId((bounty as Bounty).bountyId),
              },
            });
            fetchBounties();
            // In SDK 0.10 create-and-fund is atomic, so a new bounty is already
            // funded (`open`) — no separate fund step.
          }}
        />
      )}

      {submitWorkBountyId && (
        <SubmitWorkModal
          bountyId={submitWorkBountyId}
          onClose={() => setSubmitWorkBountyId(null)}
          onSubmitted={() => {
            setSubmitWorkBountyId(null);
            fetchBounties();
          }}
        />
      )}

      {commentBountyId && (
        <CommentModal
          bountyId={commentBountyId}
          onClose={() => setCommentBountyId(null)}
          onCommented={() => {
            setCommentBountyId(null);
            fetchBounties();
          }}
        />
      )}

      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </PanelScaffold>
  );
}
