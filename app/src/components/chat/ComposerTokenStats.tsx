import { useT } from '../../lib/i18n/I18nContext';
import { useAppSelector } from '../../store/hooks';

const DEFAULT_CONTEXT_WINDOW = 200_000;

function fmt(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0';
  if (n < 1000) return String(Math.round(n));
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}K`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

function ok(n: number): boolean {
  return Number.isFinite(n) && n > 0;
}

function dot() {
  return <span className="text-stone-300 dark:text-neutral-700">·</span>;
}

interface ComposerTokenStatsProps {
  /** Resolved model id, shown as the leading stat when present. */
  model?: string | null;
}

export default function ComposerTokenStats({ model }: ComposerTokenStatsProps = {}) {
  const { t } = useT();
  const usage = useAppSelector(state => state.chatRuntime.sessionTokenUsage);

  const inTok = usage.inputTokens || 0;
  const outTok = usage.outputTokens || 0;
  const turns = usage.turns || 0;
  const lastIn = usage.lastTurnInputTokens || 0;
  const lastOut = usage.lastTurnOutputTokens || 0;

  // Still render when only the model is known (no turns yet) so the resolved
  // model stays visible in the composer footer.
  if (turns === 0 && !model) return null;

  const showIn = ok(inTok);
  const showOut = ok(outTok);
  const contextUsed = lastIn + lastOut;
  const showCtx = ok(contextUsed);
  const contextPct = showCtx
    ? Math.min(100, Math.round((contextUsed / DEFAULT_CONTEXT_WINDOW) * 100))
    : 0;

  const parts: React.ReactNode[] = [];

  if (model) {
    parts.push(
      <span key="model" className="truncate" title={model}>
        {model}
      </span>
    );
  }
  if (showIn) {
    parts.push(
      <span key="in" title={t('token.inputTokens')}>
        {t('token.inLabel')} {fmt(inTok)}
      </span>
    );
  }
  if (showOut) {
    parts.push(
      <span key="out" title={t('token.outputTokens')}>
        {t('token.outLabel')} {fmt(outTok)}
      </span>
    );
  }
  if (turns > 0) {
    parts.push(
      <span key="turns" title={t('token.turnsCount')}>
        {turns} {turns === 1 ? t('token.turn') : t('token.turns')}
      </span>
    );
  }
  if (showCtx) {
    parts.push(
      <span key="ctx" title={t('token.contextWindow')}>
        {t('token.ctxLabel')} {contextPct}% ({fmt(contextUsed)}/{fmt(DEFAULT_CONTEXT_WINDOW)})
      </span>
    );
  }

  if (parts.length === 0) return null;

  return (
    <div className="flex min-w-0 flex-wrap items-center gap-2.5 text-[10px] font-mono text-stone-400 dark:text-neutral-500 select-none">
      {parts.map((part, i) => (
        <span key={i} className="contents">
          {i > 0 && dot()}
          {part}
        </span>
      ))}
    </div>
  );
}
