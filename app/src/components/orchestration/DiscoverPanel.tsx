/**
 * DiscoverPanel — "handle connections and guide the user to add more".
 *
 * The growth surface: shows this agent's own tiny.place discoverability (so the
 * user knows whether peers can even reach them), a link-a-new-agent form, and
 * the inbound contact-request queue (accept / decline / block). Steady-state
 * management of already-linked peers lives in the sibling {@link ConnectionsPanel}.
 */
import { type FormEvent, useCallback, useEffect, useMemo, useState } from 'react';

import { apiClient } from '../../agentworld/AgentWorldShell';
import { useT } from '../../lib/i18n/I18nContext';
import {
  orchestrationClient,
  type RelayInfo,
  type SelfIdentity,
} from '../../lib/orchestration/orchestrationClient';
import { usePairing } from '../../lib/orchestration/usePairing';
import { contactAddress, extractHandle } from '../intelligence/orchestrationTabHelpers';
import Button from '../ui/Button';
import { SectionCard } from './primitives';

export default function DiscoverPanel() {
  const { t } = useT();
  const { state, runAction, pendingAction, actionError } = usePairing();
  const [linkAgentId, setLinkAgentId] = useState('');
  const [identity, setIdentity] = useState<SelfIdentity | null>(null);
  const [relay, setRelay] = useState<RelayInfo | null>(null);
  const [identityLoading, setIdentityLoading] = useState(true);
  const [handles, setHandles] = useState<Record<string, string | null>>({});

  useEffect(() => {
    let cancelled = false;
    void Promise.allSettled([
      orchestrationClient.selfIdentity(),
      orchestrationClient.relayInfo(),
    ]).then(([id, rel]) => {
      if (cancelled) return;
      if (id.status === 'fulfilled') setIdentity(id.value);
      if (rel.status === 'fulfilled') setRelay(rel.value);
      setIdentityLoading(false);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const snapshot = state.status === 'ok' ? state.snapshot : null;
  const incoming = useMemo(() => snapshot?.requests.incoming ?? [], [snapshot?.requests.incoming]);

  // Resolve @handles for the inbound requests (best-effort; address always shown).
  const addressesKey = incoming.map(contactAddress).filter(Boolean).join(',');
  useEffect(() => {
    const ids = addressesKey ? Array.from(new Set(addressesKey.split(','))) : [];
    if (ids.length === 0) return;
    let cancelled = false;
    void Promise.all(
      ids.map(async id => {
        try {
          return [id, extractHandle(await apiClient.directory.reverse(id))] as const;
        } catch {
          return [id, null] as const;
        }
      })
    ).then(entries => {
      if (cancelled) return;
      setHandles(prev => {
        const next = { ...prev };
        for (const [id, handle] of entries) if (!(id in next)) next[id] = handle;
        return next;
      });
    });
    return () => {
      cancelled = true;
    };
  }, [addressesKey]);

  const submitLink = useCallback(
    (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const agentId = linkAgentId.trim();
      if (!agentId) return;
      void runAction(`link:${agentId}`, async () => {
        await apiClient.orchestrationPairing.linkSession(agentId);
        setLinkAgentId('');
      });
    },
    [linkAgentId, runAction]
  );

  const discoverable = identity?.discoverable ?? false;

  return (
    <div className="space-y-5" data-testid="orch-discover-panel">
      {/* Own identity / discoverability. */}
      <SectionCard title={t('orchPage.discover.identityTitle')} testId="orch-discover-identity">
        {identityLoading ? (
          <p className="text-sm text-content-muted">
            {t('tinyplaceOrchestration.identity.loading')}
          </p>
        ) : identity ? (
          <div className="space-y-2 text-sm">
            <div className="flex items-center gap-2">
              <span
                className={`rounded-full px-2 py-0.5 text-xs font-medium ${
                  discoverable
                    ? 'bg-sage-100 text-sage-700 dark:bg-sage-500/15 dark:text-sage-300'
                    : 'bg-amber-100 text-amber-700 dark:bg-amber-500/15 dark:text-amber-300'
                }`}
                data-testid="orch-discover-discoverable">
                {discoverable
                  ? t('tinyplaceOrchestration.identity.discoverable')
                  : t('tinyplaceOrchestration.identity.undiscoverable')}
              </span>
              {relay && (
                <span className="text-xs text-content-faint">
                  {relay.network === 'prod'
                    ? t('tinyplaceOrchestration.relay.prod')
                    : t('tinyplaceOrchestration.relay.staging')}
                </span>
              )}
            </div>
            <p className="text-content-secondary">
              {identity.primaryHandle
                ? `@${identity.primaryHandle.replace(/^@+/, '')}`
                : t('tinyplaceOrchestration.identity.noHandle')}
            </p>
            {!discoverable && (
              <p className="text-xs text-content-muted" data-testid="orch-discover-guide">
                {t('orchPage.discover.notDiscoverableGuide')}
              </p>
            )}
          </div>
        ) : (
          <p className="text-sm text-content-muted">
            {t('tinyplaceOrchestration.identity.noHandle')}
          </p>
        )}
      </SectionCard>

      {/* Link a new agent. */}
      <SectionCard
        title={t('orchPage.discover.linkTitle')}
        description={t('orchPage.discover.linkDescription')}
        testId="orch-discover-link">
        <form className="flex gap-2" onSubmit={submitLink}>
          <input
            value={linkAgentId}
            onChange={e => setLinkAgentId(e.target.value)}
            placeholder={t('tinyplaceOrchestration.pairing.linkPlaceholder')}
            className="min-w-0 flex-1 rounded-md border border-line bg-surface px-3 py-2 text-sm text-content outline-none transition focus:border-ocean-500 focus:ring-2 focus:ring-ocean-500/20"
            data-testid="orch-discover-link-input"
          />
          <Button
            type="submit"
            variant="primary"
            size="sm"
            disabled={!linkAgentId.trim() || pendingAction === `link:${linkAgentId.trim()}`}
            data-testid="orch-discover-link-submit">
            {t('tinyplaceOrchestration.pairing.linkAction')}
          </Button>
        </form>
        {actionError && (
          <p className="mt-2 text-xs text-coral-600 dark:text-coral-300">{actionError}</p>
        )}
      </SectionCard>

      {/* Inbound requests. */}
      <SectionCard
        title={t('tinyplaceOrchestration.pairing.requests')}
        testId="orch-discover-requests">
        {state.status === 'loading' ? (
          <p className="text-sm text-content-muted">{t('tinyplaceOrchestration.loading')}</p>
        ) : incoming.length === 0 ? (
          <p className="py-2 text-sm text-content-muted" data-testid="orch-discover-no-requests">
            {t('orchPage.discover.noRequests')}
          </p>
        ) : (
          <ul className="divide-y divide-line">
            {incoming.map(request => {
              const address = contactAddress(request);
              const handle = handles[address];
              return (
                <li
                  key={address}
                  className="flex items-center justify-between gap-3 py-2.5"
                  data-testid="orch-discover-request-row">
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium text-content">
                      {handle ? `@${handle}` : t('tinyplaceOrchestration.unknownSender')}
                    </p>
                    <p className="truncate font-mono text-[11px] text-content-faint">{address}</p>
                  </div>
                  <div className="flex shrink-0 gap-1.5">
                    <Button
                      variant="primary"
                      size="sm"
                      disabled={pendingAction === `accept:${address}`}
                      onClick={() =>
                        void runAction(`accept:${address}`, () =>
                          apiClient.orchestrationPairing.acceptRequest(address)
                        )
                      }
                      data-testid="orch-discover-accept">
                      {t('tinyplaceOrchestration.pairing.accept')}
                    </Button>
                    <Button
                      variant="tertiary"
                      size="sm"
                      disabled={pendingAction === `decline:${address}`}
                      onClick={() =>
                        void runAction(`decline:${address}`, () =>
                          apiClient.orchestrationPairing.declineRequest(address)
                        )
                      }
                      data-testid="orch-discover-decline">
                      {t('tinyplaceOrchestration.pairing.decline')}
                    </Button>
                  </div>
                </li>
              );
            })}
          </ul>
        )}
      </SectionCard>
    </div>
  );
}
