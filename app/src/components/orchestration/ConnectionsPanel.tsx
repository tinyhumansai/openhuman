/**
 * ConnectionsPanel — "manage connections".
 *
 * Lists the agent's accepted tiny.place contacts (the peers the main agent can
 * coordinate with), with a per-contact block action and a count summary. Adding
 * new links + accepting requests lives in the sibling {@link DiscoverPanel};
 * this panel is the steady-state management view.
 */
import { useEffect, useMemo, useState } from 'react';

import { apiClient } from '../../agentworld/AgentWorldShell';
import { useT } from '../../lib/i18n/I18nContext';
import { usePairing } from '../../lib/orchestration/usePairing';
import { contactAddress, extractHandle } from '../intelligence/orchestrationTabHelpers';
import Button from '../ui/Button';
import { SectionCard, StatTile } from './primitives';

export default function ConnectionsPanel({ onDiscover }: { onDiscover?: () => void }) {
  const { t } = useT();
  const { state, runAction, pendingAction, actionError } = usePairing();
  const [handles, setHandles] = useState<Record<string, string | null>>({});

  const snapshot = state.status === 'ok' ? state.snapshot : null;
  const accepted = useMemo(
    () => (snapshot?.contacts.contacts ?? []).filter(c => c.status === 'accepted'),
    [snapshot?.contacts.contacts]
  );
  const stats = snapshot?.stats ?? null;

  // Best-effort @handle resolution for the listed contacts (address always shown).
  const addressesKey = accepted.map(contactAddress).filter(Boolean).join(',');
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

  if (state.status === 'loading') {
    return (
      <p
        className="py-8 text-center text-sm text-content-muted"
        data-testid="orch-connections-loading">
        {t('tinyplaceOrchestration.loading')}
      </p>
    );
  }
  if (state.status === 'payment_required') {
    return (
      <p className="py-8 text-center text-sm text-amber-600 dark:text-amber-300">
        {t('tinyplaceOrchestration.paymentRequired')}
      </p>
    );
  }
  if (state.status === 'error') {
    return (
      <p className="py-8 text-center text-sm text-coral-600 dark:text-coral-300">
        {t('tinyplaceOrchestration.failedToLoad')}: {state.message}
      </p>
    );
  }

  return (
    <div className="space-y-5" data-testid="orch-connections-panel">
      <div className="grid grid-cols-2 gap-3">
        <StatTile
          label={t('orchPage.connections.statContacts')}
          value={stats?.contactCount ?? accepted.length}
          testId="orch-connections-stat-contacts"
        />
        <StatTile
          label={t('orchPage.connections.statPending')}
          value={(stats?.pendingIncoming ?? 0) + (stats?.pendingOutgoing ?? 0)}
          hint={t('orchPage.connections.pendingHint')}
          testId="orch-connections-stat-pending"
        />
      </div>

      {actionError && (
        <p className="rounded-md bg-coral-50 px-3 py-2 text-xs text-coral-700 dark:bg-coral-500/10 dark:text-coral-300">
          {actionError}
        </p>
      )}

      <SectionCard
        title={t('orchPage.connections.title')}
        description={t('orchPage.connections.description')}
        action={
          onDiscover && (
            <Button
              variant="secondary"
              size="sm"
              onClick={onDiscover}
              data-testid="orch-connections-add">
              {t('orchPage.discover.linkAction')}
            </Button>
          )
        }>
        {accepted.length === 0 ? (
          <div className="py-6 text-center" data-testid="orch-connections-empty">
            <p className="text-sm text-content-muted">{t('orchPage.connections.empty')}</p>
            {onDiscover && (
              <Button
                variant="primary"
                size="sm"
                className="mt-3"
                onClick={onDiscover}
                data-testid="orch-connections-empty-cta">
                {t('orchPage.connections.emptyCta')}
              </Button>
            )}
          </div>
        ) : (
          <ul className="divide-y divide-line">
            {accepted.map(contact => {
              const address = contactAddress(contact);
              const handle = handles[address];
              const actionId = `block:${address}`;
              return (
                <li
                  key={address}
                  className="flex items-center justify-between gap-3 py-2.5"
                  data-testid="orch-connection-row">
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium text-content">
                      {handle ? `@${handle}` : t('tinyplaceOrchestration.unknownSender')}
                    </p>
                    <p className="truncate font-mono text-[11px] text-content-faint">{address}</p>
                  </div>
                  <Button
                    variant="tertiary"
                    size="sm"
                    disabled={pendingAction === actionId}
                    onClick={() =>
                      void runAction(actionId, () =>
                        apiClient.orchestrationPairing.blockRequest(address)
                      )
                    }
                    data-testid="orch-connection-block">
                    {t('tinyplaceOrchestration.pairing.block')}
                  </Button>
                </li>
              );
            })}
          </ul>
        )}
      </SectionCard>
    </div>
  );
}
