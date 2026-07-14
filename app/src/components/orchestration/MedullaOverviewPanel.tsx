/**
 * MedullaOverviewPanel — the landing "Overview" for the Orchestration page.
 *
 * A teaser for **Medulla**, OpenHuman's custom-built LLM designed specifically
 * to orchestrate thousands of agents at once. It is not shipped yet; this page
 * announces it and points early-access seekers at the Discord. OpenHuman
 * subscribers get early access to the model and the orchestration engine.
 *
 * Pure marketing surface — no core RPC, no live state. Product/brand names
 * ("Medulla", "OpenHuman", "Discord") stay in every locale.
 */
import { useT } from '../../lib/i18n/I18nContext';
import { DISCORD_INVITE_URL } from '../../utils/links';
import { openUrl } from '../../utils/openUrl';

export default function MedullaOverviewPanel() {
  const { t } = useT();

  return (
    <div className="h-full overflow-y-auto" data-testid="orch-medulla">
      <div className="mx-auto flex min-h-full w-full max-w-2xl flex-col items-center justify-center px-6 py-12 text-center">
        {/* Coming-soon badge */}
        <span className="mb-6 inline-flex items-center gap-1.5 rounded-full border border-primary-200 bg-primary-50 px-3 py-1 text-[11px] font-semibold uppercase tracking-wider text-primary-700 dark:border-primary-500/30 dark:bg-primary-500/10 dark:text-primary-300">
          <span className="h-1.5 w-1.5 rounded-full bg-primary-500" aria-hidden="true" />
          {t('orchPage.medulla.badge')}
        </span>

        {/* Wordmark */}
        <h1 className="bg-gradient-to-r from-primary-500 to-primary-700 bg-clip-text text-5xl font-bold tracking-tight text-transparent dark:from-primary-300 dark:to-primary-500">
          {t('orchPage.medulla.title')}
        </h1>
        <p className="mt-2 text-base font-medium text-content-secondary">
          {t('orchPage.medulla.tagline')}
        </p>

        {/* Lead paragraph */}
        <p className="mt-6 max-w-xl text-sm leading-relaxed text-content-muted">
          {t('orchPage.medulla.body')}
        </p>

        {/* Feature highlights */}
        <div className="mt-6 flex flex-wrap items-center justify-center gap-2">
          {[
            { emoji: '🤖', label: t('orchPage.medulla.featAgents') },
            { emoji: '🧠', label: t('orchPage.medulla.featContext') },
            { emoji: '⚡', label: t('orchPage.medulla.featCost') },
          ].map(feat => (
            <span
              key={feat.label}
              className="inline-flex items-center gap-1.5 rounded-full border border-line bg-surface px-3 py-1 text-xs font-medium text-content-secondary shadow-soft">
              <span aria-hidden="true">{feat.emoji}</span>
              {feat.label}
            </span>
          ))}
        </div>

        {/* Two early-access cards: OpenHuman subscribers · the Discord. */}
        <div className="mt-8 grid w-full max-w-xl grid-cols-1 gap-3 text-left sm:grid-cols-2">
          {/* Subscribers */}
          <div className="flex flex-col rounded-2xl border border-line bg-surface p-4 shadow-soft">
            <span
              className="mb-3 flex h-9 w-9 items-center justify-center rounded-full bg-primary-50 text-primary-600 dark:bg-primary-500/10 dark:text-primary-300"
              aria-hidden="true">
              <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M13 10V3L4 14h7v7l9-11h-7z"
                />
              </svg>
            </span>
            <h3 className="text-sm font-semibold text-content">
              {t('orchPage.medulla.subscriberTitle')}
            </h3>
            <p className="mt-1 text-xs leading-relaxed text-content-muted">
              {t('orchPage.medulla.subscriberNote')}
            </p>
          </div>

          {/* Discord */}
          <div className="flex flex-col rounded-2xl border border-line bg-surface p-4 shadow-soft">
            <span
              className="mb-3 flex h-9 w-9 items-center justify-center rounded-full bg-primary-50 text-primary-600 dark:bg-primary-500/10 dark:text-primary-300"
              aria-hidden="true">
              <svg className="h-4 w-4" fill="currentColor" viewBox="0 0 24 24">
                <path d="M20.317 4.369a19.79 19.79 0 00-4.885-1.515.074.074 0 00-.079.037c-.211.375-.445.865-.608 1.25a18.27 18.27 0 00-5.487 0 12.64 12.64 0 00-.617-1.25.077.077 0 00-.079-.037A19.736 19.736 0 003.677 4.37a.07.07 0 00-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 00.031.057 19.9 19.9 0 005.993 3.03.078.078 0 00.084-.028c.462-.63.874-1.295 1.226-1.994a.076.076 0 00-.041-.106 13.107 13.107 0 01-1.872-.892.077.077 0 01-.008-.128c.126-.094.252-.192.372-.291a.074.074 0 01.077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 01.078.009c.12.099.246.198.373.292a.077.077 0 01-.006.127 12.3 12.3 0 01-1.873.892.077.077 0 00-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 00.084.028 19.839 19.839 0 006.002-3.03.077.077 0 00.032-.056c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 00-.031-.028zM8.02 15.331c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.955-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.956 2.418-2.157 2.418zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.955-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.946 2.418-2.157 2.418z" />
              </svg>
            </span>
            <h3 className="text-sm font-semibold text-content">
              {t('orchPage.medulla.discordTitle')}
            </h3>
            <p className="mt-1 text-xs leading-relaxed text-content-muted">
              {t('orchPage.medulla.earlyAccess')}
            </p>
            <button
              type="button"
              data-testid="orch-medulla-discord"
              onClick={() => void openUrl(DISCORD_INVITE_URL)}
              className="mt-3 inline-flex items-center justify-center gap-2 self-start rounded-xl bg-primary-500 px-4 py-2 text-xs font-semibold text-content-inverted shadow-soft transition-colors hover:bg-primary-600 focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-500 focus-visible:ring-offset-1">
              {t('orchPage.medulla.cta')}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
