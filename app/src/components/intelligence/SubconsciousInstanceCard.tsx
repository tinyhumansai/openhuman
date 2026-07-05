import type { ReactNode } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { SubconsciousInstanceStatus } from '../../utils/tauriCommands/subconscious';
import Button from '../ui/Button';

interface SubconsciousInstanceCardProps {
  title: string;
  subtitle: string;
  /** The status row for this world, or undefined when the core hasn't
   * registered it (older core / disabled / not bootstrapped). */
  status: SubconsciousInstanceStatus | undefined;
  /** Shown as a disabled state with `disabledHint` when true. */
  disabled?: boolean;
  disabledHint?: string;
  runLabel: string;
  triggering: boolean;
  onRun: () => void;
  /** Navigate to the provider (LLM) settings from the unavailable banner. */
  onProviderSettings?: () => void;
  /** Optional footer, e.g. a "View directives →" cross-link. */
  footer?: ReactNode;
}

/**
 * One subconscious world's health card — total ticks, last tick, failures, a
 * provider-unavailable banner, and a per-world "Run now". Shared so a third
 * world later is a data change, not new JSX.
 */
export default function SubconsciousInstanceCard({
  title,
  subtitle,
  status,
  disabled = false,
  disabledHint,
  runLabel,
  triggering,
  onRun,
  onProviderSettings,
  footer,
}: SubconsciousInstanceCardProps) {
  const { t } = useT();
  const providerUnavailable = status?.provider_available === false;
  const providerUnavailableReason = providerUnavailable
    ? (status?.provider_unavailable_reason ?? t('subconscious.providerUnavailableTitle'))
    : null;

  return (
    <div className="rounded-lg border border-line bg-surface p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h4 className="text-sm font-semibold text-content">{title}</h4>
          <p className="mt-0.5 text-xs text-content-muted">{subtitle}</p>
        </div>
        <span
          className={`flex-shrink-0 rounded-full px-2 py-0.5 text-[10px] font-medium ${
            disabled
              ? 'bg-surface-strong text-content-faint'
              : 'bg-sage-50 text-sage-700 dark:bg-sage-500/10 dark:text-sage-300'
          }`}>
          {disabled ? t('subconscious.instance.off') : t('subconscious.instance.on')}
        </span>
      </div>

      {disabled ? (
        <p className="mt-3 text-xs text-content-muted">{disabledHint}</p>
      ) : (
        <>
          <div className="mt-3 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-content-faint">
            <span>
              {status?.total_ticks ?? 0} {t('subconscious.ticks')}
            </span>
            {status?.last_tick_at != null && (
              <>
                <span className="text-content-faint dark:text-neutral-600">|</span>
                <span>
                  {t('subconscious.last')}:{' '}
                  {new Date(status.last_tick_at * 1000).toLocaleTimeString()}
                </span>
              </>
            )}
            {(status?.consecutive_failures ?? 0) > 0 && (
              <>
                <span className="text-content-faint dark:text-neutral-600">|</span>
                <span className="text-coral-500">
                  {status?.consecutive_failures} {t('subconscious.failed')}
                </span>
              </>
            )}
          </div>

          {providerUnavailable && (
            <div className="mt-3 rounded-lg border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 p-3">
              <div className="flex items-start justify-between gap-3">
                <p className="min-w-0 text-xs text-amber-700 dark:text-amber-300 break-words">
                  {providerUnavailableReason}
                </p>
                {onProviderSettings && (
                  <button
                    type="button"
                    onClick={onProviderSettings}
                    className="flex-shrink-0 rounded-md bg-amber-600 px-2.5 py-1.5 text-xs font-medium text-content-inverted hover:bg-amber-700 transition-colors">
                    {t('subconscious.providerSettings')}
                  </button>
                )}
              </div>
            </div>
          )}

          <div className="mt-3 flex items-center justify-between gap-2">
            <div className="min-w-0 text-xs text-content-faint">{footer}</div>
            <Button
              variant="secondary"
              size="sm"
              onClick={onRun}
              disabled={triggering || providerUnavailable}
              title={providerUnavailable ? t('subconscious.providerUnavailableTitle') : undefined}>
              {triggering ? (
                <div className="w-3 h-3 border border-stone-400 border-t-transparent rounded-full animate-spin" />
              ) : (
                <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M13 10V3L4 14h7v7l9-11h-7z"
                  />
                </svg>
              )}
              {runLabel}
            </Button>
          </div>
        </>
      )}
    </div>
  );
}
