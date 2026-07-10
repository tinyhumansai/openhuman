/**
 * UserErrorCenter (#3931)
 * -----------------------
 *
 * First-class, shell-mounted surface for user-actionable runtime errors. It is
 * rendered once in the desktop shell (outside the router) so entries stay
 * visible across route changes and after background/cron jobs finish — there is
 * no dependence on an active chat route.
 *
 * Notification affordance: while unresolved entries exist, a fixed trigger with
 * a count badge appears. Opening it reveals the panel; each entry shows a title,
 * one-line explanation, source + timestamp, recurrence count, a primary action
 * that deep-links to the relevant settings flow, and a dismiss control.
 *
 * Privacy: only translated copy (via i18n keys) and privacy-safe metadata are
 * shown — never raw provider responses, tokens, prompts, or PII.
 */
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import { selectActiveUserErrors } from '../../store/userErrorsSelectors';
import { dismissUserError, resolveUserError } from '../../store/userErrorsSlice';
import type { UserActionableError, UserErrorAction } from '../../types/userError';

/** Deep-link target for each primary action. `dismiss` has no route. */
const ACTION_ROUTE: Record<Exclude<UserErrorAction, 'dismiss'>, string> = {
  open_billing: '/settings/billing',
  open_provider_settings: '/settings/llm',
};

/** i18n key for each primary action's button label. */
const ACTION_LABEL_KEY: Record<Exclude<UserErrorAction, 'dismiss'>, string> = {
  open_billing: 'userErrors.action.openBilling',
  open_provider_settings: 'userErrors.action.openProviderSettings',
};

// Wall-clock read for the resolve/dismiss timestamps. Defined at module scope
// so the component body doesn't reference an impure function during render
// (react-hooks/purity); the calls below run only from event handlers.
const nowMs = (): number => Date.now();

export function UserErrorCenter() {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const active = useAppSelector(selectActiveUserErrors);
  const [open, setOpen] = useState(false);

  // Nothing to surface → render nothing at all (no idle chrome in the shell).
  if (active.length === 0) return null;

  const runAction = (entry: UserActionableError) => {
    if (entry.action !== 'dismiss') {
      navigate(ACTION_ROUTE[entry.action]);
    }
    dispatch(resolveUserError({ id: entry.id, at: nowMs() }));
    setOpen(false);
  };

  return (
    <div
      className="fixed bottom-4 right-4 z-50 flex flex-col items-end gap-2"
      data-testid="user-error-center">
      {open && (
        <div
          role="dialog"
          aria-label={t('userErrors.title')}
          data-testid="user-error-panel"
          className="w-80 max-w-[calc(100vw-2rem)] overflow-hidden rounded-xl border border-amber-300 bg-surface shadow-lg dark:border-amber-700">
          <div className="flex items-center justify-between border-b border-amber-200 px-3 py-2 dark:border-amber-800">
            <span className="text-sm font-semibold text-ink dark:text-content">
              {t('userErrors.title')}
            </span>
            <span className="rounded-full bg-amber-100 px-2 py-0.5 text-xs font-medium text-amber-900 dark:bg-amber-900 dark:text-amber-100">
              {active.length}
            </span>
          </div>

          <ul className="max-h-96 divide-y divide-line-subtle overflow-auto dark:divide-neutral-800">
            {active.map(entry => (
              <li key={entry.id} className="px-3 py-2.5" data-testid="user-error-item">
                <p className="text-sm font-semibold text-ink dark:text-content">
                  {t(entry.titleKey)}
                </p>
                <p className="mt-0.5 text-xs text-ink-soft dark:text-content-secondary">
                  {t(entry.bodyKey)}
                </p>
                <p className="mt-1 text-[11px] text-content-muted">
                  {t(`userErrors.scope.${entry.scope}`, entry.scope)}
                  {' · '}
                  {new Date(entry.lastSeenAt).toLocaleTimeString()}
                  {entry.count > 1 ? ` · ×${entry.count}` : ''}
                </p>
                <div className="mt-2 flex items-center gap-2">
                  {entry.action !== 'dismiss' && (
                    <button
                      type="button"
                      data-testid="user-error-action"
                      onClick={() => runAction(entry)}
                      className="rounded-md bg-ocean px-2.5 py-1 text-xs font-medium text-white hover:bg-ocean/90">
                      {t(ACTION_LABEL_KEY[entry.action])}
                    </button>
                  )}
                  <button
                    type="button"
                    data-testid="user-error-dismiss"
                    onClick={() => dispatch(dismissUserError({ id: entry.id }))}
                    className="rounded-md px-2.5 py-1 text-xs font-medium text-ink-soft hover:bg-surface-hover dark:text-content-secondary">
                    {t('userErrors.dismiss')}
                  </button>
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}

      <button
        type="button"
        data-testid="user-error-trigger"
        aria-label={t('userErrors.title')}
        onClick={() => setOpen(o => !o)}
        className="relative flex h-11 w-11 items-center justify-center rounded-full border border-amber-300 bg-surface text-lg shadow-md hover:bg-amber-50 dark:border-amber-700 dark:hover:bg-surface-muted">
        <span aria-hidden>⚠️</span>
        <span
          data-testid="user-error-badge"
          className="absolute -right-1 -top-1 min-w-[18px] rounded-full bg-coral px-1 text-center text-[11px] font-bold leading-[18px] text-white">
          {active.length}
        </span>
      </button>
    </div>
  );
}

export default UserErrorCenter;
