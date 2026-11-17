/**
 * The composer's context-usage control: assistant-ui's context-display ring,
 * with assistant-ui's context-breakdown element in a popover behind it.
 *
 * The ring reads the thread's usage bucket (`chatRuntime.usageByThread`),
 * which `chat_done.usage` feeds — the last turn's orchestrator tokens against
 * the model's window. The live per-round `turn_cost` socket event is not
 * handled by the frontend yet, so the ring moves once per turn, not per round.
 *
 * The breakdown (`agent.context_breakdown`) is expensive on a cold core cache,
 * so it is fetched only when the popover opens, and an older core without the
 * method leaves the popover in an error state rather than breaking the
 * composer.
 *
 * `CostFooter` restores the dollar cost / per-sub-agent spend the bespoke
 * pre-vendoring widget used to show — `chatRuntime.usageByThread`'s
 * `costUsd`/`subAgents`, accumulated from `chat_done.usage` (and
 * `subagent_completed`'s late delta). The vendored `ContextBreakdown` element
 * has no cost field, so this renders underneath it as its own small card
 * rather than being folded into that element.
 */
import {
  ContextBreakdown,
  type ContextSegment,
} from '@/components/assistant-ui/elements/context-breakdown';
import {
  type ContextDisplayLabels,
  ContextDisplayRing,
  type TokenUsage,
} from '@/components/assistant-ui/elements/context-display';
import { ErrorState } from '@/components/assistant-ui/elements/error-state';
import { paper, ShimmerLabel } from '@/components/assistant-ui/elements/surfaces';
import { cn } from '@/components/assistant-ui/lib/utils';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/assistant-ui/ui/popover';
import debug from 'debug';
import { useCallback, useMemo, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type ContextBreakdown as ContextBreakdownData,
  getContextBreakdown,
} from '../../../services/api/agentContextApi';
import type { CostSource } from '../../../services/chatService';
import { emptySessionTokenUsage, type SessionTokenUsage } from '../../../store/chatRuntimeSlice';
import { useAppSelector } from '../../../store/hooks';

const log = debug('openhuman:context-usage');

/** Window assumed when neither the model nor any turn has reported one. */
const DEFAULT_CONTEXT_WINDOW = 200_000;

const EMPTY_USAGE = emptySessionTokenUsage();

const formatUsd = (usd: number): string => (usd >= 1 ? `${usd.toFixed(2)}$` : `${usd.toFixed(4)}$`);

/**
 * A cost as the core reported it: the provider's charge as is, a list-price
 * estimate marked `≈`, and nothing (`—`) when the cost is not known. An
 * unknown cost is never shown as a number: the core declines to guess one.
 */
export function formatCost(usd: number, source: CostSource): string {
  if (source === 'unknown') return '—';
  return source === 'estimated' ? `≈ ${formatUsd(usd)}` : formatUsd(usd);
}

type BreakdownState =
  | { status: 'idle' }
  | { status: 'loading' }
  | { status: 'ready'; data: ContextBreakdownData }
  | { status: 'error' };

/**
 * Map the core's sections onto the element's segments: translate the fixed
 * labels, group every rendered prompt heading into one System prompt bucket,
 * fold duplicate buckets into one row and drop empty sections.
 */
export function contextBreakdownSegments(
  data: ContextBreakdownData,
  t: (key: string) => string,
  usage: Pick<SessionTokenUsage, 'lastTurnContextUsed'> = EMPTY_USAGE
): readonly ContextSegment[] {
  let systemPrompt = 0;
  let toolSchemas = 0;
  let hiddenToolUsage = 0;
  for (const section of data.sections) {
    if (section.est_tokens <= 0) continue;
    const label = section.label.toLowerCase();
    if (label === 'tools' || label === 'tool_schemas' || label === 'tool schemas') {
      toolSchemas += section.est_tokens;
    } else if (label === 'tool_usage' || label === 'tool usage' || label === 'tool results') {
      hiddenToolUsage += section.est_tokens;
    } else if (label === 'thinking' || label === 'reasoning') {
      // Provider output currently folds these tokens into its output total.
      // Hide the separate row until the usage wire reports it independently.
    } else if (label !== 'history' && label !== 'input' && label !== 'output') {
      systemPrompt += section.est_tokens;
    }
  }
  // The turn-wide output total includes replies from earlier calls that may
  // have been trimmed or compacted out of the final request. Keep the final
  // context partition limited to its fixed prompt and remaining conversation.
  const yourInput = Math.max(
    0,
    usage.lastTurnContextUsed - systemPrompt - toolSchemas - hiddenToolUsage
  );
  return [
    {
      label: t('conversations.composer.context.section.preamble'),
      tokens: systemPrompt,
      tint: 'bg-blue-500',
    },
    {
      label: t('conversations.composer.context.section.toolSchemas'),
      tokens: toolSchemas,
      tint: 'bg-violet-500',
    },
    {
      label: t('conversations.composer.context.section.yourInput'),
      tokens: yourInput,
      tint: 'bg-amber-500',
    },
  ];
}

export function ContextUsage({
  threadId,
  modelContextWindow,
}: {
  threadId: string | null;
  /** The selected model's window; wins over the one the last turn reported. */
  modelContextWindow?: number | null;
}) {
  const { t } = useT();
  const usage = useAppSelector(state =>
    threadId ? (state.chatRuntime.usageByThread[threadId] ?? EMPTY_USAGE) : EMPTY_USAGE
  );
  const [open, setOpen] = useState(false);
  const [breakdown, setBreakdown] = useState<BreakdownState>({ status: 'idle' });
  // Only the newest request may land: reopening, or retrying, supersedes it.
  const requestSeq = useRef(0);

  const contextWindow =
    modelContextWindow && modelContextWindow > 0
      ? modelContextWindow
      : usage.contextWindow > 0
        ? usage.contextWindow
        : DEFAULT_CONTEXT_WINDOW;

  // The ring measures one request against the window. The turn's input and
  // output totals sum every model call of the turn, so on a long tool loop
  // they run to many times the window; they belong with the spend figures
  // below, not under the gauge.
  const ringUsage = useMemo<TokenUsage>(
    () => ({ totalTokens: usage.lastTurnContextUsed }),
    [usage.lastTurnContextUsed]
  );

  const labels = useMemo<ContextDisplayLabels>(
    () => ({
      full: percent =>
        t('conversations.composer.context.full').replace('{percent}', String(percent)),
      input: t('conversations.composer.context.input'),
      cachedInput: t('conversations.composer.context.cached'),
      output: t('conversations.composer.context.output'),
      reasoning: t('conversations.composer.context.reasoning'),
    }),
    [t]
  );

  const loadBreakdown = useCallback(() => {
    const seq = ++requestSeq.current;
    log('breakdown fetch start thread=%s seq=%d', threadId ?? '(none)', seq);
    setBreakdown({ status: 'loading' });
    getContextBreakdown(threadId).then(
      data => {
        if (seq !== requestSeq.current) return;
        log('breakdown fetch ok seq=%d sections=%d', seq, data.sections.length);
        setBreakdown({ status: 'ready', data });
      },
      (error: unknown) => {
        if (seq !== requestSeq.current) return;
        log('breakdown fetch failed seq=%d: %O', seq, error);
        setBreakdown({ status: 'error' });
      }
    );
  }, [threadId]);

  const handleOpenChange = useCallback(
    (next: boolean) => {
      setOpen(next);
      if (next) loadBreakdown();
    },
    [loadBreakdown]
  );

  let body;
  if (breakdown.status === 'ready') {
    const limit = breakdown.data.context_window > 0 ? breakdown.data.context_window : contextWindow;
    const cacheHit =
      usage.inputTokens > 0
        ? Math.min(100, Math.round((usage.cachedTokens / usage.inputTokens) * 100))
        : 0;
    const stats = [
      { label: t('token.popCacheHit'), value: `${cacheHit}%` },
      { label: `${t('token.costTitle')}:`, value: formatCost(usage.costUsd, usage.costSource) },
      ...Object.values(usage.subAgents).map(sub => ({
        label: t('conversations.composer.context.subagentCost').replace('{agent}', sub.agentId),
        value: `${(sub.inputTokens + sub.outputTokens).toLocaleString('en-US')} · ${formatCost(
          sub.costUsd,
          sub.costSource
        )}`,
      })),
    ];
    body = (
      <ContextBreakdown
        segments={contextBreakdownSegments(breakdown.data, t, usage)}
        limit={limit}
        title={t('conversations.composer.context.title')}
        headroomLabel={t('conversations.composer.context.headroom')}
        stats={stats}
        meterLabel={label =>
          t('conversations.composer.context.meterLabel').replace('{label}', label)
        }
        meterValueText={(used, max) =>
          t('conversations.composer.context.meterValue')
            .replace('{used}', used)
            .replace('{limit}', max)
        }
      />
    );
  } else if (breakdown.status === 'error') {
    body = (
      <div className={cn(paper, 'w-72 rounded-2xl p-4')}>
        <ErrorState
          title={t('conversations.composer.context.errorTitle')}
          detail={t('conversations.composer.context.errorDetail')}
          retrying={false}
          onRetry={loadBreakdown}
          retryLabel={t('common.retry')}
        />
      </div>
    );
  } else {
    body = (
      <div className={cn(paper, 'w-72 rounded-2xl p-4')}>
        <ShimmerLabel className="text-foreground/55 text-sm">
          {t('conversations.composer.context.loading')}
        </ShimmerLabel>
      </div>
    );
  }

  return (
    <Popover open={open} onOpenChange={handleOpenChange}>
      <PopoverTrigger
        openOnHover
        delay={0}
        closeDelay={120}
        render={
          <ContextDisplayRing
            showTooltip={false}
            data-testid="composer-context-usage"
            aria-label={t('conversations.composer.context.usage')}
            modelContextWindow={contextWindow}
            usage={ringUsage}
            resetKey={threadId ?? undefined}
            labels={labels}
          />
        }
      />
      <PopoverContent
        data-testid="composer-token-breakdown"
        side="top"
        align="start"
        className="w-auto bg-transparent p-0 shadow-none ring-0">
        {body}
      </PopoverContent>
    </Popover>
  );
}

export default ContextUsage;
