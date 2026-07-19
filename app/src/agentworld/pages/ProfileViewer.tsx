/**
 * ProfileViewer — Agent World public profile viewer for an ARBITRARY handle.
 *
 * Route: `/agent-world/profiles/:username`. Where `ProfilesSection` is hard-wired
 * to the connected wallet, this viewer renders any user's or agent's profile via
 * the already-plumbed read handlers: `graphql.profile(username)` (a rich
 * `GqlProfile`) and `graphql.agentCard(cryptoId)` (the agent card + the
 * signer-aware `viewerIsFollowing` flag), plus `follows.stats`. It adds a
 * follow / unfollow button (reusing `follows.follow` / `follows.unfollow`) and a
 * copy-link affordance so a profile is shareable.
 *
 * Read-only. Profile EDITING lives in `ProfilesSection` and is tracked
 * separately (#4930); this component never mutates the viewed profile.
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';
import { useParams } from 'react-router-dom';

import PanelScaffold from '../../components/layout/PanelScaffold';
import Button from '../../components/ui/Button';
import {
  type AgentCard,
  type FollowStats,
  type GqlAttestation,
  type GqlProfile,
  type Identity,
  PaymentRequiredError,
} from '../../lib/agentworld/invokeApiClient';
import { useT } from '../../lib/i18n/I18nContext';
import { fetchWalletStatus } from '../../services/walletApi';
import { apiClient } from '../AgentWorldShell';

const log = debug('agentworld:profileviewer');

// ── Helpers ─────────────────────────────────────────────────────────────────────

/** Sanitized error label for diagnostics — an error NAME/kind only, never the
 *  raw message (which may carry backend internals / PII). */
function errorKind(err: unknown): string {
  if (err instanceof PaymentRequiredError) return 'payment_required';
  if (err instanceof Error) return err.name || 'Error';
  return 'unknown';
}

function truncateCryptoId(cryptoId: string): string {
  if (cryptoId.length <= 12) return cryptoId;
  return `${cryptoId.slice(0, 6)}…${cryptoId.slice(-4)}`;
}

/** Format an ISO date using the runtime locale (never a hard-coded language). */
function formatDate(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

/** Pick the primary identity, else the first. */
function pickPrimary<T extends { primary?: boolean }>(identities: T[]): T | undefined {
  return identities.find(i => i.primary) ?? identities[0];
}

/** Read the signer-aware `viewerIsFollowing` flag off an agent card, if present. */
function readViewerFollows(card: AgentCard | null): boolean | null {
  const value = card?.['viewerIsFollowing'];
  return typeof value === 'boolean' ? value : null;
}

/** Resolve the wallet's Solana address (the viewer's tiny.place cryptoId). */
function useMyAgentId(): string | null {
  const [agentId, setAgentId] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    log('[agentworld:profileviewer] resolving wallet agent id');
    void fetchWalletStatus()
      .then(status => {
        if (cancelled) return;
        const solana = (status.accounts ?? []).find(a => a.chain === 'solana');
        if (solana?.address) setAgentId(solana.address);
        else log('[agentworld:profileviewer] no solana wallet account; viewer is signed out');
      })
      .catch((err: unknown) => {
        if (!cancelled) log('[agentworld:profileviewer] wallet resolve failed: %s', errorKind(err));
      });
    return () => {
      cancelled = true;
    };
  }, []);
  return agentId;
}

// ── Profile fetch ────────────────────────────────────────────────────────────────

type ViewerState =
  | { status: 'loading' }
  | { status: 'not_found' }
  | { status: 'error' }
  | { status: 'ok'; profile: GqlProfile };

/**
 * Load the target handle's public profile, keyed by the normalized handle so a
 * stale response for a previous handle can never surface the wrong profile. The
 * effect never calls setState synchronously — the only state writes happen in
 * the async resolve/reject, satisfying the repo's no-sync-setState-in-effect
 * rule. `null` from the server → not found.
 */
function useProfile(handle: string): ViewerState {
  const [entry, setEntry] = useState<{ key: string; state: ViewerState }>({
    key: '',
    state: { status: 'loading' },
  });

  useEffect(() => {
    if (!handle) return;
    let cancelled = false;
    log('[agentworld:profileviewer] loading profile');
    void apiClient.graphql
      .profile(handle)
      .then(profile => {
        if (cancelled) return;
        log('[agentworld:profileviewer] profile %s', profile ? 'loaded' : 'not found');
        setEntry({
          key: handle,
          state: profile ? { status: 'ok', profile } : { status: 'not_found' },
        });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        log('[agentworld:profileviewer] profile load failed: %s', errorKind(err));
        setEntry({ key: handle, state: { status: 'error' } });
      });
    return () => {
      cancelled = true;
    };
  }, [handle]);

  if (!handle) return { status: 'not_found' };
  // Until THIS handle's response lands, show loading — prevents a previous
  // handle's profile (and its follow/share actions) from flashing.
  return entry.key === handle ? entry.state : { status: 'loading' };
}

// ── Agent card (also carries the signer-aware follow flag) ───────────────────────

/** Fetch the agent card for `cryptoId`. Returns `null` for non-agent profiles or
 *  on failure — callers degrade to "unknown". */
function useAgentCard(cryptoId: string): AgentCard | null {
  const [card, setCard] = useState<AgentCard | null>(null);
  useEffect(() => {
    if (!cryptoId) return;
    let cancelled = false;
    log('[agentworld:profileviewer] loading agent card');
    void apiClient.graphql
      .agentCard(cryptoId)
      .then(c => {
        if (!cancelled) setCard(c);
      })
      .catch((err: unknown) => {
        if (!cancelled)
          log('[agentworld:profileviewer] agent card load failed: %s', errorKind(err));
      });
    return () => {
      cancelled = true;
    };
  }, [cryptoId]);
  return card;
}

// ── Follow control ─────────────────────────────────────────────────────────────

type FollowState = 'unknown' | 'following' | 'not_following';

/**
 * Follow relationship for `targetCryptoId`, derived ONLY from the direct,
 * signer-aware `viewerFollowsHint` (from the agent card). It never infers a
 * relationship from partial or failed data: when the hint is absent it stays
 * `'unknown'` and the button is hidden, rather than defaulting to "not
 * following" and risking a follow() on an existing relationship. A user toggle
 * takes optimistic precedence via `override`.
 */
function useFollow(
  myAgentId: string | null,
  targetCryptoId: string,
  viewerFollowsHint: boolean | null,
  onFollowChange?: (next: 'following' | 'not_following') => void
) {
  const [override, setOverride] = useState<'following' | 'not_following' | null>(null);
  const [busy, setBusy] = useState(false);

  const isSelf = myAgentId != null && myAgentId === targetCryptoId;
  const enabled = myAgentId != null && targetCryptoId !== '' && !isSelf;
  const state: FollowState =
    override ??
    (viewerFollowsHint == null ? 'unknown' : viewerFollowsHint ? 'following' : 'not_following');

  const toggle = useCallback(async () => {
    if (busy || !enabled || state === 'unknown') return;
    setBusy(true);
    const next = state === 'following' ? 'not_following' : 'following';
    try {
      if (state === 'following') await apiClient.follows.unfollow(targetCryptoId);
      else await apiClient.follows.follow(targetCryptoId);
      setOverride(next);
      // Keep the follower count in step with the button (it is fetched once and
      // would otherwise lag behind after a follow/unfollow).
      onFollowChange?.(next);
      // No PII: only the resulting action, never the address/handle.
      log('[agentworld:profileviewer] follow toggled -> %s', next);
    } catch (err) {
      log('[agentworld:profileviewer] follow toggle failed: %s', errorKind(err));
    } finally {
      setBusy(false);
    }
  }, [busy, enabled, state, targetCryptoId, onFollowChange]);

  return { state, busy, isSelf, enabled, toggle };
}

// ── Follow stats sub-hook ────────────────────────────────────────────────────────

function useFollowStats(cryptoId: string): FollowStats | null {
  const [stats, setStats] = useState<FollowStats | null>(null);
  useEffect(() => {
    if (!cryptoId) return;
    let cancelled = false;
    void apiClient.follows
      .stats(cryptoId)
      .then(s => {
        if (!cancelled) setStats(s);
      })
      .catch((err: unknown) => {
        if (!cancelled)
          log('[agentworld:profileviewer] follow stats load failed: %s', errorKind(err));
      });
    return () => {
      cancelled = true;
    };
  }, [cryptoId]);
  return stats;
}

// ── Presentational bits ──────────────────────────────────────────────────────────

function StatusBlock({ tone, title, body }: { tone: string; title: string; body?: string }) {
  return (
    <div className="flex h-64 flex-col items-center justify-center gap-2 text-center">
      <p className={`text-base font-medium ${tone}`}>{title}</p>
      {body && <p className="max-w-md text-sm text-content-muted">{body}</p>}
    </div>
  );
}

// ── Main export ──────────────────────────────────────────────────────────────────

export default function ProfileViewer() {
  const { username } = useParams<{ username: string }>();
  const { t } = useT();
  const routeHandle = (username ?? '').replace(/^@+/, '').trim();
  const state = useProfile(routeHandle);

  let body: React.ReactNode;
  if (state.status === 'loading') {
    body = (
      <div className="flex h-64 items-center justify-center text-content-faint">
        <span className="animate-pulse text-sm">{t('agentWorld.profileViewer.loading')}</span>
      </div>
    );
  } else if (state.status === 'not_found') {
    body = (
      <StatusBlock
        tone="text-content-secondary"
        title={t('agentWorld.profileViewer.notFoundTitle')}
        body={t('agentWorld.profileViewer.notFoundBody')}
      />
    );
  } else if (state.status === 'error') {
    // Generic, translated copy only — never the raw external error string.
    body = (
      <StatusBlock
        tone="text-red-600 dark:text-red-400"
        title={t('agentWorld.profileViewer.errorTitle')}
      />
    );
  } else {
    body = <ProfileCard profile={state.profile} routeHandle={routeHandle} />;
  }

  return (
    <PanelScaffold description={t('agentWorld.profileViewer.description')}>{body}</PanelScaffold>
  );
}

// ── Profile card ─────────────────────────────────────────────────────────────────

function ProfileCard({ profile, routeHandle }: { profile: GqlProfile; routeHandle: string }) {
  const { t } = useT();
  const myAgentId = useMyAgentId();
  const cryptoId = profile.cryptoId;
  const agentCard = useAgentCard(cryptoId);
  // Optimistic follower-count adjustment so the count tracks the follow button
  // (stats are fetched once and would otherwise lag after a toggle).
  const [followerDelta, setFollowerDelta] = useState(0);
  const follow = useFollow(myAgentId, cryptoId, readViewerFollows(agentCard), next =>
    setFollowerDelta(d => d + (next === 'following' ? 1 : -1))
  );
  const followStats = useFollowStats(cryptoId);
  const [copied, setCopied] = useState(false);
  // Fall back to the initials monogram if the avatar image fails to load.
  const [avatarBroken, setAvatarBroken] = useState(false);

  const primaryIdentity = pickPrimary(profile.identities ?? []);
  const primaryUsername = primaryIdentity?.username ?? null;
  const hasHandle = primaryUsername !== null;
  const usernameClean = (primaryUsername ?? profile.displayName ?? '').replace(/^@+/, '');
  const handle = hasHandle ? `@${usernameClean}` : usernameClean;
  const displayName = profile.displayName || usernameClean || '?';
  const initials = displayName.slice(0, 2).toUpperCase();
  const skills = profile.tags ?? [];
  const attestations: GqlAttestation[] = profile.attestations ?? [];
  const ownedIdentities: Identity[] = profile.identities ?? [];
  const actorType = profile.actorType ?? '';
  const isHuman = actorType.toLowerCase() === 'human';
  const showFollow = follow.enabled && follow.state !== 'unknown';
  // Read-only agent-card summary (only when it adds something over the profile).
  const cardDescription = (agentCard?.description ?? '').trim();

  const copyLink = useCallback(() => {
    // Build the deep link from the ROUTE handle (what the user actually
    // navigated to), not from `identities`/`displayName` — those may be null or
    // differ from the selected route, producing a link that does not resolve.
    const { origin, pathname } = window.location;
    const link = `${origin}${pathname}#/agent-world/profiles/${encodeURIComponent(routeHandle)}`;
    void navigator.clipboard
      ?.writeText(link)
      .then(() => {
        setCopied(true);
        log('[agentworld:profileviewer] copied share link');
        window.setTimeout(() => setCopied(false), 2000);
      })
      .catch((err: unknown) => {
        log('[agentworld:profileviewer] copy link failed: %s', errorKind(err));
      });
  }, [routeHandle]);

  return (
    <div className="rounded-lg border border-line bg-surface p-4">
      {/* Header row: identity + actions */}
      <div className="flex items-start gap-4">
        {profile.avatarUrl && !avatarBroken ? (
          <img
            src={profile.avatarUrl}
            alt={displayName}
            onError={() => setAvatarBroken(true)}
            className="h-14 w-14 shrink-0 rounded-full object-cover"
          />
        ) : (
          <div className="bg-primary-600 flex h-14 w-14 shrink-0 items-center justify-center rounded-full text-lg font-semibold text-content-inverted">
            {initials}
          </div>
        )}
        <div className="min-w-0 flex-1">
          <h3 className="flex items-center gap-1.5 text-sm font-semibold text-content">
            <span className="truncate">{handle}</span>
            {profile.verified && (
              <span className="text-xs text-blue-500" title="Verified">
                &#10003;
              </span>
            )}
            {actorType && (
              <span
                className={`rounded-full px-1.5 py-0.5 text-[10px] font-medium ${
                  isHuman
                    ? 'bg-emerald-50 text-emerald-600 dark:bg-emerald-900/30 dark:text-emerald-300'
                    : 'bg-violet-50 text-violet-600 dark:bg-violet-900/30 dark:text-violet-300'
                }`}>
                {isHuman
                  ? t('agentWorld.profileViewer.humanBadge')
                  : t('agentWorld.profileViewer.agentBadge')}
              </span>
            )}
          </h3>
          {cryptoId && (
            <p className="mt-0.5 font-mono text-xs text-content-muted" title={cryptoId}>
              {truncateCryptoId(cryptoId)}
            </p>
          )}
          {profile.bio && (
            <p className="mt-1.5 text-xs leading-relaxed text-content-secondary">{profile.bio}</p>
          )}
        </div>

        {/* Actions */}
        <div className="flex shrink-0 flex-col items-end gap-1.5">
          {follow.isSelf ? (
            <span className="text-[11px] text-content-faint">
              {t('agentWorld.profileViewer.ownProfile')}
            </span>
          ) : showFollow ? (
            <Button
              variant={follow.state === 'following' ? 'secondary' : 'primary'}
              size="sm"
              disabled={follow.busy}
              onClick={() => void follow.toggle()}>
              {follow.state === 'following'
                ? t('agentWorld.profileViewer.following')
                : t('agentWorld.profileViewer.follow')}
            </Button>
          ) : null}
          <Button variant="tertiary" size="sm" onClick={copyLink} data-testid="profile-copy-link">
            {copied
              ? t('agentWorld.profileViewer.linkCopied')
              : t('agentWorld.profileViewer.copyLink')}
          </Button>
        </div>
      </div>

      {cardDescription && cardDescription !== (profile.bio ?? '').trim() && (
        <div className="mt-4 border-t border-line pt-4">
          <h4 className="mb-2 text-xs font-medium text-content">
            {t('agentWorld.profileViewer.agentCard')}
          </h4>
          <p className="text-xs leading-relaxed text-content-secondary">{cardDescription}</p>
        </div>
      )}

      {skills.length > 0 && (
        <div className="mt-4 border-t border-line pt-4">
          <h4 className="mb-2 text-xs font-medium text-content">
            {t('agentWorld.profileViewer.skills')}
          </h4>
          <div className="flex flex-wrap gap-1.5">
            {skills.map(skill => (
              <span
                key={skill}
                className="rounded-full bg-surface-subtle px-2 py-0.5 text-xs text-content-secondary">
                {skill}
              </span>
            ))}
          </div>
        </div>
      )}

      {attestations.length > 0 && (
        <div className="mt-4 border-t border-line pt-4">
          <h4 className="mb-2 text-xs font-medium text-content">
            {t('agentWorld.profileViewer.verifiedAccounts')}
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
        <div className="mt-4 border-t border-line pt-4">
          <h4 className="mb-2 text-xs font-medium text-content">
            {t('agentWorld.profileViewer.handlesOwned')}
          </h4>
          <div className="flex flex-wrap gap-1.5">
            {ownedIdentities.map(id => (
              <span
                key={id.username}
                className="rounded-full border border-line px-2 py-0.5 text-xs text-content-secondary">
                @{id.username.replace(/^@+/, '')}
              </span>
            ))}
          </div>
        </div>
      )}

      {followStats && (
        <div className="mt-4 border-t border-line pt-4">
          <div className="flex gap-6">
            <div>
              <span className="text-sm font-semibold text-content">
                {Math.max(0, followStats.followerCount + followerDelta)}
              </span>
              <span className="ml-1 text-xs text-content-muted">
                {t('agentWorld.profileViewer.followers')}
              </span>
            </div>
            <div>
              <span className="text-sm font-semibold text-content">
                {followStats.followingCount}
              </span>
              <span className="ml-1 text-xs text-content-muted">
                {t('agentWorld.profileViewer.followingCount')}
              </span>
            </div>
          </div>
        </div>
      )}

      {profile.createdAt && (
        <div className="mt-4 border-t border-line pt-4">
          <span className="text-xs text-content-muted">
            {t('agentWorld.profileViewer.joined')} {formatDate(profile.createdAt)}
          </span>
        </div>
      )}
    </div>
  );
}
