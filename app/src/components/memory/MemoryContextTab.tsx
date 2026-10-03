/**
 * Memory → Context: the compiled `context.md` brief a new session starts with.
 * Renders the markdown, when it was generated and how many tokens it costs,
 * regenerates it on demand (`memory_context_refresh`), and edits the schedule
 * and budget (`memory_context_set`).
 *
 * debug logging: DEBUG=openhuman:memory:context
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';
import { LuRefreshCw } from 'react-icons/lu';

import { BubbleMarkdown } from '../../features/conversations/components/AgentMessageBubble';
import { useT } from '../../lib/i18n/I18nContext';
import {
  type ContextUpdate,
  type MemoryContext,
  memoryContextGet,
  memoryContextRefresh,
  memoryContextSet,
  memoryErrorMessage,
} from '../../services/api/memoryApi';
import { Alert, AlertDescription, Button, Card, NumberField, Switch } from '../ui';
import { CenteredLoadingState } from '../ui/LoadingState';
import { fill, formatTimestamp, parsePositiveInt } from './memoryFormat';

const log = debug('openhuman:memory:context');

export default function MemoryContextTab() {
  const { t } = useT();
  const [ctx, setCtx] = useState<MemoryContext | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [saving, setSaving] = useState(false);
  const [interval, setIntervalValue] = useState('');
  const [budget, setBudget] = useState('');

  const apply = useCallback((next: MemoryContext) => {
    setCtx(next);
    setIntervalValue(String(next.interval_mins));
    setBudget(String(next.budget_tokens));
  }, []);

  useEffect(() => {
    let cancelled = false;
    memoryContextGet()
      .then(next => {
        if (!cancelled) apply(next);
      })
      .catch(err => {
        if (cancelled) return;
        log('get failed: %o', err);
        setError(memoryErrorMessage(err));
      });
    return () => {
      cancelled = true;
    };
  }, [apply]);

  const regenerate = async () => {
    setRefreshing(true);
    setError(null);
    try {
      const next = await memoryContextRefresh();
      log('refreshed: tokens=%d', next.tokens);
      apply(next);
    } catch (err) {
      log('refresh failed: %o', err);
      setError(memoryErrorMessage(err));
    } finally {
      setRefreshing(false);
    }
  };

  const save = async (update: ContextUpdate) => {
    setSaving(true);
    setError(null);
    try {
      log('set: %o', update);
      apply(await memoryContextSet(update));
    } catch (err) {
      log('set failed: %o', err);
      setError(memoryErrorMessage(err));
    } finally {
      setSaving(false);
    }
  };

  const commitNumber = (field: 'interval_mins' | 'budget_tokens', raw: string) => {
    if (!ctx) return;
    const value = parsePositiveInt(raw);
    if (value === null) {
      if (field === 'interval_mins') setIntervalValue(String(ctx.interval_mins));
      else setBudget(String(ctx.budget_tokens));
      return;
    }
    if (value === ctx[field]) return;
    void save({ [field]: value });
  };

  if (ctx === null) {
    return error !== null ? (
      <Alert variant="destructive" data-testid="memory-context-error">
        <AlertDescription>{error}</AlertDescription>
      </Alert>
    ) : (
      <CenteredLoadingState label={t('memoryPage.loading')} />
    );
  }

  const generatedAt = formatTimestamp(ctx.generated_at);

  return (
    <div className="space-y-4 animate-fade-up" data-testid="memory-context-tab">
      {error !== null && (
        <Alert variant="destructive" data-testid="memory-context-error">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}

      <Card
        title={t('memoryPage.context.briefTitle')}
        description={
          generatedAt
            ? fill(t('memoryPage.context.generatedAt'), { when: generatedAt, tokens: ctx.tokens })
            : t('memoryPage.context.neverGenerated')
        }
        headerRight={
          <Button
            type="button"
            variant="secondary"
            size="sm"
            data-testid="memory-context-regenerate"
            disabled={refreshing}
            onClick={() => void regenerate()}>
            <LuRefreshCw className="h-3.5 w-3.5" aria-hidden />
            {refreshing ? t('memoryPage.context.regenerating') : t('memoryPage.context.regenerate')}
          </Button>
        }
        data-testid="memory-context-brief">
        <div className="px-4 py-3" data-testid="memory-context-markdown">
          {ctx.markdown.trim() ? (
            <BubbleMarkdown content={ctx.markdown} />
          ) : (
            <p className="text-sm text-content-muted">{t('memoryPage.context.empty')}</p>
          )}
        </div>
      </Card>

      <Card
        title={t('memoryPage.context.settingsTitle')}
        description={t('memoryPage.context.settingsDescription')}>
        <div className="flex items-center justify-between gap-4 px-4 py-3">
          <label htmlFor="memory-context-enabled" className="text-sm text-content">
            {t('memoryPage.context.enabled')}
          </label>
          <Switch
            id="memory-context-enabled"
            data-testid="memory-context-enabled"
            checked={ctx.enabled}
            disabled={saving}
            aria-label={t('memoryPage.context.enabled')}
            onCheckedChange={next => void save({ enabled: next })}
          />
        </div>
        <div className="flex items-center justify-between gap-4 px-4 py-3">
          <p className="text-sm text-content">{t('memoryPage.context.interval')}</p>
          <NumberField
            id="memory-context-interval"
            data-testid="memory-context-interval"
            aria-label={t('memoryPage.context.interval')}
            value={interval}
            min={1}
            unit={t('memoryPage.context.minutes')}
            disabled={saving || !ctx.enabled}
            onChange={setIntervalValue}
            onCommit={() => commitNumber('interval_mins', interval)}
          />
        </div>
        <div className="flex items-center justify-between gap-4 px-4 py-3">
          <p className="text-sm text-content">{t('memoryPage.context.budget')}</p>
          <NumberField
            id="memory-context-budget"
            data-testid="memory-context-budget"
            aria-label={t('memoryPage.context.budget')}
            value={budget}
            min={1}
            unit={t('memoryPage.context.tokens')}
            disabled={saving || !ctx.enabled}
            onChange={setBudget}
            onCommit={() => commitNumber('budget_tokens', budget)}
          />
        </div>
      </Card>
    </div>
  );
}
