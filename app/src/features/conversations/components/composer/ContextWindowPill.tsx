import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/assistant-ui/ui/tooltip';
import { GaugeIcon } from 'lucide-react';

import { useT } from '../../../../lib/i18n/I18nContext';
import type { SessionTokenUsage } from '../../../../store/chatRuntimeSlice';

/**
 * Token accounting for the current thread.
 *
 * Every field is what the *provider* reports, not a local estimate, which is
 * why `cachedInput` is tracked apart from `input`: a cache read is billed at a
 * fraction of a fresh read, so a turn's cost cannot be derived from a single
 * token count.
 */
export type ContextUsage = {
  /** Tokens currently occupying the window. */
  used: number;
  /** Size of the window for the selected model. */
  limit: number;
  input: number;
  cachedInput: number;
  output: number;
  /** Accumulated spend for the thread, in USD. */
  costUsd: number;
};

/** Map the authoritative Redux token bucket onto the compact meter shape. */
export function contextUsageFromTokenUsage(
  usage: SessionTokenUsage,
  selectedModelContextWindow?: number | null
): ContextUsage {
  const cachedInput = Math.max(0, usage.cachedTokens || 0);
  return {
    used: Math.max(0, usage.lastTurnContextUsed || 0),
    limit:
      selectedModelContextWindow === undefined
        ? Math.max(0, usage.contextWindow || 0)
        : Math.max(0, selectedModelContextWindow ?? 0),
    // `inputTokens` includes provider-reported cache reads. Keep fresh input
    // and cached input in distinct rows instead of counting cache hits twice.
    input: Math.max(0, (usage.inputTokens || 0) - cachedInput),
    cachedInput,
    output: Math.max(0, usage.outputTokens || 0),
    costUsd: Math.max(0, usage.costUsd || 0),
  };
}

const compact = (n: number): string =>
  n >= 1000 ? `${(n / 1000).toFixed(n >= 10_000 ? 0 : 1)}k` : String(n);

const Row = ({ label, value }: { label: string; value: string }) => (
  <div className="flex items-baseline justify-between gap-6">
    <span className="text-muted-foreground">{label}</span>
    <span className="tabular-nums">{value}</span>
  </div>
);

/**
 * How full the context window is, with the cost and token breakdown behind a
 * hover.
 *
 * The pill shows the one number that changes a decision in the moment — how
 * much room is left — and keeps the accounting out of the way until asked for.
 * A bar rather than a percentage because the useful reading is "am I near the
 * end", not the exact figure.
 */
export function ContextWindowPill({ usage }: { usage: ContextUsage }) {
  const { t } = useT();
  const limitKnown = usage.limit > 0;
  const ratio = limitKnown ? Math.min(1, usage.used / usage.limit) : 0;
  // Amber past three quarters, red past nine tenths: the points where the next
  // long turn starts being at risk of truncation.
  const tone = ratio > 0.9 ? 'bg-coral-500' : ratio > 0.75 ? 'bg-amber-500' : 'bg-primary-500';

  return (
    <TooltipProvider>
      <Tooltip>
        <TooltipTrigger
          render={
            <button
              type="button"
              aria-label={t('conversations.composer.context.title')}
              className="text-muted-foreground hover:text-foreground hover:bg-muted flex h-7 shrink-0 items-center gap-1.5 rounded-full px-2.5 text-xs transition-colors">
              <GaugeIcon className="size-3.5" />
              <span className="tabular-nums">
                {compact(usage.used)}/{limitKnown ? compact(usage.limit) : '—'}
              </span>
              <span className="bg-muted h-1 w-8 overflow-hidden rounded-full">
                <span
                  className={`block h-full rounded-full ${tone}`}
                  style={{ width: limitKnown ? `${Math.max(2, ratio * 100)}%` : '0%' }}
                />
              </span>
            </button>
          }
        />
        {/* The vendored `TooltipContent` is an `inline-flex` row with `items-center`,
            which would sit the heading beside the rows instead of above them. */}
        <TooltipContent side="top" className="min-w-52 flex-col items-stretch gap-0 p-2.5 text-xs">
          <p className="mb-1.5 font-medium">{t('conversations.composer.context.title')}</p>
          <div className="flex flex-col gap-1">
            <Row label={t('conversations.composer.context.input')} value={compact(usage.input)} />
            <Row
              label={t('conversations.composer.context.cached')}
              value={compact(usage.cachedInput)}
            />
            <Row label={t('conversations.composer.context.output')} value={compact(usage.output)} />
            <Row
              label={t('conversations.composer.context.cost')}
              value={`$${usage.costUsd.toFixed(3)}`}
            />
          </div>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

export default ContextWindowPill;
