/**
 * AgentProfileModal — read view of a Tiny Place directory entry's profile.
 *
 * Opened from `DirectorySection` when a directory card is clicked (previously a
 * dead path that only toggled a highlight — GH-4927). Seeds instantly from the
 * `AgentCard` already in hand, then enriches with the full public profile via
 * `apiClient.graphql.user(agentId)` (`tinyplace_graphql_user`). If that lookup
 * fails or 402s, the modal degrades gracefully to the card data with a soft
 * notice rather than blanking.
 */
import debugFactory from 'debug';
import { useEffect, useState } from 'react';

import { ModalShell } from '../../components/ui/ModalShell';
import {
  type AgentCard,
  type GqlProfile,
  PaymentRequiredError,
} from '../../lib/agentworld/invokeApiClient';
import { useT } from '../../lib/i18n/I18nContext';
import { apiClient } from '../AgentWorldShell';
import { getHandle, getInitials, getSkills } from '../pages/directoryHelpers';

const debug = debugFactory('agentworld:directory');

/** Enrichment fetch state for the full GraphQL profile. */
type ProfileFetch =
  | { status: 'loading' }
  | { status: 'ok'; profile: GqlProfile }
  | { status: 'unavailable' };

/** Format an ISO timestamp as a short human date; fall back to the raw value. */
function formatJoined(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  // Runtime locale (not hard-coded en-US) so the date localizes with the app.
  return date.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
}

export interface AgentProfileModalProps {
  agent: AgentCard;
  onClose: () => void;
}

export default function AgentProfileModal({ agent, onClose }: AgentProfileModalProps) {
  const { t } = useT();
  const [fetchState, setFetchState] = useState<ProfileFetch>({ status: 'loading' });

  useEffect(() => {
    let cancelled = false;
    debug('[tinyplace][ui] AgentProfileModal: fetching full profile for a directory entry');

    void apiClient.graphql
      .user(agent.agentId)
      .then(profile => {
        if (cancelled) return;
        if (profile) {
          debug('[tinyplace][ui] AgentProfileModal: full profile loaded');
          setFetchState({ status: 'ok', profile });
        } else {
          debug('[tinyplace][ui] AgentProfileModal: no profile record, using card data');
          setFetchState({ status: 'unavailable' });
        }
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        if (err instanceof PaymentRequiredError) {
          debug('[tinyplace][ui] AgentProfileModal: 402 payment_required, using card data');
        } else {
          debug('[tinyplace][ui] AgentProfileModal: profile fetch failed: %s', String(err));
        }
        setFetchState({ status: 'unavailable' });
      });

    return () => {
      cancelled = true;
    };
  }, [agent.agentId]);

  const profile = fetchState.status === 'ok' ? fetchState.profile : null;
  const handle = getHandle(agent);
  const initials = getInitials(agent);
  const actorType = profile?.actorType?.trim() || undefined;
  const bio = profile?.bio?.trim() || (agent.description ?? '').trim();
  const tags = profile?.tags && profile.tags.length > 0 ? profile.tags : getSkills(agent);
  const verified = profile?.verified === true;
  const joined = profile?.createdAt ? formatJoined(profile.createdAt) : null;

  return (
    <ModalShell
      title={handle}
      titleId="agentworld-directory-profile-title"
      subtitle={actorType}
      maxWidthClassName="max-w-md"
      onClose={onClose}
      icon={<span className="text-xs font-semibold">{initials}</span>}>
      <div className="space-y-4" data-testid="agent-profile-modal">
        {/* Meta row: verified badge + joined date. */}
        {(verified || joined) && (
          <div className="flex flex-wrap items-center gap-2 text-xs">
            {verified && (
              <span className="rounded-full bg-emerald-100 px-2 py-0.5 font-medium text-emerald-700 dark:bg-emerald-900/30 dark:text-emerald-400">
                {t('agentWorld.directory.profile.verified')}
              </span>
            )}
            {joined && (
              <span className="text-content-faint">
                {t('agentWorld.directory.profile.joined')} {joined}
              </span>
            )}
          </div>
        )}

        {/* Bio (or an empty-state placeholder). */}
        <p className="text-sm text-content-secondary">
          {bio || t('agentWorld.directory.profile.noBio')}
        </p>

        {/* Skills / tags. */}
        {tags.length > 0 && (
          <div>
            <p className="mb-1.5 text-xs font-medium text-content-muted">
              {t('agentWorld.directory.profile.skills')}
            </p>
            <div className="flex flex-wrap gap-1">
              {tags.map(tag => (
                <span
                  key={tag}
                  className="rounded-full bg-surface-subtle px-1.5 py-0.5 text-xs text-content-secondary">
                  {tag}
                </span>
              ))}
            </div>
          </div>
        )}

        {/* Soft notice when the richer profile couldn't be loaded. */}
        {fetchState.status === 'unavailable' && (
          <p className="text-xs text-content-faint" data-testid="agent-profile-load-notice">
            {t('agentWorld.directory.profile.loadError')}
          </p>
        )}
      </div>
    </ModalShell>
  );
}
