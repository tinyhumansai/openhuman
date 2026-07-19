/**
 * MedullaDemoNetwork — the scale showcase for the "Network" tab shown to users
 * without Medulla access. A busy grid of fake peer agents with mixed connection
 * states, behind the preview banner.
 */
import { useMemo } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import DemoScaleBanner from './DemoScaleBanner';
import { buildDemoPeers, type DemoPeerStatus } from './medullaDemoData';

const STATUS_META: Record<DemoPeerStatus, { dot: string; text: string }> = {
  connected: { dot: 'bg-sage-500', text: 'text-sage-600 dark:text-sage-300' },
  connecting: { dot: 'bg-amber-500 animate-pulse', text: 'text-amber-600 dark:text-amber-300' },
  idle: { dot: 'bg-content-faint', text: 'text-content-faint' },
};

export default function MedullaDemoNetwork() {
  const { t } = useT();
  const peers = useMemo(() => buildDemoPeers(), []);
  const connectedCount = peers.filter(p => p.status === 'connected').length;
  const sessionCount = peers.reduce((sum, p) => sum + p.sessions, 0);

  return (
    <div
      className="mx-auto h-full w-full max-w-5xl overflow-y-auto p-4"
      data-testid="orch-demo-network">
      <div className="mx-auto max-w-3xl animate-fade-up space-y-4">
        <DemoScaleBanner />

        <div className="flex items-baseline justify-between px-1">
          <h2 className="text-sm font-semibold text-content">{t('orchPage.demo.networkTitle')}</h2>
          <p className="text-xs text-content-muted">
            {t('orchPage.demo.networkSummary')
              .replace('{peers}', String(connectedCount))
              .replace('{sessions}', String(sessionCount))}
          </p>
        </div>

        <div
          className="grid gap-2 sm:gap-3"
          style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(11rem, 1fr))' }}>
          {peers.map(peer => {
            const meta = STATUS_META[peer.status];
            return (
              <div
                key={peer.id}
                className="rounded-2xl border border-line bg-surface p-3 shadow-soft"
                data-testid={`orch-demo-peer-${peer.id}`}>
                <div className="flex items-center gap-2">
                  <span className={`h-2 w-2 rounded-full ${meta.dot}`} aria-hidden="true" />
                  <span className="truncate font-mono text-xs font-medium text-content">
                    {peer.address}
                  </span>
                </div>
                <p className={`mt-1.5 text-[11px] font-medium ${meta.text}`}>
                  {t(`orchPage.demo.peer.${peer.status}`)}
                  {peer.sessions > 0 && (
                    <span className="text-content-faint">
                      {' · '}
                      {t('orchPage.demo.peerSessions').replace('{count}', String(peer.sessions))}
                    </span>
                  )}
                </p>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
