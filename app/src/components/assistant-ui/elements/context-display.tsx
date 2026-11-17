'use client';

/**
 * How full the model's context window is, as a ring / bar / text trigger with
 * the token breakdown in a tooltip.
 *
 * Vendored from the assistant-ui `context-display` registry item
 * (https://r.assistant-ui.com/styles/base-nova/context-display.json) — the
 * props-only file, not its `.aui` wrapper, which reads usage through
 * `@assistant-ui/ai-sdk` (not a dependency here). Changes from upstream:
 * - `cn` and tooltip import paths (`@/components/assistant-ui/...`).
 * - The "% full" caption and the Input / Cached input / Output / Reasoning
 *   row labels are a `labels` prop with English defaults, for `useT()`.
 * - The Ring preset forwards any other button props to its trigger, so the
 *   "Context usage" accessible name can be translated and it can itself be a
 *   popover trigger (`render={<ContextDisplayRing />}`). Only the Ring preset
 *   is used; the upstream Bar/Text presets were dropped as unused.
 * See `ContextUsage` in `features/conversations/aui/ContextUsage.tsx`, the
 * only caller.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/assistant-ui/ui/tooltip';
import { useT } from '@/lib/i18n/I18nContext';
import {
  type ComponentProps,
  createContext,
  type FC,
  type ReactNode,
  useContext,
  useMemo,
  useState,
} from 'react';

export type TokenUsage = {
  totalTokens?: number | undefined;
  inputTokens?: number | undefined;
  cachedInputTokens?: number | undefined;
  outputTokens?: number | undefined;
  reasoningTokens?: number | undefined;
};

export const formatTokenCount = (tokens: number): string => {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}mn`;
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(1).replace(/\.0$/, '')}k`;
  return `${tokens}`;
};

const getUsagePercent = (totalTokens: number | undefined, modelContextWindow: number): number => {
  if (!totalTokens) return 0;
  return Math.min((totalTokens / modelContextWindow) * 100, 100);
};

type UsageSeverity = 'normal' | 'warning' | 'critical';

const getUsageSeverity = (percent: number): UsageSeverity => {
  if (percent > 85) return 'critical';
  if (percent >= 65) return 'warning';
  return 'normal';
};

const getStrokeColor = (percent: number): string => {
  const severity = getUsageSeverity(percent);
  if (severity === 'critical') return 'stroke-red-500';
  if (severity === 'warning') return 'stroke-amber-500';
  return 'stroke-foreground';
};

const getBarColor = (percent: number): string => {
  const severity = getUsageSeverity(percent);
  if (severity === 'critical') return 'bg-red-500';
  if (severity === 'warning') return 'bg-amber-500';
  return 'bg-foreground';
};

const getPercentColor = (percent: number): string => {
  const severity = getUsageSeverity(percent);
  if (severity === 'critical') return 'text-red-500';
  if (severity === 'warning') return 'text-amber-500';
  return 'text-muted-foreground';
};
export type ContextDisplayLabels = {
  full: (percent: number) => string;
  input: string;
  cachedInput: string;
  output: string;
  reasoning: string;
};

const DEFAULT_LABELS: ContextDisplayLabels = {
  full: percent => `${percent}% full`,
  input: 'Input',
  cachedInput: 'Cached input',
  output: 'Output',
  reasoning: 'Reasoning',
};

type ContextDisplayContextValue = {
  usage: TokenUsage | undefined;
  totalTokens: number;
  percent: number;
  modelContextWindow: number;
  labels: ContextDisplayLabels;
};

const ContextDisplayContext = createContext<ContextDisplayContextValue | null>(null);

function useContextDisplay(): ContextDisplayContextValue {
  const ctx = useContext(ContextDisplayContext);
  if (!ctx) {
    throw new Error('ContextDisplay.* must be used within ContextDisplay.Root');
  }
  return ctx;
}
export type PresetProps = Omit<ComponentProps<'button'>, 'children' | 'className'> & {
  modelContextWindow: number;
  className?: string;
  side?: 'top' | 'bottom' | 'left' | 'right';
  usage?: TokenUsage | undefined;
  resetKey?: string | undefined;
  labels?: ContextDisplayLabels | undefined;
  showTooltip?: boolean;
};

export type ContextDisplayRootProps = {
  modelContextWindow: number;
  children: ReactNode;
  usage?: TokenUsage | undefined;
  resetKey?: string | undefined;
  labels?: ContextDisplayLabels | undefined;
};

function ContextDisplayRoot({
  modelContextWindow,
  children,
  usage,
  resetKey,
  labels = DEFAULT_LABELS,
}: ContextDisplayRootProps) {
  const rawTokens = usage?.totalTokens ?? 0;
  const [tokenState, setTokenState] = useState({
    resetKey,
    totalTokens: rawTokens > 0 ? rawTokens : 0,
    usage,
  });

  if (
    tokenState.resetKey !== resetKey ||
    (rawTokens > 0 && rawTokens !== tokenState.totalTokens) ||
    usage !== tokenState.usage
  ) {
    setTokenState(prev => {
      if (prev.resetKey !== resetKey) {
        return { resetKey, totalTokens: rawTokens > 0 ? rawTokens : 0, usage };
      }
      if (rawTokens > 0 && rawTokens !== prev.totalTokens) {
        return { ...prev, totalTokens: rawTokens, usage };
      }
      if (usage !== prev.usage) {
        return { ...prev, usage };
      }
      return prev;
    });
  }

  const current =
    tokenState.resetKey === resetKey
      ? tokenState
      : { totalTokens: rawTokens > 0 ? rawTokens : 0, usage };
  const totalTokens = current.totalTokens;
  const percent = getUsagePercent(totalTokens, modelContextWindow);
  const hasUsage = current.usage !== undefined || totalTokens > 0;

  const contextValue = useMemo(
    () => ({ usage: current.usage, totalTokens, percent, modelContextWindow, labels }),
    [current.usage, totalTokens, percent, modelContextWindow, labels]
  );

  if (!hasUsage) return null;

  return (
    <ContextDisplayContext.Provider value={contextValue}>
      <TooltipProvider>
        <Tooltip>{children}</Tooltip>
      </TooltipProvider>
    </ContextDisplayContext.Provider>
  );
}
function ContextDisplayTrigger({
  className,
  children,
  showTooltip = true,
  ...props
}: React.ComponentProps<'button'> & { showTooltip?: boolean }) {
  const trigger = (
    <button
      type="button"
      data-slot="context-display-trigger"
      className={cn('inline-flex items-center rounded-md transition-colors', className)}
      {...props}>
      {children}
    </button>
  );
  return showTooltip ? <TooltipTrigger render={trigger} /> : trigger;
}

type ContextSegment = { label: string; tokens: number };

// Whether a provider counts cached tokens inside inputTokens, or reasoning
// inside outputTokens, differs by provider: OpenAI reports cached_tokens as a
// subset of prompt_tokens, while Anthropic documents input_tokens as excluding
// cache_read_input_tokens. Nothing in the usage contract says which is in hand,
// so these are reported as the counts they are and none of them is given a
// share of the bar, which stays the one reading that always holds: the
// provider's own total against the window.
const getContextSegments = (
  usage: TokenUsage | undefined,
  labels: ContextDisplayLabels
): ContextSegment[] => {
  if (!usage) return [];
  return [
    { label: labels.input, tokens: usage.inputTokens ?? 0 },
    { label: labels.cachedInput, tokens: usage.cachedInputTokens ?? 0 },
    { label: labels.output, tokens: usage.outputTokens ?? 0 },
    { label: labels.reasoning, tokens: usage.reasoningTokens ?? 0 },
  ].filter(segment => segment.tokens > 0);
};

function ContextDisplayContent({
  side = 'top',
  className,
}: {
  side?: 'top' | 'bottom' | 'left' | 'right' | undefined;
  className?: string;
}) {
  const { usage, totalTokens, percent, modelContextWindow, labels } = useContextDisplay();
  const segments = getContextSegments(usage, labels);

  return (
    <TooltipContent
      side={side}
      sideOffset={8}
      data-slot="context-display-popover"
      className={cn(
        'bg-popover text-popover-foreground block w-56 border p-3 text-left [&_[data-slot=tooltip-arrow]]:hidden',
        className
      )}>
      <div className="text-xs">
        <div className="flex items-baseline justify-between gap-6 whitespace-nowrap">
          <span className={getPercentColor(percent)}>{labels.full(Math.round(percent))}</span>
          <span className="font-mono tabular-nums">
            {formatTokenCount(Math.min(totalTokens, modelContextWindow))} /{' '}
            {formatTokenCount(modelContextWindow)}
          </span>
        </div>
        <div className="bg-muted inset-ring-border mt-2.5 h-1 overflow-hidden rounded-full inset-ring forced-colors:border">
          <div
            className={cn(
              'h-full w-(--usage-width) rounded-full transition-[width] duration-300 forced-color-adjust-none',
              totalTokens > 0 && 'min-w-1',
              getBarColor(percent)
            )}
            style={{ '--usage-width': `${percent}%` } as React.CSSProperties}
          />
        </div>
        {segments.length > 0 && (
          <div className="mt-3 grid gap-1.5">
            {segments.map(segment => (
              <div key={segment.label} className="flex items-baseline justify-between gap-6">
                <span className="text-muted-foreground">{segment.label}</span>
                <span className="font-mono tabular-nums">{formatTokenCount(segment.tokens)}</span>
              </div>
            ))}
          </div>
        )}
      </div>
    </TooltipContent>
  );
}

const RING_SIZE = 18;
const RING_STROKE = 2.5;
const RING_RADIUS = (RING_SIZE - RING_STROKE) / 2;
const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS;

function RingVisual() {
  const { percent } = useContextDisplay();

  return (
    <svg
      aria-hidden="true"
      width={RING_SIZE}
      height={RING_SIZE}
      viewBox={`0 0 ${RING_SIZE} ${RING_SIZE}`}
      className="-rotate-90">
      <circle
        cx={RING_SIZE / 2}
        cy={RING_SIZE / 2}
        r={RING_RADIUS}
        fill="none"
        strokeWidth={RING_STROKE}
        className="stroke-muted"
      />
      <circle
        cx={RING_SIZE / 2}
        cy={RING_SIZE / 2}
        r={RING_RADIUS}
        fill="none"
        strokeWidth={RING_STROKE}
        strokeLinecap="round"
        strokeDasharray={RING_CIRCUMFERENCE}
        strokeDashoffset={RING_CIRCUMFERENCE - (percent / 100) * RING_CIRCUMFERENCE}
        className={cn(
          'transition-[stroke-dashoffset,stroke] duration-300',
          getStrokeColor(percent)
        )}
      />
    </svg>
  );
}

function RingPercentLabel() {
  const { percent } = useContextDisplay();
  return <span className="font-mono tabular-nums">{Math.round(percent)}%</span>;
}
const ContextDisplayRing: FC<PresetProps> = ({
  modelContextWindow,
  className,
  side,
  usage,
  resetKey,
  labels,
  showTooltip = true,
  ...triggerProps
}) => {
  const { t } = useT();
  return (
    <ContextDisplayRoot
      modelContextWindow={modelContextWindow}
      usage={usage}
      resetKey={resetKey}
      labels={labels}>
      <ContextDisplayTrigger
        showTooltip={showTooltip}
        className={cn(
          'text-muted-foreground hover:text-foreground gap-1.5 px-1.5 py-1 text-xs',
          className
        )}
        aria-label={t('conversations.composer.context.usage', 'Context usage')}
        {...triggerProps}>
        <RingVisual />
        <RingPercentLabel />
      </ContextDisplayTrigger>
      {showTooltip && <ContextDisplayContent side={side} />}
    </ContextDisplayRoot>
  );
};

export { ContextDisplayRing };
