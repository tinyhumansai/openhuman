/**
 * In-app "Star us on GitHub" CTA (#5005).
 *
 * A subtle, dismissible card that nudges satisfied users to star the OpenHuman
 * repo without leaving the app. Deliberately non-intrusive: it blocks nothing,
 * lives inside Settings → About (a surface the user navigates to on purpose, so
 * it is never shown on first launch), and never reappears once the user stars
 * or dismisses it — the choice is persisted per-user via the `githubStar`
 * Redux slice (see store/githubStarSlice.ts).
 *
 * Clicking "Star" opens the repo in the host browser through the shared
 * `openUrl` helper (never `window.open` directly) so it works on macOS,
 * Windows, and Linux via `tauri-plugin-opener`.
 */
import { useCallback } from 'react';

import { trackAnalyticsEvent } from '../../components/analytics';
import Button from '../../components/ui/Button';
import { useT } from '../../lib/i18n/I18nContext';
import { dismissGithubStarCta } from '../../store/githubStarSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import { OPENHUMAN_GITHUB_REPO_URL } from '../../utils/config';
import { openUrl } from '../../utils/openUrl';

const LOG_PREFIX = '[github-star-cta]';

export function GitHubStarCard() {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const dismissed = useAppSelector(state => state.githubStar.dismissed);

  const handleStar = useCallback(() => {
    console.debug(`${LOG_PREFIX} star clicked; opening repo`);
    trackAnalyticsEvent('github_star_cta_clicked');
    // Acting on the CTA also retires it — a user who starred does not want the
    // nudge to keep reappearing.
    dispatch(dismissGithubStarCta());
    void openUrl(OPENHUMAN_GITHUB_REPO_URL).catch(err =>
      console.debug(`${LOG_PREFIX} openUrl failed: ${String(err)}`)
    );
  }, [dispatch]);

  const handleDismiss = useCallback(() => {
    console.debug(`${LOG_PREFIX} dismissed`);
    trackAnalyticsEvent('github_star_cta_dismissed');
    dispatch(dismissGithubStarCta());
  }, [dispatch]);

  // Durable dismissal: once handled, never render again.
  if (dismissed) return null;

  return (
    <div
      data-testid="github-star-cta"
      className="px-4 py-3 space-y-2 rounded-xl border border-primary-500/30 bg-primary-500/5">
      <div className="flex items-start gap-2">
        <span aria-hidden="true" className="text-lg leading-none">
          ⭐
        </span>
        <div className="space-y-1">
          <div className="text-sm font-medium text-content">
            {t('settings.about.starCta.title')}
          </div>
          <p className="text-xs text-content-muted leading-relaxed">
            {t('settings.about.starCta.body')}
          </p>
        </div>
      </div>
      <div className="flex flex-wrap gap-2">
        <Button
          type="button"
          variant="primary"
          size="xs"
          analyticsId="github_star_cta_clicked"
          onClick={handleStar}>
          {t('settings.about.starCta.star')}
        </Button>
        <Button
          type="button"
          variant="tertiary"
          size="xs"
          analyticsId="github_star_cta_dismissed"
          onClick={handleDismiss}>
          {t('settings.about.starCta.dismiss')}
        </Button>
      </div>
    </div>
  );
}
