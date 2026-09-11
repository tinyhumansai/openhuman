/**
 * NoticeCenter — the single place the app raises notices at the user.
 *
 * A small, quiet FAB in the **bottom-right** corner with a count badge,
 * opening a panel of everything currently worth telling them: classified
 * runtime errors, the memory-embedding budget, plan usage limits. Sources are
 * merged by {@link useAppNotices}; this file is presentation only.
 *
 * It replaces the full-width banners that used to be pushed above every route
 * (`GlobalUpsellBanner`, `MemoryEmbeddingBudgetBanner`). Those displaced page
 * content for something the user frequently cannot act on at that moment, and
 * up to three could stack. Nothing is lost by moving them here: the copy, the
 * CTA and the dismissal rules are carried over intact, including the OS-level
 * notification for an exhausted memory budget, which fires from
 * {@link useEmbeddingBudgetNativeNotice} independently of whether this panel
 * is open — the point of that notification is to reach someone who is *not*
 * looking at the app.
 *
 * Privacy: only translated copy and privacy-safe metadata are rendered — never
 * raw provider responses, tokens, prompts, or PII.
 */
import { useEffect, useRef, useState } from 'react';
import { LuBell, LuInfo, LuTriangleAlert, LuX } from 'react-icons/lu';

import { cn } from '../../lib/cn';
import { useT } from '../../lib/i18n/I18nContext';
import { Button } from '../ui';
import { type AppNotice, type NoticeSeverity, peakSeverity, useAppNotices } from './useAppNotices';
import { useEmbeddingBudgetNativeNotice } from './useEmbeddingBudgetNativeNotice';

/** Accent per severity — the badge fill and the row's leading icon. */
const SEVERITY_STYLE: Record<NoticeSeverity, { badge: string; icon: string }> = {
  error: { badge: 'bg-coral-500 text-white', icon: 'text-coral-500' },
  warning: { badge: 'bg-amber-500 text-white', icon: 'text-amber-500' },
  info: { badge: 'bg-primary-500 text-content-inverted', icon: 'text-primary-500' },
};

function SeverityIcon({ severity, className }: { severity: NoticeSeverity; className?: string }) {
  const Icon = severity === 'info' ? LuInfo : LuTriangleAlert;
  return <Icon aria-hidden className={cn('h-4 w-4 shrink-0', className)} />;
}

function NoticeRow({ notice }: { notice: AppNotice }) {
  const { t } = useT();
  return (
    <li className="flex gap-2.5 px-3 py-2.5" data-testid="notice-item">
      <SeverityIcon
        severity={notice.severity}
        className={cn('mt-0.5', SEVERITY_STYLE[notice.severity].icon)}
      />
      <div className="min-w-0 flex-1">
        <p className="text-sm font-semibold text-content">{notice.title}</p>
        {/* `whitespace-pre-line` so a notice carrying a source detail renders
            it as its own paragraph rather than one run-on line. */}
        <p className="mt-0.5 whitespace-pre-line text-xs leading-relaxed text-content-secondary">
          {notice.body}
        </p>
        {notice.meta && <p className="mt-1 text-[11px] text-content-muted">{notice.meta}</p>}
        {(notice.onAction || notice.onSecondaryAction || notice.onDismiss) && (
          <div className="mt-2 flex items-center gap-1.5">
            {notice.onAction && notice.actionLabel && (
              <Button
                size="xs"
                variant="primary"
                data-testid="notice-action"
                analyticsId="notice-center-action"
                onClick={notice.onAction}>
                {notice.actionLabel}
              </Button>
            )}
            {notice.onSecondaryAction && notice.secondaryActionLabel && (
              <Button
                size="xs"
                variant="secondary"
                data-testid="notice-secondary-action"
                analyticsId="notice-center-secondary-action"
                onClick={notice.onSecondaryAction}>
                {notice.secondaryActionLabel}
              </Button>
            )}
            {notice.onDismiss && (
              <Button
                size="xs"
                variant="tertiary"
                data-testid="notice-dismiss"
                analyticsId="notice-center-dismiss"
                onClick={notice.onDismiss}>
                {t('userErrors.dismiss')}
              </Button>
            )}
          </div>
        )}
      </div>
    </li>
  );
}

export default function NoticeCenter() {
  const { t } = useT();
  const notices = useAppNotices();
  const [open, setOpen] = useState(false);
  // Reaches a user who is not looking at the app; deliberately independent of
  // whether this panel is open, or even rendered (the hook runs before the
  // empty-list early return below).
  useEmbeddingBudgetNativeNotice();
  const panelRef = useRef<HTMLDivElement | null>(null);

  // Close on Escape while the panel is open. A fixed overlay with no dismissal
  // affordance beyond its own toggle is a trap for keyboard users.
  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(false);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [open]);

  // The last notice going away closes the panel, so an empty dialog is never
  // left hanging over the app after the user fixes the last thing in it.
  useEffect(() => {
    if (notices.length === 0) setOpen(false);
  }, [notices.length]);

  // Nothing to say → no idle chrome in the shell at all.
  if (notices.length === 0) return null;

  const peak = peakSeverity(notices) ?? 'info';

  return (
    <div
      className="fixed bottom-2 right-2 z-50 flex flex-col items-end gap-2"
      data-testid="notice-center">
      {open && (
        <div
          ref={panelRef}
          role="dialog"
          aria-label={t('notices.title')}
          data-testid="notice-panel"
          className="w-80 max-w-[calc(100vw-2rem)] overflow-hidden rounded-xl border border-line bg-surface shadow-lg">
          <div className="flex items-center justify-between border-b border-line-subtle px-3 py-2">
            <span className="text-sm font-semibold text-content">{t('notices.title')}</span>
            <Button
              iconOnly
              size="xs"
              variant="tertiary"
              data-testid="notice-close"
              analyticsId="notice-center-close"
              aria-label={t('common.close')}
              onClick={() => setOpen(false)}>
              <LuX className="h-3.5 w-3.5" />
            </Button>
          </div>
          <ul className="max-h-96 divide-y divide-line-subtle overflow-y-auto">
            {notices.map(notice => (
              <NoticeRow key={notice.id} notice={notice} />
            ))}
          </ul>
        </div>
      )}

      {/* Subtle by design: the app's own surface and hairline, not a coloured
          alert slab. Severity shows in the badge and the icon tint, which is
          enough to distinguish "look at this" from "this is broken" without
          the corner shouting on every render. */}
      <Button
        iconOnly
        variant="secondary"
        data-testid="notice-trigger"
        analyticsId="notice-center-toggle"
        aria-label={t('notices.title')}
        aria-expanded={open}
        title={t('notices.title')}
        onClick={() => setOpen(value => !value)}
        className="relative h-10 w-10 rounded-full border-line bg-surface text-content-muted shadow-md hover:text-content-secondary">
        <LuBell className="h-4 w-4" />
        <span
          data-testid="notice-badge"
          className={cn(
            'absolute -right-0.5 -top-0.5 min-w-[18px] rounded-full px-1 text-center text-[11px] font-bold leading-[18px]',
            SEVERITY_STYLE[peak].badge
          )}>
          {notices.length}
        </span>
      </Button>
    </div>
  );
}
