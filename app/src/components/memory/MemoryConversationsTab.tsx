/**
 * Memory → Conversations: whether chat threads are stored automatically, and
 * when (`batch_turns` committed turns, or `idle_secs` of quiet), plus the
 * threads stored most recently. Backed by `memory_conversations_get/_set`.
 *
 * debug logging: DEBUG=openhuman:memory:conversations
 */
import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  type ConversationsSettings,
  type ConversationsUpdate,
  memoryConversationsGet,
  memoryConversationsSet,
  memoryErrorMessage,
} from '../../services/api/memoryApi';
import { Alert, AlertDescription, Card, NumberField, Switch } from '../ui';
import { CenteredLoadingState } from '../ui/LoadingState';
import { fill, formatTimestamp, parsePositiveInt } from './memoryFormat';

const log = debug('openhuman:memory:conversations');

export default function MemoryConversationsTab() {
  const { t } = useT();
  const [settings, setSettings] = useState<ConversationsSettings | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [batch, setBatch] = useState('');
  const [idle, setIdle] = useState('');

  const apply = useCallback((next: ConversationsSettings) => {
    setSettings({ ...next, recent: next.recent ?? [] });
    setBatch(String(next.batch_turns));
    setIdle(String(next.idle_secs));
  }, []);

  useEffect(() => {
    let cancelled = false;
    memoryConversationsGet()
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

  const save = async (update: ConversationsUpdate) => {
    setSaving(true);
    setError(null);
    try {
      log('set: %o', update);
      apply(await memoryConversationsSet(update));
    } catch (err) {
      log('set failed: %o', err);
      setError(memoryErrorMessage(err));
    } finally {
      setSaving(false);
    }
  };

  const commitNumber = (field: 'batch_turns' | 'idle_secs', raw: string) => {
    if (!settings) return;
    const value = parsePositiveInt(raw);
    if (value === null) {
      // Revert an invalid entry to the stored value instead of saving it.
      if (field === 'batch_turns') setBatch(String(settings.batch_turns));
      else setIdle(String(settings.idle_secs));
      return;
    }
    if (value === settings[field]) return;
    void save({ [field]: value });
  };

  if (settings === null) {
    return error !== null ? (
      <Alert variant="destructive" data-testid="memory-conversations-error">
        <AlertDescription>{error}</AlertDescription>
      </Alert>
    ) : (
      <CenteredLoadingState label={t('memoryPage.loading')} />
    );
  }

  return (
    <div className="space-y-4 animate-fade-up" data-testid="memory-conversations-tab">
      {error !== null && (
        <Alert variant="destructive" data-testid="memory-conversations-error">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}

      <Card
        title={t('memoryPage.conversations.settingsTitle')}
        description={t('memoryPage.conversations.settingsDescription')}>
        <div className="flex items-center justify-between gap-4 px-4 py-3">
          <label htmlFor="memory-conversations-enabled" className="text-sm text-content">
            {t('memoryPage.conversations.enabled')}
          </label>
          <Switch
            id="memory-conversations-enabled"
            data-testid="memory-conversations-enabled"
            checked={settings.enabled}
            disabled={saving}
            aria-label={t('memoryPage.conversations.enabled')}
            onCheckedChange={next => void save({ enabled: next })}
          />
        </div>
        <div className="flex items-center justify-between gap-4 px-4 py-3">
          <div className="min-w-0">
            <p className="text-sm text-content">{t('memoryPage.conversations.batchTurns')}</p>
            <p className="text-xs text-content-muted">
              {t('memoryPage.conversations.batchTurnsHelp')}
            </p>
          </div>
          <NumberField
            id="memory-conversations-batch"
            data-testid="memory-conversations-batch"
            aria-label={t('memoryPage.conversations.batchTurns')}
            value={batch}
            min={1}
            disabled={saving || !settings.enabled}
            onChange={setBatch}
            onCommit={() => commitNumber('batch_turns', batch)}
          />
        </div>
        <div className="flex items-center justify-between gap-4 px-4 py-3">
          <div className="min-w-0">
            <p className="text-sm text-content">{t('memoryPage.conversations.idleSecs')}</p>
            <p className="text-xs text-content-muted">
              {t('memoryPage.conversations.idleSecsHelp')}
            </p>
          </div>
          <NumberField
            id="memory-conversations-idle"
            data-testid="memory-conversations-idle"
            aria-label={t('memoryPage.conversations.idleSecs')}
            value={idle}
            min={1}
            unit={t('memoryPage.conversations.seconds')}
            disabled={saving || !settings.enabled}
            onChange={setIdle}
            onCommit={() => commitNumber('idle_secs', idle)}
          />
        </div>
      </Card>

      <Card
        title={t('memoryPage.conversations.recentTitle')}
        data-testid="memory-conversations-recent">
        {settings.recent.length === 0 ? (
          <p
            className="px-4 py-3 text-sm text-content-muted"
            data-testid="memory-conversations-empty">
            {t('memoryPage.conversations.recentEmpty')}
          </p>
        ) : (
          <ul className="divide-y divide-line-subtle">
            {settings.recent.map(entry => (
              <li
                key={`${entry.thread_id}-${entry.stored_at}`}
                className="flex items-center justify-between gap-3 px-4 py-2.5"
                data-testid={`memory-conversation-${entry.thread_id}`}>
                <span className="truncate font-mono text-xs text-content">{entry.thread_id}</span>
                <span className="shrink-0 text-xs text-content-muted">
                  {fill(t('memoryPage.conversations.turns'), { count: entry.turns })}
                  {formatTimestamp(entry.stored_at) ? ` · ${formatTimestamp(entry.stored_at)}` : ''}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Card>
    </div>
  );
}
