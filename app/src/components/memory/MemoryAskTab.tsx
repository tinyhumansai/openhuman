/**
 * Memory → Ask: put a question to memory and read the engine's answer with its
 * citations (`memory_recall`). The "Raw results" switch runs the same text as
 * a plain search instead (`memory_fetch`), with a mode picker limited to the
 * engine's `fetch_modes`, and lists every hit with its score.
 *
 * debug logging: DEBUG=openhuman:memory:ask
 */
import debug from 'debug';
import { type FormEvent, useState } from 'react';

import { BubbleMarkdown } from '../../features/conversations/components/AgentMessageBubble';
import { useT } from '../../lib/i18n/I18nContext';
import {
  type FetchMode,
  type Hit,
  memoryErrorMessage,
  memoryFetch,
  memoryRecall,
  type RecallAnswer,
} from '../../services/api/memoryApi';
import {
  Alert,
  AlertDescription,
  Button,
  Card,
  Label,
  NativeSelect,
  Switch,
  TextArea,
} from '../ui';
import { CenteredLoadingState } from '../ui/LoadingState';
import MemoryHitRow from './MemoryHitRow';

const log = debug('openhuman:memory:ask');

const RAW_LIMIT = 20;

interface MemoryAskTabProps {
  /** The fetch modes the active engine supports; empty hides the picker. */
  fetchModes: FetchMode[];
}

export default function MemoryAskTab({ fetchModes }: MemoryAskTabProps) {
  const { t } = useT();
  const [question, setQuestion] = useState('');
  const [raw, setRaw] = useState(false);
  const [mode, setMode] = useState<FetchMode | ''>(fetchModes[0] ?? '');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [answer, setAnswer] = useState<RecallAnswer | null>(null);
  const [hits, setHits] = useState<Hit[] | null>(null);

  const modeLabel = (m: FetchMode): string => {
    switch (m) {
      case 'keyword':
        return t('memoryPage.ask.modeKeyword');
      case 'vector':
        return t('memoryPage.ask.modeVector');
      case 'hybrid':
        return t('memoryPage.ask.modeHybrid');
      default:
        return m;
    }
  };

  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    const text = question.trim();
    if (!text || busy) return;
    setBusy(true);
    setError(null);
    try {
      if (raw) {
        log('fetch: mode=%s len=%d', mode || 'default', text.length);
        const page = await memoryFetch({ query: text, mode: mode || undefined, limit: RAW_LIMIT });
        setHits(page.hits ?? []);
      } else {
        log('recall: len=%d', text.length);
        const res = await memoryRecall({ question: text });
        setAnswer({ ...res, citations: res.citations ?? [] });
      }
    } catch (err) {
      log('%s failed: %o', raw ? 'fetch' : 'recall', err);
      setError(memoryErrorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-4 animate-fade-up" data-testid="memory-ask-tab">
      <Card padded divided={false}>
        <form className="flex flex-col gap-3" onSubmit={e => void submit(e)}>
          <Label htmlFor="memory-ask-question" className="text-xs text-content-secondary">
            {raw ? t('memoryPage.ask.queryLabel') : t('memoryPage.ask.questionLabel')}
          </Label>
          <TextArea
            id="memory-ask-question"
            data-testid="memory-ask-input"
            rows={3}
            value={question}
            placeholder={t('memoryPage.ask.placeholder')}
            onChange={e => setQuestion(e.target.value)}
            onKeyDown={e => {
              if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void submit();
            }}
          />
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div className="flex flex-wrap items-center gap-3">
              <div className="flex items-center gap-2">
                <Switch
                  id="memory-ask-raw"
                  data-testid="memory-ask-raw-toggle"
                  checked={raw}
                  onCheckedChange={setRaw}
                  aria-label={t('memoryPage.ask.rawToggle')}
                />
                <Label htmlFor="memory-ask-raw" className="text-xs text-content-secondary">
                  {t('memoryPage.ask.rawToggle')}
                </Label>
              </div>
              {raw && fetchModes.length > 0 && (
                <NativeSelect
                  aria-label={t('memoryPage.ask.modeLabel')}
                  data-testid="memory-ask-mode"
                  value={mode}
                  onChange={e => setMode(e.target.value as FetchMode)}>
                  {fetchModes.map(m => (
                    <option key={m} value={m}>
                      {modeLabel(m)}
                    </option>
                  ))}
                </NativeSelect>
              )}
            </div>
            <Button
              type="submit"
              variant="primary"
              size="sm"
              data-testid="memory-ask-submit"
              disabled={busy || question.trim().length === 0}>
              {raw ? t('memoryPage.ask.search') : t('memoryPage.ask.ask')}
            </Button>
          </div>
        </form>
      </Card>

      {error !== null && (
        <Alert variant="destructive" data-testid="memory-ask-error">
          <AlertDescription>{error}</AlertDescription>
        </Alert>
      )}

      {busy && <CenteredLoadingState label={t('memoryPage.ask.thinking')} />}

      {!busy && !raw && answer && (
        <Card title={t('memoryPage.ask.answerTitle')} data-testid="memory-ask-answer">
          <div className="px-4 py-3">
            <BubbleMarkdown content={answer.answer || t('memoryPage.ask.noAnswer')} />
          </div>
          {answer.citations.length > 0 && (
            <div>
              <h4 className="px-4 pt-3 text-[10px] font-semibold uppercase tracking-wide text-content-faint">
                {t('memoryPage.ask.citations')}
              </h4>
              <ul className="divide-y divide-line-subtle" data-testid="memory-ask-citations">
                {answer.citations.map(c => (
                  <MemoryHitRow
                    key={c.id}
                    id={c.id}
                    kind={c.kind}
                    text={c.snippet}
                    meta={c.meta}
                    score={c.score}
                    data-testid={`memory-citation-${c.id}`}
                  />
                ))}
              </ul>
            </div>
          )}
        </Card>
      )}

      {!busy && raw && hits && (
        <Card title={t('memoryPage.ask.rawTitle')} data-testid="memory-ask-hits">
          {hits.length === 0 ? (
            <p className="px-4 py-3 text-sm text-content-muted">{t('memoryPage.ask.noHits')}</p>
          ) : (
            <ul className="divide-y divide-line-subtle">
              {hits.map(h => (
                <MemoryHitRow
                  key={h.id}
                  id={h.id}
                  kind={h.kind}
                  text={h.text}
                  meta={h.meta}
                  score={h.score}
                  showScore
                />
              ))}
            </ul>
          )}
        </Card>
      )}
    </div>
  );
}
