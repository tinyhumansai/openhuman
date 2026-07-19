/**
 * MedullaDemoChat — the scale showcase for the "Chat" tab shown to users
 * without Medulla access. A short, read-only orchestration conversation (the
 * orchestrator fanning work out to agents) with the composer disabled and a
 * preview banner. No live agent, no core RPC.
 */
import { useT } from '../../../lib/i18n/I18nContext';
import DemoScaleBanner from './DemoScaleBanner';
import { DEMO_CHAT } from './medullaDemoData';

export default function MedullaDemoChat() {
  const { t } = useT();

  return (
    <div className="flex h-full flex-col" data-testid="orch-demo-chat">
      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-2xl space-y-4 px-4 py-6">
          <DemoScaleBanner />

          {DEMO_CHAT.map(msg => {
            if (msg.role === 'activity') {
              // Sub-agent activity line — a compact, centered status row.
              return (
                <div key={msg.id} className="flex items-center justify-center gap-2">
                  <span className="h-1.5 w-1.5 flex-shrink-0 animate-pulse rounded-full bg-primary-500" />
                  <span className="font-mono text-[11px] text-content-muted">{t(msg.textKey)}</span>
                </div>
              );
            }
            const isUser = msg.role === 'user';
            return (
              <div key={msg.id} className={`flex ${isUser ? 'justify-end' : 'justify-start'}`}>
                <div
                  className={`max-w-[80%] rounded-2xl px-4 py-2.5 text-sm leading-relaxed shadow-soft ${
                    isUser
                      ? 'bg-primary-500 text-content-inverted'
                      : 'border border-line bg-surface text-content'
                  }`}>
                  {t(msg.textKey)}
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* Disabled composer — visually present, non-interactive. */}
      <div className="border-t border-line bg-surface/80 px-4 py-3">
        <div className="mx-auto flex w-full max-w-2xl items-center gap-2">
          <div
            aria-disabled="true"
            className="flex-1 cursor-not-allowed select-none rounded-xl border border-line bg-surface-subtle px-4 py-2.5 text-sm text-content-faint">
            {t('orchPage.demo.chat.composerDisabled')}
          </div>
          <button
            type="button"
            disabled
            aria-label={t('orchPage.demo.chat.composerDisabled')}
            className="flex h-9 w-9 flex-shrink-0 cursor-not-allowed items-center justify-center rounded-xl bg-surface-subtle text-content-faint">
            <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M12 19l9 2-9-18-9 18 9-2zm0 0v-8"
              />
            </svg>
          </button>
        </div>
      </div>
    </div>
  );
}
