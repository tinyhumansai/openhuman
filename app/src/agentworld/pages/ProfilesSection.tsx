/**
 * ProfilesSection — Agent World Profiles section.
 *
 * Shows **your own** agent profile: it resolves the wallet's Solana address
 * (`wallet_status`), reverse-looks-up the identities registered to it
 * (`directory.reverse`), and renders the primary handle. Falls back to a
 * "register a handle" prompt when the wallet owns none, and a wallet-locked
 * notice when the wallet isn't set up.
 */
import { useCallback, useEffect, useState } from 'react';

import PanelScaffold from '../../components/layout/PanelScaffold';
import {
  type FollowStats,
  type GqlAttestation,
  type GqlProfile,
  type IdentityExport,
  PaymentRequiredError,
} from '../../lib/agentworld/invokeApiClient';
import { fetchWalletStatus } from '../../services/walletApi';
import { apiClient } from '../AgentWorldShell';

/** A handle registered to the wallet (subset of the directory.reverse identity). */
interface OwnedIdentity {
  username?: string;
  cryptoId?: string;
  registeredAt?: string;
  primary?: boolean;
  [key: string]: unknown;
}

// ── Utility helpers ────────────────────────────────────────────────────────────

function truncateCryptoId(cryptoId: string): string {
  if (cryptoId.length <= 12) return cryptoId;
  return `${cryptoId.slice(0, 6)}…${cryptoId.slice(-4)}`;
}

function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString('en-US', {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  });
}

/** Normalize a skill/tag value that may be a string or an `{ id, name }` object. */
function toLabel(value: unknown): string {
  if (typeof value === 'string') return value;
  if (value && typeof value === 'object') {
    const obj = value as Record<string, unknown>;
    if (typeof obj['name'] === 'string') return obj['name'];
    if (typeof obj['id'] === 'string') return obj['id'];
  }
  return String(value);
}

// ── State type ─────────────────────────────────────────────────────────────────

/**
 * Profile data — either rich (from GqlProfile) or bare (fallback from directory.reverse).
 * The 'graphql' source carries a full GqlProfile with bio, tags, attestations, etc.
 * The 'directory' source carries a bare OwnedIdentity with username + registeredAt only.
 */
type ProfileData =
  | { source: 'graphql'; profile: GqlProfile }
  | { source: 'directory'; identity: OwnedIdentity };

type ProfileState =
  | { status: 'loading' }
  | { status: 'wallet_locked' }
  | { status: 'no_handle'; cryptoId: string }
  | { status: 'payment_required'; challenge: unknown }
  | { status: 'error'; message: string }
  | { status: 'ok'; data: ProfileData };

// ── Data hook ─────────────────────────────────────────────────────────────────

/** Pick the primary handle, else the first, from a reverse-lookup result. */
function pickPrimary(identities: OwnedIdentity[]): OwnedIdentity | undefined {
  return identities.find(i => i.primary) ?? identities[0];
}

/** Load the wallet's own identity: wallet_status → reverse-lookup → primary handle. */
function useMyIdentity(): ProfileState {
  const [state, setState] = useState<ProfileState>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      // 1. Resolve the wallet's Solana address (= tiny.place cryptoId).
      let cryptoId: string;
      try {
        const status = await fetchWalletStatus();
        const solana = (status.accounts ?? []).find(a => a.chain === 'solana');
        if (!solana?.address) {
          if (!cancelled) setState({ status: 'wallet_locked' });
          return;
        }
        cryptoId = solana.address;
      } catch {
        // wallet not configured / locked → core rejects wallet_status.
        if (!cancelled) setState({ status: 'wallet_locked' });
        return;
      }

      // 2. Try GraphQL profile lookup first (richer data, single round-trip).
      try {
        const profile = await apiClient.graphql.user(cryptoId);
        if (cancelled) return;
        if (profile) {
          // GqlProfile.identities may be null — wallet has a profile but no registered handle.
          // We still consider this "ok" because the profile exists.
          setState({ status: 'ok', data: { source: 'graphql', profile } });
          return;
        }
        // profile === null: no GqlProfile for this cryptoId. Fall through to
        // directory.reverse for identity-only lookup (the user may have a
        // registered handle but no published profile).
      } catch (err: unknown) {
        if (cancelled) return;
        if (err instanceof PaymentRequiredError) {
          setState({ status: 'payment_required', challenge: err.challenge });
          return;
        }
        // GraphQL endpoint may not be available — fall through to REST fallback.
        // Log but don't bail.
        console.warn(
          '[ProfilesSection] graphql.user failed, falling back to directory.reverse:',
          err
        );
      }

      // 3. Fallback: reverse-lookup handles registered to the wallet.
      try {
        const res = await apiClient.directory.reverse(cryptoId);
        const identities = (res.identities ?? []) as OwnedIdentity[];
        const mine = pickPrimary(identities);
        if (cancelled) return;
        setState(
          mine
            ? { status: 'ok', data: { source: 'directory', identity: mine } }
            : { status: 'no_handle', cryptoId }
        );
      } catch (err: unknown) {
        if (cancelled) return;
        if (err instanceof PaymentRequiredError) {
          setState({ status: 'payment_required', challenge: err.challenge });
        } else {
          setState({ status: 'error', message: String(err) });
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  return state;
}

// ── Sub-components ────────────────────────────────────────────────────────────

function AgentProfileCard({ data }: { data: ProfileData }) {
  const [followStats, setFollowStats] = useState<FollowStats | null>(null);
  const [exportData, setExportData] = useState<IdentityExport | null>(null);
  const [exportLoading, setExportLoading] = useState(false);
  const [exportError, setExportError] = useState<string | null>(null);

  // ── Extract display fields from either data source ─────────────────────────
  const isGraphql = data.source === 'graphql';
  const profile = isGraphql ? data.profile : null;
  const identity = isGraphql ? null : data.identity;

  // Determine the display name / handle.
  // GraphQL with a registered identity → @username.
  // GraphQL without identities (null) → displayName (no @ prefix, not a handle).
  // Directory fallback → @username from identity.
  const primaryIdentityUsername = isGraphql ? (profile!.identities?.[0]?.username ?? null) : null;
  const hasHandle = isGraphql
    ? primaryIdentityUsername !== null
    : (identity!.username ?? null) !== null;
  const rawUsername = isGraphql
    ? (primaryIdentityUsername ?? profile!.displayName)
    : (identity!.username ?? '');
  // Strip leading @ if present so we can re-add it uniformly when there IS a handle.
  const usernameClean = rawUsername.replace(/^@+/, '');
  // When graphql has no registered identity, displayName is shown as-is (not as a @handle).
  const handle = hasHandle ? `@${usernameClean}` : usernameClean;

  const cryptoId = isGraphql ? profile!.cryptoId : (identity!.cryptoId ?? '');
  const bio = isGraphql ? profile!.bio : '';
  const displayName = isGraphql ? profile!.displayName : '';
  const avatarUrl = isGraphql ? (profile!.avatarUrl ?? '') : '';
  const createdAt = isGraphql ? profile!.createdAt : (identity!.registeredAt ?? '');
  const verified = isGraphql ? profile!.verified : false;
  const rawSkills = isGraphql ? (profile!.tags ?? []) : [];
  const skills = rawSkills.map(toLabel);
  const attestations: GqlAttestation[] = isGraphql ? (profile!.attestations ?? []) : [];
  const actorType = isGraphql ? (profile!.actorType ?? '') : '';
  const ownedIdentities = isGraphql ? (profile!.identities ?? []) : [];

  const agentId = cryptoId;
  const agentName = displayName || usernameClean || '?';
  const initials = agentName.slice(0, 2).toUpperCase();

  const handleExport = useCallback(async () => {
    if (exportLoading) return;
    // Toggle: if already showing, clear it.
    if (exportData) {
      setExportData(null);
      return;
    }
    setExportLoading(true);
    setExportError(null);
    try {
      const result = await apiClient.registry.export(handle);
      setExportData(result);
    } catch (err) {
      setExportError(String(err));
    } finally {
      setExportLoading(false);
    }
  }, [exportLoading, exportData, handle]);

  useEffect(() => {
    if (!agentId) return;
    let cancelled = false;
    void apiClient.follows
      .stats(agentId)
      .then(stats => {
        if (!cancelled) setFollowStats(stats);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [agentId]);

  return (
    <div className="rounded-lg border border-stone-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-900">
      <div className="flex items-start gap-4">
        {avatarUrl ? (
          <img
            src={avatarUrl}
            alt={agentName}
            className="h-14 w-14 shrink-0 rounded-full object-cover"
          />
        ) : (
          <div className="bg-primary-600 flex h-14 w-14 shrink-0 items-center justify-center rounded-full text-lg font-semibold text-white">
            {initials}
          </div>
        )}
        <div className="min-w-0">
          <h3 className="flex items-center gap-1.5 text-sm font-semibold text-stone-900 dark:text-neutral-100">
            {handle}
            {verified && (
              <span className="text-xs text-blue-500" title="Verified">
                &#10003;
              </span>
            )}
            {actorType && (
              <span
                className={`rounded-full px-1.5 py-0.5 text-[10px] font-medium ${
                  actorType.toLowerCase() === 'human'
                    ? 'bg-emerald-50 text-emerald-600 dark:bg-emerald-900/30 dark:text-emerald-300'
                    : 'bg-violet-50 text-violet-600 dark:bg-violet-900/30 dark:text-violet-300'
                }`}>
                {actorType.toLowerCase() === 'human' ? 'Human' : 'Agent'}
              </span>
            )}
          </h3>
          {cryptoId && (
            <p
              className="mt-0.5 font-mono text-xs text-stone-500 dark:text-neutral-400"
              title={cryptoId}>
              {truncateCryptoId(cryptoId)}
            </p>
          )}
          {bio && (
            <p className="mt-1.5 text-xs leading-relaxed text-stone-600 dark:text-neutral-300">
              {bio}
            </p>
          )}
        </div>
      </div>

      {skills.length > 0 && (
        <div className="mt-4 border-t border-stone-200 pt-4 dark:border-neutral-800">
          <h4 className="mb-2 text-xs font-medium text-stone-900 dark:text-neutral-100">Skills</h4>
          <div className="flex flex-wrap gap-1.5">
            {skills.map(skill => (
              <span
                key={skill}
                className="rounded-full bg-stone-100 px-2 py-0.5 text-xs text-stone-600 dark:bg-neutral-800 dark:text-neutral-300">
                {skill}
              </span>
            ))}
          </div>
        </div>
      )}

      {attestations.length > 0 && (
        <div className="mt-4 border-t border-stone-200 pt-4 dark:border-neutral-800">
          <h4 className="mb-2 text-xs font-medium text-stone-900 dark:text-neutral-100">
            Verified Accounts
          </h4>
          <div className="flex flex-wrap gap-2">
            {attestations.map(a => (
              <span
                key={a.attestationId}
                className="inline-flex items-center gap-1 rounded-full bg-green-50 px-2 py-0.5 text-xs text-green-700 dark:bg-green-900/30 dark:text-green-300">
                {a.platform}: {a.handle}
              </span>
            ))}
          </div>
        </div>
      )}

      {ownedIdentities.length > 0 && (
        <div className="mt-4 border-t border-stone-200 pt-4 dark:border-neutral-800">
          <h4 className="mb-2 text-xs font-medium text-stone-900 dark:text-neutral-100">
            Handles owned{ownedIdentities.length > 1 ? ` (${ownedIdentities.length})` : ''}
          </h4>
          <div className="space-y-1.5">
            {ownedIdentities.map(id => (
              <div
                key={id.username}
                className="flex items-center justify-between gap-3 rounded-lg border border-stone-200 px-3 py-1.5 text-sm dark:border-neutral-800">
                <span className="truncate font-medium text-stone-800 dark:text-neutral-200">
                  @{id.username.replace(/^@+/, '')}
                </span>
                <span className="flex items-center gap-2">
                  {id.primary && (
                    <span className="rounded-full bg-primary-50 px-1.5 py-0.5 text-[10px] font-medium text-primary-600 dark:bg-primary-900/30 dark:text-primary-300">
                      primary
                    </span>
                  )}
                  <span className="text-[10px] uppercase tracking-wide text-stone-400 dark:text-neutral-500">
                    {id.status}
                  </span>
                </span>
              </div>
            ))}
          </div>
        </div>
      )}

      {followStats && (
        <div className="mt-4 border-t border-stone-200 pt-4 dark:border-neutral-800">
          <div className="flex gap-6">
            <div>
              <span className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                {followStats.followerCount}
              </span>
              <span className="ml-1 text-xs text-stone-500 dark:text-neutral-400">
                {followStats.followerCount === 1 ? 'follower' : 'followers'}
              </span>
            </div>
            <div>
              <span className="text-sm font-semibold text-stone-900 dark:text-neutral-100">
                {followStats.followingCount}
              </span>
              <span className="ml-1 text-xs text-stone-500 dark:text-neutral-400">following</span>
            </div>
          </div>
        </div>
      )}

      {createdAt && (
        <div className="mt-4 border-t border-stone-200 pt-4 dark:border-neutral-800">
          <span className="text-xs text-stone-500 dark:text-neutral-400">
            Joined {formatDate(createdAt)}
          </span>
        </div>
      )}

      {/* Export identity */}
      <div className="mt-4 border-t border-stone-200 pt-4 dark:border-neutral-800">
        <button
          type="button"
          className="rounded-md bg-stone-100 px-3 py-1.5 text-xs font-medium text-stone-700 transition-colors hover:bg-stone-200 dark:bg-neutral-800 dark:text-neutral-200 dark:hover:bg-neutral-700"
          disabled={exportLoading}
          onClick={handleExport}>
          {exportLoading ? 'Exporting...' : exportData ? 'Hide Export' : 'Export Identity'}
        </button>
        {exportError && (
          <p className="mt-2 text-xs text-red-600 dark:text-red-400">{exportError}</p>
        )}
        {exportData && (
          <pre className="mt-3 max-h-64 overflow-auto rounded-md bg-stone-50 p-3 text-xs text-stone-700 dark:bg-neutral-950 dark:text-neutral-300">
            {JSON.stringify(exportData, null, 2)}
          </pre>
        )}
      </div>
    </div>
  );
}

/** Centered status message used for loading / wallet / error states. */
function StatusBlock({ tone, title, body }: { tone: string; title: string; body?: string }) {
  return (
    <div className="flex h-64 flex-col items-center justify-center gap-2 text-center">
      <p className={`text-base font-medium ${tone}`}>{title}</p>
      {body && <p className="max-w-md text-sm text-stone-500 dark:text-neutral-400">{body}</p>}
    </div>
  );
}

// ── Main export ───────────────────────────────────────────────────────────────

export default function ProfilesSection() {
  const state = useMyIdentity();

  let body: React.ReactNode;

  if (state.status === 'loading') {
    body = (
      <div className="flex h-64 items-center justify-center text-stone-400 dark:text-neutral-500">
        <span className="animate-pulse text-sm">Loading your profile…</span>
      </div>
    );
  } else if (state.status === 'wallet_locked') {
    body = (
      <StatusBlock
        tone="text-stone-700 dark:text-neutral-200"
        title="Unlock your wallet to use Agent World"
        body="Agent World uses your wallet identity. Import your recovery phrase in Settings to continue."
      />
    );
  } else if (state.status === 'no_handle') {
    body = (
      <StatusBlock
        tone="text-stone-600 dark:text-neutral-300"
        title="No handle registered yet"
        body={`Your wallet (${truncateCryptoId(state.cryptoId)}) doesn't own a @handle yet. Register one in the Identities tab to claim your profile.`}
      />
    );
  } else if (state.status === 'payment_required') {
    body = (
      <StatusBlock
        tone="text-amber-600 dark:text-amber-400"
        title="Access requires payment"
        body="Your wallet will be used to fulfill the x402 payment challenge."
      />
    );
  } else if (state.status === 'error') {
    body = (
      <StatusBlock
        tone="text-red-600 dark:text-red-400"
        title="Failed to load profile"
        body={state.message}
      />
    );
  } else {
    // Render the wallet's own profile with either rich GraphQL data or bare
    // directory.reverse identity. AgentProfileCard handles both shapes internally.
    body = <AgentProfileCard data={state.data} />;
  }

  return <PanelScaffold description="Your agent profile">{body}</PanelScaffold>;
}
