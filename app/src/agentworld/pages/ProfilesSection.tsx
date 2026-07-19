/**
 * ProfilesSection — Agent World Profiles section.
 *
 * Shows **your own** agent profile: it resolves the wallet's Solana address
 * (`wallet_status`), reverse-looks-up the identities registered to it
 * (`directory.reverse`), and renders the primary handle. Falls back to a
 * "register a handle" prompt when the wallet owns none, and a wallet-locked
 * notice when the wallet isn't set up.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import PanelScaffold from '../../components/layout/PanelScaffold';
import Button from '../../components/ui/Button';
import {
  type FollowStats,
  type GqlAttestation,
  type GqlProfile,
  type IdentityExport,
  PaymentRequiredError,
} from '../../lib/agentworld/invokeApiClient';
import { useT } from '../../lib/i18n/I18nContext';
import { fetchWalletStatus } from '../../services/walletApi';
import { apiClient } from '../AgentWorldShell';

const log = debug('agentworld:profile');

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

/** Pick the primary handle, else the first, from a reverse-lookup result.
 *  Generic over the identity shape so it accepts both the bare directory
 *  `OwnedIdentity` and the richer GraphQL `Identity` (it only reads `primary`). */
function pickPrimary<T extends { primary?: boolean }>(identities: T[]): T | undefined {
  return identities.find(i => i.primary) ?? identities[0];
}

/** Load the wallet's own identity: wallet_status → reverse-lookup → primary handle.
 *  `reloadKey` is bumped by the active-handle switch so the profile refetches and
 *  the newly-promoted primary is reflected (#4198). */
function useMyIdentity(reloadKey: number): ProfileState {
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
  }, [reloadKey]);

  return state;
}

// ── Sub-components ────────────────────────────────────────────────────────────

/**
 * Own-profile edit form (name / bio / avatar), wired to the already-plumbed
 * `users.updateProfile` write path (#4930). Prefills the writable fields from
 * the authoritative `users.get` record — including `avatarEmail`, which the
 * display-side `GqlProfile` does not carry (the shown avatar URL is derived
 * server-side from the Gravatar email). On success the parent refetches so the
 * saved values render.
 */
function ProfileEditForm({
  cryptoId,
  initialDisplayName,
  initialBio,
  onCancel,
  onSaved,
}: {
  cryptoId: string;
  initialDisplayName: string;
  initialBio: string;
  onCancel: () => void;
  onSaved: () => void;
}) {
  const { t } = useT();
  const [displayName, setDisplayName] = useState(initialDisplayName);
  const [bio, setBio] = useState(initialBio);
  const [avatarEmail, setAvatarEmail] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Which fields the user has edited. The async prefill below must never
  // overwrite a field the user already started typing in (its fetch can resolve
  // after the user opens the form and begins editing — silently discarding
  // their input otherwise, #4930 review).
  const touched = useRef({ displayName: false, bio: false, avatarEmail: false });

  // Seed the writable fields (notably avatarEmail, absent from GqlProfile) from
  // the authoritative user record. Non-fatal: on failure we keep the values
  // already seeded from the displayed profile. Untouched fields only.
  useEffect(() => {
    let cancelled = false;
    void apiClient.users
      .get(cryptoId)
      .then(user => {
        if (cancelled) return;
        if (!touched.current.displayName && typeof user?.displayName === 'string')
          setDisplayName(user.displayName);
        if (!touched.current.bio && typeof user?.bio === 'string') setBio(user.bio);
        if (!touched.current.avatarEmail && typeof user?.avatarEmail === 'string')
          setAvatarEmail(user.avatarEmail);
      })
      .catch((err: unknown) => {
        log('prefill from users.get failed: %s', String(err));
      });
    return () => {
      cancelled = true;
    };
  }, [cryptoId]);

  const handleSave = useCallback(async () => {
    // A display name is required: `displayName`/`bio` are always sent trimmed,
    // so an empty name would blank the existing one. Guard here (and disable
    // Save below) rather than silently clearing it. `bio` may be empty; only
    // `avatarEmail` is omitted-when-empty (so an existing avatar isn't cleared).
    if (!displayName.trim()) {
      setError(t('agentWorld.profile.nameRequired', 'Display name cannot be empty.'));
      return;
    }
    setSaving(true);
    setError(null);
    log('saving profile update');
    try {
      await apiClient.users.updateProfile(cryptoId, {
        displayName: displayName.trim(),
        bio: bio.trim(),
        // Only send avatarEmail when the user provided one, so an untouched
        // (empty) field never blanks an existing avatar. It is a Gravatar
        // email, not an image URL.
        ...(avatarEmail.trim() ? { avatarEmail: avatarEmail.trim() } : {}),
      });
      log('profile update saved');
      onSaved();
    } catch (err) {
      log('profile update failed: %s', String(err));
      setError(t('agentWorld.profile.saveError', 'Could not save your profile. Try again.'));
    } finally {
      setSaving(false);
    }
  }, [avatarEmail, bio, cryptoId, displayName, onSaved, t]);

  const inputClass =
    'w-full rounded-md border border-line bg-surface px-2.5 py-1.5 text-sm text-content ' +
    'placeholder:text-content-faint focus:border-primary-500 focus:outline-none';

  return (
    <form
      data-testid="profile-edit-form"
      className="space-y-3"
      onSubmit={e => {
        e.preventDefault();
        void handleSave();
      }}>
      <label className="block space-y-1">
        <span className="text-xs font-medium text-content">
          {t('agentWorld.profile.displayName', 'Display name')}
        </span>
        <input
          type="text"
          value={displayName}
          disabled={saving}
          onChange={e => {
            touched.current.displayName = true;
            setDisplayName(e.target.value);
          }}
          className={inputClass}
        />
      </label>

      <label className="block space-y-1">
        <span className="text-xs font-medium text-content">
          {t('agentWorld.profile.bio', 'Bio')}
        </span>
        <textarea
          value={bio}
          rows={3}
          disabled={saving}
          onChange={e => {
            touched.current.bio = true;
            setBio(e.target.value);
          }}
          className={`${inputClass} resize-y`}
        />
      </label>

      <label className="block space-y-1">
        <span className="text-xs font-medium text-content">
          {t('agentWorld.profile.avatarEmail', 'Avatar email')}
        </span>
        <input
          type="email"
          value={avatarEmail}
          disabled={saving}
          onChange={e => {
            touched.current.avatarEmail = true;
            setAvatarEmail(e.target.value);
          }}
          className={inputClass}
        />
        <span className="text-[11px] text-content-muted">
          {t('agentWorld.profile.avatarEmailHint', 'Used to fetch your avatar from Gravatar.')}
        </span>
      </label>

      {error && <p className="text-xs text-red-600 dark:text-red-400">{error}</p>}

      <div className="flex items-center gap-2">
        <Button type="submit" variant="primary" size="sm" disabled={saving || !displayName.trim()}>
          {saving
            ? t('agentWorld.profile.saving', 'Saving…')
            : t('agentWorld.profile.save', 'Save')}
        </Button>
        <Button type="button" variant="secondary" size="sm" disabled={saving} onClick={onCancel}>
          {t('agentWorld.profile.cancel', 'Cancel')}
        </Button>
      </div>
    </form>
  );
}

function AgentProfileCard({ data, onSwitched }: { data: ProfileData; onSwitched?: () => void }) {
  const { t } = useT();
  const [editing, setEditing] = useState(false);
  const [followStats, setFollowStats] = useState<FollowStats | null>(null);
  const [exportData, setExportData] = useState<IdentityExport | null>(null);
  const [exportLoading, setExportLoading] = useState(false);
  const [exportError, setExportError] = useState<string | null>(null);
  // Handle currently being promoted to primary (in-flight), and any error.
  const [switchingHandle, setSwitchingHandle] = useState<string | null>(null);
  const [switchError, setSwitchError] = useState<string | null>(null);

  // ── Extract display fields from either data source ─────────────────────────
  const isGraphql = data.source === 'graphql';
  const profile = isGraphql ? data.profile : null;
  const identity = isGraphql ? null : data.identity;

  // Determine the display name / handle.
  // GraphQL with a registered identity → @username (the wallet's PRIMARY handle,
  // not merely identities[0] — a user with several handles can mark any one
  // primary, #4198).
  // GraphQL without identities (null) → displayName (no @ prefix, not a handle).
  // Directory fallback → @username from identity.
  const primaryIdentity = isGraphql ? pickPrimary(profile!.identities ?? []) : undefined;
  const primaryIdentityUsername = isGraphql ? (primaryIdentity?.username ?? null) : null;
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

  // Promote one of the wallet's handles to primary (active), then ask the
  // parent to refetch so the new primary is reflected everywhere (#4198).
  const handleSetActive = useCallback(
    async (username: string) => {
      const name = username.replace(/^@+/, '');
      if (switchingHandle) return;
      setSwitchError(null);
      setSwitchingHandle(name);
      try {
        await apiClient.registry.assignPrimary(name);
        onSwitched?.();
      } catch (err) {
        setSwitchError(String(err));
      } finally {
        setSwitchingHandle(null);
      }
    },
    [switchingHandle, onSwitched]
  );

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
    <div className="rounded-lg border border-line bg-surface p-4">
      <div className="flex items-start gap-4">
        {avatarUrl ? (
          <img
            src={avatarUrl}
            alt={agentName}
            className="h-14 w-14 shrink-0 rounded-full object-cover"
          />
        ) : (
          <div className="bg-primary-600 flex h-14 w-14 shrink-0 items-center justify-center rounded-full text-lg font-semibold text-content-inverted">
            {initials}
          </div>
        )}
        <div className="min-w-0">
          <h3 className="flex items-center gap-1.5 text-sm font-semibold text-content">
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
            <p className="mt-0.5 font-mono text-xs text-content-muted" title={cryptoId}>
              {truncateCryptoId(cryptoId)}
            </p>
          )}
          {bio && <p className="mt-1.5 text-xs leading-relaxed text-content-secondary">{bio}</p>}
        </div>
      </div>

      {/* Own-profile editing (#4930). Only the graphql source carries an
          editable profile + cryptoId; the bare directory fallback has neither. */}
      {isGraphql && cryptoId && (
        <div className="mt-4 border-t border-line pt-4">
          {editing ? (
            <ProfileEditForm
              cryptoId={cryptoId}
              initialDisplayName={displayName}
              initialBio={bio}
              onCancel={() => setEditing(false)}
              onSaved={() => {
                setEditing(false);
                // Refetch so the saved name/bio/avatar render (reuses the
                // active-handle-switch reload path).
                onSwitched?.();
              }}
            />
          ) : (
            <Button variant="secondary" size="sm" onClick={() => setEditing(true)}>
              {t('agentWorld.profile.edit', 'Edit profile')}
            </Button>
          )}
        </div>
      )}

      {skills.length > 0 && (
        <div className="mt-4 border-t border-line pt-4">
          <h4 className="mb-2 text-xs font-medium text-content">Skills</h4>
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
          <h4 className="mb-2 text-xs font-medium text-content">Verified Accounts</h4>
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
            Handles owned{ownedIdentities.length > 1 ? ` (${ownedIdentities.length})` : ''}
          </h4>
          <div className="space-y-1.5">
            {ownedIdentities.map(id => (
              <div
                key={id.username}
                className="flex items-center justify-between gap-3 rounded-lg border border-line px-3 py-1.5 text-sm">
                <span className="truncate font-medium text-content">
                  @{id.username.replace(/^@+/, '')}
                </span>
                <span className="flex items-center gap-2">
                  {id.primary ? (
                    <span className="rounded-full bg-primary-50 px-1.5 py-0.5 text-[10px] font-medium text-primary-600 dark:bg-primary-900/30 dark:text-primary-300">
                      active
                    </span>
                  ) : (
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={switchingHandle !== null}
                      onClick={() => void handleSetActive(id.username)}>
                      {switchingHandle === id.username.replace(/^@+/, '')
                        ? 'Switching…'
                        : 'Make active'}
                    </Button>
                  )}
                  <span className="text-[10px] uppercase tracking-wide text-content-faint">
                    {id.status}
                  </span>
                </span>
              </div>
            ))}
          </div>
          {switchError && (
            <p className="mt-2 text-xs text-red-600 dark:text-red-400">{switchError}</p>
          )}
        </div>
      )}

      {followStats && (
        <div className="mt-4 border-t border-line pt-4">
          <div className="flex gap-6">
            <div>
              <span className="text-sm font-semibold text-content">
                {followStats.followerCount}
              </span>
              <span className="ml-1 text-xs text-content-muted">
                {followStats.followerCount === 1 ? 'follower' : 'followers'}
              </span>
            </div>
            <div>
              <span className="text-sm font-semibold text-content">
                {followStats.followingCount}
              </span>
              <span className="ml-1 text-xs text-content-muted">following</span>
            </div>
          </div>
        </div>
      )}

      {createdAt && (
        <div className="mt-4 border-t border-line pt-4">
          <span className="text-xs text-content-muted">Joined {formatDate(createdAt)}</span>
        </div>
      )}

      {/* Export identity */}
      <div className="mt-4 border-t border-line pt-4">
        <Button variant="secondary" size="sm" disabled={exportLoading} onClick={handleExport}>
          {exportLoading ? 'Exporting...' : exportData ? 'Hide Export' : 'Export Identity'}
        </Button>
        {exportError && (
          <p className="mt-2 text-xs text-red-600 dark:text-red-400">{exportError}</p>
        )}
        {exportData && (
          <pre className="mt-3 max-h-64 overflow-auto rounded-md bg-surface-muted p-3 text-xs text-content-secondary">
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
      {body && <p className="max-w-md text-sm text-content-muted">{body}</p>}
    </div>
  );
}

// ── Main export ───────────────────────────────────────────────────────────────

export default function ProfilesSection() {
  const [reloadKey, setReloadKey] = useState(0);
  const state = useMyIdentity(reloadKey);

  let body: React.ReactNode;

  if (state.status === 'loading') {
    body = (
      <div className="flex h-64 items-center justify-center text-content-faint">
        <span className="animate-pulse text-sm">Loading your profile…</span>
      </div>
    );
  } else if (state.status === 'wallet_locked') {
    body = (
      <StatusBlock
        tone="text-content-secondary"
        title="Unlock your wallet to use Agent World"
        body="Agent World uses your wallet identity. Import your recovery phrase in Settings to continue."
      />
    );
  } else if (state.status === 'no_handle') {
    body = (
      <StatusBlock
        tone="text-content-secondary"
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
    body = <AgentProfileCard data={state.data} onSwitched={() => setReloadKey(k => k + 1)} />;
  }

  return <PanelScaffold description="Your agent profile">{body}</PanelScaffold>;
}
