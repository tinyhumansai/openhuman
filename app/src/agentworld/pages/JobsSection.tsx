/**
 * JobsSection — Agent World "Jobs" section.
 *
 * Renders the public jobs board via
 * `apiClient.graphql.jobs()` (GraphQL, no auth required).
 * Supports inline row expansion to show full job details including
 * client profile (avatar + display name), budget, skills chips,
 * dispute info, and on-chain data.
 *
 * Write surface (Phase 6): Post a Job, Apply, Cancel, View Proposals,
 * Shortlist, Select, Withdraw, Open/Adjudicate Dispute.
 * All write actions are wallet-gated behind useMyAgentId().
 *
 * Pattern mirrors LedgerSection / FeedSection: useState + useEffect fetch,
 * PanelScaffold wrapper, StatusBlock for loading/error/empty states.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

import PanelScaffold from '../../components/layout/PanelScaffold';
import Button from '../../components/ui/Button';
import { ModalShell } from '../../components/ui/ModalShell';
import {
  type GqlJobPosting,
  type JobCreateParams,
  type Proposal,
  type ProposalCreateParams,
} from '../../lib/agentworld/invokeApiClient';
import { useT } from '../../lib/i18n/I18nContext';
import { fetchWalletStatus } from '../../services/walletApi';
import { apiClient } from '../AgentWorldShell';
import { explorerTxUrl } from '../hooks/useX402Buy';

// ── State types ───────────────────────────────────────────────────────────────

type JobsState =
  | { status: 'loading' }
  | { status: 'error'; message: string }
  | { status: 'ok'; jobs: GqlJobPosting[] };

// ── Helpers ───────────────────────────────────────────────────────────────────

// TODO: extract shared relativeTime helper once Feed/Ledger/Jobs all use it.
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

/**
 * Group the integer part of a numeric amount with thousands separators while
 * preserving the original decimals ("1000000" → "1,000,000", "0.50" → "0.50").
 * Non-numeric strings pass through unchanged.
 */
function formatAmount(amount: string): string {
  if (!Number.isFinite(Number(amount))) return amount;
  const negative = amount.startsWith('-');
  const body = negative ? amount.slice(1) : amount;
  const [intPart, fracPart] = body.split('.');
  const grouped = Number(intPart).toLocaleString('en-US');
  const out = fracPart != null ? `${grouped}.${fracPart}` : grouped;
  return negative ? `-${out}` : out;
}

/** Collapse a raw base58 address-like display name to `abcd…wxyz`; leave real names. */
function displayClientName(name: string): string {
  if (name.length > 16 && !/\s/.test(name)) {
    return `${name.slice(0, 4)}…${name.slice(-4)}`;
  }
  return name;
}

/** Verified check badge (replaces the bare ✓ glyph). */
function VerifiedBadge() {
  return (
    <svg
      className="h-3.5 w-3.5 shrink-0 text-primary-500"
      viewBox="0 0 20 20"
      fill="currentColor"
      aria-label="Verified">
      <path
        fillRule="evenodd"
        d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z"
        clipRule="evenodd"
      />
    </svg>
  );
}

/** Centered status message for loading / error / info states. */
function StatusBlock({ tone, title, body }: { tone: string; title: string; body?: string }) {
  return (
    <div className="flex h-64 flex-col items-center justify-center gap-2 text-center">
      <p className={`text-base font-medium ${tone}`}>{title}</p>
      {body && <p className="max-w-md text-sm text-stone-500 dark:text-neutral-400">{body}</p>}
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

// ── JobStatusBadge ─────────────────────────────────────────────────────────────
// Job statuses (OPEN/IN_PROGRESS/COMPLETED/DISPUTED/CANCELLED) have different
// semantics and colors from ledger statuses — defined locally, not imported.

export function JobStatusBadge({ status }: { status: string }) {
  const color =
    status === 'OPEN'
      ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400'
      : status === 'IN_PROGRESS'
        ? 'bg-primary-100 text-primary-700 dark:bg-primary-900/30 dark:text-primary-400'
        : status === 'COMPLETED'
          ? 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400'
          : status === 'DISPUTED'
            ? 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400'
            : status === 'CANCELLED'
              ? 'bg-stone-100 text-stone-600 dark:bg-neutral-800 dark:text-neutral-400'
              : 'bg-stone-100 text-stone-600 dark:bg-neutral-800 dark:text-neutral-400';
  return (
    <span className={`inline-flex rounded-full px-2 py-0.5 text-xs font-medium ${color}`}>
      {status}
    </span>
  );
}

// ── SkillChip ─────────────────────────────────────────────────────────────────

function SkillChip({ skill }: { skill: string }) {
  return (
    <span className="inline-flex rounded-full bg-primary-50 px-2 py-0.5 text-xs text-primary-700 dark:bg-primary-900/20 dark:text-primary-400">
      {skill}
    </span>
  );
}

// ── ClientAvatar ──────────────────────────────────────────────────────────────

function ClientAvatar({ avatarUrl, displayName }: { avatarUrl?: string; displayName: string }) {
  const initials = displayName
    .split(' ')
    .map(w => w[0] ?? '')
    .slice(0, 2)
    .join('')
    .toUpperCase();

  if (avatarUrl) {
    return (
      <img
        src={avatarUrl}
        alt={displayName}
        className="h-7 w-7 shrink-0 rounded-full object-cover"
        onError={e => {
          // Swap to initials circle on load failure
          const target = e.currentTarget as HTMLImageElement;
          target.style.display = 'none';
          if (target.nextElementSibling) {
            (target.nextElementSibling as HTMLElement).style.display = 'flex';
          }
        }}
      />
    );
  }

  return (
    <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-primary-100 text-xs font-medium text-primary-700 dark:bg-primary-900/30 dark:text-primary-400">
      {initials || '?'}
    </div>
  );
}

// ── PostJobModal ──────────────────────────────────────────────────────────────

function PostJobModal({ onClose, onCreated }: { onClose: () => void; onCreated: () => void }) {
  const { t } = useT();
  const [title, setTitle] = useState('');
  const [description, setDescription] = useState('');
  const [category, setCategory] = useState('');
  const [skillsCsv, setSkillsCsv] = useState('');
  const [budgetAmount, setBudgetAmount] = useState('');
  const [budgetAsset, setBudgetAsset] = useState('USDC');
  const [proposalDeadline, setProposalDeadline] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!title.trim() || !budgetAmount.trim()) return;
    setSubmitting(true);
    setError(null);
    // <input type="date"> yields "YYYY-MM-DD" but the server requires RFC3339.
    // Pin to end-of-day UTC, consistent with BountiesSection.
    const deadlineDate = proposalDeadline.trim();
    const deadlineIso = deadlineDate ? `${deadlineDate}T23:59:59Z` : undefined;
    // Reject today and past dates by comparing the raw date string against the
    // local calendar date — end-of-day UTC would otherwise let "today" pass
    // until midnight UTC.
    if (deadlineDate) {
      const now = new Date();
      const todayLocal = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}-${String(now.getDate()).padStart(2, '0')}`;
      if (deadlineDate <= todayLocal) {
        setError(t('agentWorld.jobs.deadlineFuture', 'Proposal deadline must be in the future'));
        setSubmitting(false);
        return;
      }
    }
    const params: JobCreateParams = {
      title: title.trim(),
      description: description.trim() || undefined,
      category: category.trim() || undefined,
      skills: skillsCsv.trim()
        ? skillsCsv
            .split(',')
            .map(s => s.trim())
            .filter(Boolean)
        : undefined,
      budgetAmount: budgetAmount.trim(),
      budgetAsset: budgetAsset.trim() || 'USDC',
      proposalDeadline: deadlineIso,
    };
    try {
      await apiClient.jobsWrite.create(params);
      onCreated();
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <ModalShell
      onClose={onClose}
      title="Post a Job"
      titleId="post-job-modal-title"
      maxWidthClassName="max-w-lg">
      <form
        onSubmit={e => {
          void handleSubmit(e);
        }}
        className="space-y-3">
        <div>
          <label className="mb-1 block text-xs font-medium text-stone-700 dark:text-neutral-300">
            Title *
          </label>
          <input
            type="text"
            required
            value={title}
            onChange={e => setTitle(e.target.value)}
            className="w-full rounded border border-stone-300 bg-white px-2.5 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
            placeholder="e.g. Build a Solana integration"
          />
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium text-stone-700 dark:text-neutral-300">
            Description
          </label>
          <textarea
            rows={3}
            value={description}
            onChange={e => setDescription(e.target.value)}
            className="w-full rounded border border-stone-300 bg-white px-2.5 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
            placeholder="Describe the work, requirements, and deliverables"
          />
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium text-stone-700 dark:text-neutral-300">
            Category
          </label>
          <input
            type="text"
            value={category}
            onChange={e => setCategory(e.target.value)}
            className="w-full rounded border border-stone-300 bg-white px-2.5 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
            placeholder="e.g. development, design, research"
          />
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium text-stone-700 dark:text-neutral-300">
            Skills
          </label>
          <input
            type="text"
            value={skillsCsv}
            onChange={e => setSkillsCsv(e.target.value)}
            className="w-full rounded border border-stone-300 bg-white px-2.5 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
            placeholder="e.g. React, TypeScript"
          />
        </div>
        <div className="flex gap-2">
          <div className="flex-1">
            <label className="mb-1 block text-xs font-medium text-stone-700 dark:text-neutral-300">
              Budget Amount *
            </label>
            <input
              type="text"
              required
              value={budgetAmount}
              onChange={e => setBudgetAmount(e.target.value)}
              className="w-full rounded border border-stone-300 bg-white px-2.5 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
              placeholder="500"
            />
          </div>
          <div className="w-28">
            <label className="mb-1 block text-xs font-medium text-stone-700 dark:text-neutral-300">
              Asset
            </label>
            <input
              type="text"
              value={budgetAsset}
              onChange={e => setBudgetAsset(e.target.value)}
              className="w-full rounded border border-stone-300 bg-white px-2.5 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
              placeholder="USDC"
            />
          </div>
        </div>
        <div>
          <label className="mb-1 block text-xs font-medium text-stone-700 dark:text-neutral-300">
            Proposal Deadline
          </label>
          <input
            type="date"
            value={proposalDeadline}
            onChange={e => setProposalDeadline(e.target.value)}
            className="w-full rounded border border-stone-300 bg-white px-2.5 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
          />
        </div>
        {error && <p className="text-xs text-red-600 dark:text-red-400">{error}</p>}
        <div className="flex justify-end gap-2 pt-1">
          <Button type="button" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={submitting}>
            {submitting ? 'Posting…' : 'Post Job'}
          </Button>
        </div>
      </form>
    </ModalShell>
  );
}

// ── ApplyModal ────────────────────────────────────────────────────────────────

function ApplyModal({
  jobId,
  onClose,
  onApplied,
}: {
  jobId: string;
  onClose: () => void;
  onApplied: () => void;
}) {
  const { t } = useT();
  const [coverLetter, setCoverLetter] = useState('');
  const [bidAmount, setBidAmount] = useState('');
  const [estimatedDelivery, setEstimatedDelivery] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [succeeded, setSucceeded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const successTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Clear the auto-close timer on unmount to avoid state updates after teardown.
  useEffect(() => {
    return () => {
      if (successTimerRef.current) {
        clearTimeout(successTimerRef.current);
      }
    };
  }, []);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setSubmitting(true);
    setError(null);
    const params: ProposalCreateParams = {
      coverLetter: coverLetter.trim() || undefined,
      bidAmount: bidAmount.trim() || undefined,
      estimatedDelivery: estimatedDelivery.trim() || undefined,
    };
    try {
      await apiClient.jobsWrite.apply(jobId, params);
      setSucceeded(true);
      onApplied();
      // Auto-close after a short success display window.
      successTimerRef.current = setTimeout(() => {
        onClose();
      }, 1500);
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <ModalShell
      onClose={onClose}
      title={t('agentworld.jobs.applyModal.title')}
      titleId="apply-modal-title"
      maxWidthClassName="max-w-lg">
      {succeeded ? (
        <div
          role="status"
          aria-live="polite"
          className="flex flex-col items-center gap-3 py-6 text-center">
          <p className="text-sm font-medium text-green-700 dark:text-green-400">
            {t('agentworld.jobs.applyModal.successHeading')}
          </p>
          <p className="text-xs text-stone-500 dark:text-neutral-400">
            {t('agentworld.jobs.applyModal.successBody')}
          </p>
        </div>
      ) : (
        <form
          onSubmit={e => {
            void handleSubmit(e);
          }}
          className="space-y-3">
          <div>
            <label className="mb-1 block text-xs font-medium text-stone-700 dark:text-neutral-300">
              {t('agentworld.jobs.applyModal.coverLetterLabel')}
            </label>
            <textarea
              rows={4}
              value={coverLetter}
              onChange={e => setCoverLetter(e.target.value)}
              className="w-full rounded border border-stone-300 bg-white px-2.5 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
              placeholder={t('agentworld.jobs.applyModal.coverLetterPlaceholder')}
            />
          </div>
          <div>
            <label className="mb-1 block text-xs font-medium text-stone-700 dark:text-neutral-300">
              {t('agentworld.jobs.applyModal.bidAmountLabel')}
            </label>
            <input
              type="text"
              value={bidAmount}
              onChange={e => setBidAmount(e.target.value)}
              className="w-full rounded border border-stone-300 bg-white px-2.5 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
              placeholder={t('agentworld.jobs.applyModal.bidAmountPlaceholder')}
            />
          </div>
          <div>
            <label className="mb-1 block text-xs font-medium text-stone-700 dark:text-neutral-300">
              {t('agentworld.jobs.applyModal.deliveryLabel')}
            </label>
            <input
              type="text"
              value={estimatedDelivery}
              onChange={e => setEstimatedDelivery(e.target.value)}
              className="w-full rounded border border-stone-300 bg-white px-2.5 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
              placeholder={t('agentworld.jobs.applyModal.deliveryPlaceholder')}
            />
          </div>
          {error && (
            <p role="alert" className="text-xs text-red-600 dark:text-red-400">
              {error}
            </p>
          )}
          <div className="flex justify-end gap-2 pt-1">
            <Button type="button" onClick={onClose} disabled={submitting}>
              {t('agentworld.jobs.applyModal.cancel')}
            </Button>
            <Button type="submit" disabled={submitting}>
              {submitting
                ? t('agentworld.jobs.applyModal.submitting')
                : t('agentworld.jobs.applyModal.submit')}
            </Button>
          </div>
        </form>
      )}
    </ModalShell>
  );
}

// ── DisputeModal ──────────────────────────────────────────────────────────────

function DisputeModal({
  jobId,
  onClose,
  onDisputed,
}: {
  jobId: string;
  onClose: () => void;
  onDisputed: () => void;
}) {
  const [reason, setReason] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!reason.trim()) return;
    setSubmitting(true);
    setError(null);
    try {
      await apiClient.jobsWrite.openDispute(jobId, reason.trim());
      onDisputed();
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <ModalShell
      onClose={onClose}
      title="Open Dispute"
      titleId="dispute-modal-title"
      maxWidthClassName="max-w-md">
      <form
        onSubmit={e => {
          void handleSubmit(e);
        }}
        className="space-y-3">
        <div>
          <label className="mb-1 block text-xs font-medium text-stone-700 dark:text-neutral-300">
            Reason *
          </label>
          <textarea
            rows={4}
            required
            value={reason}
            onChange={e => setReason(e.target.value)}
            className="w-full rounded border border-stone-300 bg-white px-2.5 py-1.5 text-sm text-stone-900 dark:border-neutral-700 dark:bg-neutral-800 dark:text-neutral-100"
            placeholder="Describe the issue that requires dispute resolution"
          />
        </div>
        {error && <p className="text-xs text-red-600 dark:text-red-400">{error}</p>}
        <div className="flex justify-end gap-2 pt-1">
          <Button type="button" onClick={onClose}>
            Cancel
          </Button>
          <Button type="submit" disabled={submitting}>
            {submitting ? 'Opening…' : 'Open Dispute'}
          </Button>
        </div>
      </form>
    </ModalShell>
  );
}

// ── JobRow ────────────────────────────────────────────────────────────────────

function JobRow({
  job,
  expanded,
  onToggle,
  myAgentId,
  onApply,
  onCancel,
  onViewProposals,
  onOpenDispute,
  onAdjudicate,
  proposalsForJobId,
  proposals,
  proposalsLoading,
  onShortlist,
  onSelect,
  onWithdraw,
  mutating,
}: {
  job: GqlJobPosting;
  expanded: boolean;
  onToggle: () => void;
  myAgentId: string | null;
  onApply: (jobId: string) => void;
  onCancel: (jobId: string) => void;
  onViewProposals: (jobId: string) => void;
  onOpenDispute: (jobId: string) => void;
  onAdjudicate: (jobId: string) => void;
  proposalsForJobId: string | null;
  proposals: Proposal[];
  proposalsLoading: boolean;
  onShortlist: (jobId: string, proposalId: string) => void;
  onSelect: (jobId: string, proposalId: string) => void;
  onWithdraw: (jobId: string, proposalId: string) => void;
  mutating: boolean;
}) {
  const budgetLabel = `${formatAmount(job.budget.amount)} ${job.budget.asset}`;
  const skills = job.skills ?? [];
  const visibleSkills = skills.slice(0, 3);
  const overflowCount = skills.length - visibleSkills.length;

  const isClient = myAgentId === job.client;
  const showingProposals = proposalsForJobId === job.jobId;
  const proposalLabel = `${job.proposalCount} proposal${job.proposalCount !== 1 ? 's' : ''}`;

  return (
    <div className="border-b border-stone-100 last:border-0 dark:border-neutral-800">
      {/* Summary row — avatar · stacked content · fixed meta column */}
      <button
        type="button"
        onClick={onToggle}
        className="flex w-full items-start gap-3 px-4 py-3 text-left transition-colors hover:bg-stone-50 dark:hover:bg-neutral-800/50">
        <ClientAvatar
          avatarUrl={job.clientProfile.avatarUrl ?? undefined}
          displayName={job.clientProfile.displayName}
        />

        {/* Content */}
        <div className="min-w-0 flex-1">
          {/* Line 1: title + status */}
          <div className="flex items-center gap-2">
            <span className="truncate text-sm font-semibold text-stone-900 dark:text-neutral-100">
              {job.title}
            </span>
            <span className="shrink-0">
              <JobStatusBadge status={job.status} />
            </span>
          </div>

          {/* Line 2: client · budget */}
          <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-xs text-stone-500 dark:text-neutral-400">
            <span className="truncate">{displayClientName(job.clientProfile.displayName)}</span>
            {job.clientProfile.verified && <VerifiedBadge />}
            <span className="text-stone-300 dark:text-neutral-600">·</span>
            <span className="whitespace-nowrap font-medium text-stone-700 dark:text-neutral-300">
              {budgetLabel}
            </span>
          </div>

          {/* Line 3: skills + proposal count */}
          <div className="mt-1.5 flex flex-wrap items-center gap-1.5">
            {visibleSkills.map(skill => (
              <SkillChip key={skill} skill={skill} />
            ))}
            {overflowCount > 0 && (
              <span className="text-xs text-stone-400 dark:text-neutral-500">+{overflowCount}</span>
            )}
            <span className="text-xs text-stone-400 dark:text-neutral-500">
              {skills.length > 0 ? '· ' : ''}
              {proposalLabel}
            </span>
          </div>
        </div>

        {/* Fixed meta column: time + chevron */}
        <div className="flex shrink-0 flex-col items-end gap-2">
          <span className="whitespace-nowrap text-xs text-stone-400 dark:text-neutral-500">
            {relativeTime(job.createdAt)}
          </span>
          <svg
            className={`h-4 w-4 shrink-0 text-stone-400 transition-transform dark:text-neutral-500 ${expanded ? 'rotate-180' : ''}`}
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
          </svg>
        </div>
      </button>

      {/* Expanded detail */}
      {expanded && (
        <div className="border-t border-stone-100 bg-stone-50 px-4 py-3 dark:border-neutral-800 dark:bg-neutral-900/50">
          {/* Description */}
          <p className="mb-3 whitespace-pre-wrap text-sm text-stone-700 dark:text-neutral-300">
            {job.description}
          </p>

          <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
            {/* Job ID */}
            <dt className="font-medium text-stone-500 dark:text-neutral-400">Job ID</dt>
            <dd className="break-all font-mono text-stone-800 dark:text-neutral-200">
              {job.jobId}
            </dd>

            {/* Category */}
            {job.category && (
              <>
                <dt className="font-medium text-stone-500 dark:text-neutral-400">Category</dt>
                <dd className="text-stone-800 dark:text-neutral-200">{job.category}</dd>
              </>
            )}

            {/* All skills */}
            {skills.length > 0 && (
              <>
                <dt className="font-medium text-stone-500 dark:text-neutral-400">Skills</dt>
                <dd className="flex flex-wrap gap-1">
                  {skills.map(skill => (
                    <SkillChip key={skill} skill={skill} />
                  ))}
                </dd>
              </>
            )}

            {/* Budget chain */}
            {job.budget.chain && (
              <>
                <dt className="font-medium text-stone-500 dark:text-neutral-400">Chain</dt>
                <dd className="text-stone-800 dark:text-neutral-200">{job.budget.chain}</dd>
              </>
            )}

            {/* Proposal deadline */}
            {job.proposalDeadline && (
              <>
                <dt className="font-medium text-stone-500 dark:text-neutral-400">
                  Proposal Deadline
                </dt>
                <dd className="text-stone-800 dark:text-neutral-200">{job.proposalDeadline}</dd>
              </>
            )}

            {/* Contract escrow ID */}
            {job.contractEscrowId && (
              <>
                <dt className="font-medium text-stone-500 dark:text-neutral-400">Escrow ID</dt>
                <dd className="break-all font-mono text-stone-800 dark:text-neutral-200">
                  {job.contractEscrowId}
                </dd>
              </>
            )}

            {/* Selected candidate */}
            {job.selectedCandidate && (
              <>
                <dt className="font-medium text-stone-500 dark:text-neutral-400">
                  Selected Candidate
                </dt>
                <dd className="break-all font-mono text-stone-800 dark:text-neutral-200">
                  {job.selectedCandidate}
                </dd>
              </>
            )}

            {/* Group ID */}
            {job.groupId && (
              <>
                <dt className="font-medium text-stone-500 dark:text-neutral-400">Group ID</dt>
                <dd className="break-all font-mono text-stone-800 dark:text-neutral-200">
                  {job.groupId}
                </dd>
              </>
            )}

            {/* Timestamps */}
            <dt className="font-medium text-stone-500 dark:text-neutral-400">Created</dt>
            <dd className="text-stone-800 dark:text-neutral-200">{job.createdAt}</dd>

            <dt className="font-medium text-stone-500 dark:text-neutral-400">Updated</dt>
            <dd className="text-stone-800 dark:text-neutral-200">{job.updatedAt}</dd>
          </dl>

          {/* Dispute section */}
          {job.dispute && (
            <div className="mt-3">
              <p className="mb-1 text-xs font-semibold text-red-600 dark:text-red-400">Dispute</p>
              <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
                <dt className="font-medium text-stone-500 dark:text-neutral-400">Reason</dt>
                <dd className="text-stone-800 dark:text-neutral-200">{job.dispute.reason}</dd>

                <dt className="font-medium text-stone-500 dark:text-neutral-400">Opened By</dt>
                <dd className="break-all font-mono text-stone-800 dark:text-neutral-200">
                  {job.dispute.openedBy}
                </dd>

                <dt className="font-medium text-stone-500 dark:text-neutral-400">Opened At</dt>
                <dd className="text-stone-800 dark:text-neutral-200">{job.dispute.openedAt}</dd>

                <dt className="font-medium text-stone-500 dark:text-neutral-400">Status</dt>
                <dd className="text-stone-800 dark:text-neutral-200">{job.dispute.status}</dd>

                {job.dispute.outcome && (
                  <>
                    <dt className="font-medium text-stone-500 dark:text-neutral-400">Outcome</dt>
                    <dd className="text-stone-800 dark:text-neutral-200">{job.dispute.outcome}</dd>
                  </>
                )}

                {job.dispute.splitBps != null && (
                  <>
                    <dt className="font-medium text-stone-500 dark:text-neutral-400">Split bps</dt>
                    <dd className="text-stone-800 dark:text-neutral-200">{job.dispute.splitBps}</dd>
                  </>
                )}

                {job.dispute.judgeModel && (
                  <>
                    <dt className="font-medium text-stone-500 dark:text-neutral-400">
                      Judge Model
                    </dt>
                    <dd className="text-stone-800 dark:text-neutral-200">
                      {job.dispute.judgeModel}
                    </dd>
                  </>
                )}

                {job.dispute.presided != null && (
                  <>
                    <dt className="font-medium text-stone-500 dark:text-neutral-400">Presided</dt>
                    <dd className="text-stone-800 dark:text-neutral-200">
                      {job.dispute.presided ? 'Yes' : 'No'}
                    </dd>
                  </>
                )}

                {job.dispute.reasoning && (
                  <>
                    <dt className="font-medium text-stone-500 dark:text-neutral-400">Reasoning</dt>
                    <dd className="text-stone-800 dark:text-neutral-200">
                      {job.dispute.reasoning}
                    </dd>
                  </>
                )}

                {job.dispute.resolvedAt && (
                  <>
                    <dt className="font-medium text-stone-500 dark:text-neutral-400">
                      Resolved At
                    </dt>
                    <dd className="text-stone-800 dark:text-neutral-200">
                      {job.dispute.resolvedAt}
                    </dd>
                  </>
                )}
              </dl>

              {/* Jury votes table */}
              {job.dispute.jury && job.dispute.jury.length > 0 && (
                <div className="mt-2">
                  <p className="mb-1 text-xs font-medium text-stone-500 dark:text-neutral-400">
                    Jury Votes
                  </p>
                  <div className="overflow-x-auto">
                    <table className="w-full text-xs">
                      <thead>
                        <tr className="border-b border-stone-200 dark:border-neutral-700">
                          <th className="pb-1 text-left font-medium text-stone-500 dark:text-neutral-400">
                            Model
                          </th>
                          <th className="pb-1 text-left font-medium text-stone-500 dark:text-neutral-400">
                            Outcome
                          </th>
                          <th className="pb-1 text-left font-medium text-stone-500 dark:text-neutral-400">
                            Split bps
                          </th>
                          <th className="pb-1 text-left font-medium text-stone-500 dark:text-neutral-400">
                            Reasoning
                          </th>
                        </tr>
                      </thead>
                      <tbody>
                        {job.dispute.jury.map((vote, i) => (
                          <tr
                            key={i}
                            className="border-b border-stone-100 last:border-0 dark:border-neutral-800">
                            <td className="py-0.5 font-mono text-stone-800 dark:text-neutral-200">
                              {vote.model}
                            </td>
                            <td className="py-0.5 text-stone-800 dark:text-neutral-200">
                              {vote.outcome}
                            </td>
                            <td className="py-0.5 text-stone-800 dark:text-neutral-200">
                              {vote.splitBps}
                            </td>
                            <td className="py-0.5 text-stone-800 dark:text-neutral-200">
                              {vote.reasoning ?? '-'}
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </div>
              )}
            </div>
          )}

          {/* On-chain section */}
          {job.onChain && (
            <div className="mt-3">
              <p className="mb-1 text-xs font-semibold text-stone-500 dark:text-neutral-400">
                On-chain
              </p>
              <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-xs">
                {job.onChain.vault && (
                  <>
                    <dt className="font-medium text-stone-500 dark:text-neutral-400">Vault</dt>
                    <dd className="break-all font-mono text-stone-800 dark:text-neutral-200">
                      {job.onChain.vault}
                    </dd>
                  </>
                )}

                {job.onChain.jobPdaCommit && (
                  <>
                    <dt className="font-medium text-stone-500 dark:text-neutral-400">
                      Job PDA Commit
                    </dt>
                    <dd className="break-all font-mono text-stone-800 dark:text-neutral-200">
                      {job.onChain.jobPdaCommit}
                    </dd>
                  </>
                )}

                {job.onChain.fundingTxSig && (
                  <>
                    <dt className="font-medium text-stone-500 dark:text-neutral-400">Funding Tx</dt>
                    <dd className="break-all font-mono text-stone-800 dark:text-neutral-200">
                      <a
                        href={explorerTxUrl(job.onChain.fundingTxSig, 'solana-devnet')}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="text-primary-600 hover:text-primary-700 dark:text-primary-400 dark:hover:text-primary-300">
                        {job.onChain.fundingTxSig}
                      </a>
                    </dd>
                  </>
                )}
              </dl>
            </div>
          )}

          {/* Write actions (wallet-gated) */}
          {myAgentId ? (
            <div className="mt-4 flex flex-wrap gap-2">
              {/* Candidate actions: Apply (non-client, OPEN jobs) */}
              {!isClient && job.status === 'OPEN' && (
                <Button type="button" onClick={() => onApply(job.jobId)} disabled={mutating}>
                  Apply
                </Button>
              )}

              {/* Client actions */}
              {isClient && (
                <>
                  {job.status === 'OPEN' && (
                    <Button type="button" onClick={() => onCancel(job.jobId)} disabled={mutating}>
                      Cancel Job
                    </Button>
                  )}
                  {(job.status === 'OPEN' || job.status === 'IN_PROGRESS') && (
                    <Button
                      type="button"
                      onClick={() => onViewProposals(job.jobId)}
                      disabled={mutating}>
                      View Proposals
                    </Button>
                  )}
                  {job.status === 'IN_PROGRESS' && !job.dispute && (
                    <Button
                      type="button"
                      onClick={() => onOpenDispute(job.jobId)}
                      disabled={mutating}>
                      Open Dispute
                    </Button>
                  )}
                  {job.status === 'DISPUTED' && (
                    <Button
                      type="button"
                      onClick={() => onAdjudicate(job.jobId)}
                      disabled={mutating}>
                      Adjudicate
                    </Button>
                  )}
                </>
              )}
            </div>
          ) : (
            <p className="mt-4 text-xs text-stone-400 dark:text-neutral-500">
              Unlock your wallet to interact with this job.
            </p>
          )}

          {/* Inline proposals panel */}
          {showingProposals && (
            <div className="mt-4">
              <p className="mb-2 text-xs font-semibold text-stone-600 dark:text-neutral-300">
                Proposals
              </p>
              {proposalsLoading ? (
                <p className="text-xs text-stone-400 dark:text-neutral-500 animate-pulse">
                  Loading proposals…
                </p>
              ) : proposals.length === 0 ? (
                <p className="text-xs text-stone-400 dark:text-neutral-500">No proposals yet.</p>
              ) : (
                <div className="space-y-2">
                  {proposals.map(p => (
                    <div
                      key={p.proposalId}
                      className="rounded border border-stone-200 bg-white p-2 text-xs dark:border-neutral-700 dark:bg-neutral-800">
                      <div className="mb-1 flex items-center gap-2">
                        <span className="font-mono text-stone-600 dark:text-neutral-400">
                          {p.candidate.slice(0, 8)}…
                        </span>
                        <span className="text-stone-500 dark:text-neutral-500">{p.status}</span>
                        {p.bidAmount && (
                          <span className="font-medium text-stone-800 dark:text-neutral-200">
                            {p.bidAmount}
                          </span>
                        )}
                      </div>
                      {p.coverLetter && (
                        <p className="mb-1 text-stone-700 dark:text-neutral-300 line-clamp-2">
                          {p.coverLetter}
                        </p>
                      )}
                      <div className="flex gap-1">
                        <Button
                          type="button"
                          onClick={() => onShortlist(job.jobId, p.proposalId)}
                          disabled={mutating}>
                          Shortlist
                        </Button>
                        <Button
                          type="button"
                          onClick={() => onSelect(job.jobId, p.proposalId)}
                          disabled={mutating}>
                          Select
                        </Button>
                        <Button
                          type="button"
                          onClick={() => onWithdraw(job.jobId, p.proposalId)}
                          disabled={mutating}>
                          Withdraw
                        </Button>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ── JobsSection (main export) ─────────────────────────────────────────────────

export default function JobsSection() {
  const [jobsState, setJobsState] = useState<JobsState>({ status: 'loading' });
  const [expandedJobId, setExpandedJobId] = useState<string | null>(null);
  const [showPostJob, setShowPostJob] = useState(false);
  const [applyingJobId, setApplyingJobId] = useState<string | null>(null);
  const [disputeJobId, setDisputeJobId] = useState<string | null>(null);
  const [proposalsForJobId, setProposalsForJobId] = useState<string | null>(null);
  const [proposals, setProposals] = useState<Proposal[]>([]);
  const [proposalsLoading, setProposalsLoading] = useState(false);
  const [mutating, setMutating] = useState(false);

  const myAgentId = useMyAgentId();

  // ── Fetch jobs ─────────────────────────────────────────────────────────────
  const refetchJobs = useCallback(() => {
    setJobsState({ status: 'loading' });
    void apiClient.graphql
      .jobs({ limit: 50 })
      .then(result => {
        const jobs = Array.isArray(result?.jobs) ? result.jobs : [];
        setJobsState({ status: 'ok', jobs });
      })
      .catch((err: unknown) => {
        setJobsState({ status: 'error', message: String(err) });
      });
  }, []);

  useEffect(() => {
    let cancelled = false;
    setJobsState({ status: 'loading' });

    void apiClient.graphql
      .jobs({ limit: 50 })
      .then(result => {
        if (cancelled) return;
        const jobs = Array.isArray(result?.jobs) ? result.jobs : [];
        setJobsState({ status: 'ok', jobs });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setJobsState({ status: 'error', message: String(err) });
      });

    return () => {
      cancelled = true;
    };
  }, []);

  // ── Handlers ───────────────────────────────────────────────────────────────

  const handleCancel = useCallback(
    async (jobId: string) => {
      setMutating(true);
      try {
        await apiClient.jobsWrite.cancel(jobId);
        refetchJobs();
      } catch (err) {
        console.error('[JobsSection] cancel failed:', err);
      } finally {
        setMutating(false);
      }
    },
    [refetchJobs]
  );

  const handleViewProposals = useCallback(async (jobId: string) => {
    setProposalsForJobId(jobId);
    setProposalsLoading(true);
    try {
      const result = await apiClient.jobsWrite.listProposals(jobId);
      setProposals(Array.isArray(result?.proposals) ? result.proposals : []);
    } catch (err) {
      console.error('[JobsSection] listProposals failed:', err);
      setProposals([]);
    } finally {
      setProposalsLoading(false);
    }
  }, []);

  const handleShortlist = useCallback(
    async (jobId: string, proposalId: string) => {
      setMutating(true);
      try {
        await apiClient.jobsWrite.shortlistProposal(jobId, proposalId);
        await handleViewProposals(jobId);
      } catch (err) {
        console.error('[JobsSection] shortlist failed:', err);
      } finally {
        setMutating(false);
      }
    },
    [handleViewProposals]
  );

  const handleSelect = useCallback(
    async (jobId: string, proposalId: string) => {
      if (!window.confirm('Select this candidate and initiate escrow?')) return;
      setMutating(true);
      try {
        const result = await apiClient.jobsWrite.select(jobId, proposalId);
        console.debug('[JobsSection] selected candidate, escrow:', result.contractEscrowId);
        refetchJobs();
      } catch (err) {
        console.error('[JobsSection] select failed:', err);
      } finally {
        setMutating(false);
      }
    },
    [refetchJobs]
  );

  const handleWithdraw = useCallback(
    async (jobId: string, proposalId: string) => {
      setMutating(true);
      try {
        await apiClient.jobsWrite.withdrawProposal(jobId, proposalId);
        if (proposalsForJobId === jobId) {
          await handleViewProposals(jobId);
        }
      } catch (err) {
        console.error('[JobsSection] withdraw failed:', err);
      } finally {
        setMutating(false);
      }
    },
    [proposalsForJobId, handleViewProposals]
  );

  const handleAdjudicate = useCallback(
    async (jobId: string) => {
      setMutating(true);
      try {
        await apiClient.jobsWrite.adjudicateDispute(jobId);
        refetchJobs();
      } catch (err) {
        console.error('[JobsSection] adjudicate failed:', err);
      } finally {
        setMutating(false);
      }
    },
    [refetchJobs]
  );

  // ── Render ─────────────────────────────────────────────────────────────────

  let body: React.ReactNode;

  if (jobsState.status === 'loading') {
    body = (
      <div className="flex h-64 items-center justify-center text-stone-400 dark:text-neutral-500">
        <span className="animate-pulse text-sm">Loading jobs...</span>
      </div>
    );
  } else if (jobsState.status === 'error') {
    body = (
      <StatusBlock
        tone="text-red-600 dark:text-red-400"
        title="Failed to load jobs"
        body={jobsState.message}
      />
    );
  } else if (jobsState.jobs.length === 0) {
    body = (
      <StatusBlock
        tone="text-stone-500 dark:text-neutral-400"
        title="No jobs found"
        body="The jobs board is empty or no postings match the current filter."
      />
    );
  } else {
    body = (
      <div className="rounded-lg border border-stone-200 bg-white dark:border-neutral-800 dark:bg-neutral-900">
        {jobsState.jobs.map(job => (
          <JobRow
            key={job.jobId}
            job={job}
            expanded={expandedJobId === job.jobId}
            onToggle={() => setExpandedJobId(prev => (prev === job.jobId ? null : job.jobId))}
            myAgentId={myAgentId}
            onApply={jobId => setApplyingJobId(jobId)}
            onCancel={jobId => {
              void handleCancel(jobId);
            }}
            onViewProposals={jobId => {
              void handleViewProposals(jobId);
            }}
            onOpenDispute={jobId => setDisputeJobId(jobId)}
            onAdjudicate={jobId => {
              void handleAdjudicate(jobId);
            }}
            proposalsForJobId={proposalsForJobId}
            proposals={proposals}
            proposalsLoading={proposalsLoading}
            onShortlist={(jobId, proposalId) => {
              void handleShortlist(jobId, proposalId);
            }}
            onSelect={(jobId, proposalId) => {
              void handleSelect(jobId, proposalId);
            }}
            onWithdraw={(jobId, proposalId) => {
              void handleWithdraw(jobId, proposalId);
            }}
            mutating={mutating}
          />
        ))}
      </div>
    );
  }

  return (
    <PanelScaffold description="Jobs">
      {/* Post a Job button (wallet-gated) */}
      {myAgentId && (
        <div className="mb-4 flex justify-end">
          <Button onClick={() => setShowPostJob(true)}>Post a Job</Button>
        </div>
      )}

      {body}

      {/* Modals */}
      {showPostJob && (
        <PostJobModal onClose={() => setShowPostJob(false)} onCreated={refetchJobs} />
      )}
      {applyingJobId && (
        <ApplyModal
          jobId={applyingJobId}
          onClose={() => setApplyingJobId(null)}
          onApplied={() => {
            refetchJobs();
          }}
        />
      )}
      {disputeJobId && (
        <DisputeModal
          jobId={disputeJobId}
          onClose={() => setDisputeJobId(null)}
          onDisputed={refetchJobs}
        />
      )}
    </PanelScaffold>
  );
}
