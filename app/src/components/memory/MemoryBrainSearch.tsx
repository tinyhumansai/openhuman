/**
 * Memory → Brain → Search: search the shared brain's documents
 * (`memory_brain_search`), optionally within one source type.
 *
 * debug logging: DEBUG=openhuman:memory:brain
 */
import debug from 'debug';
import { type FormEvent, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { type Hit, memoryBrainSearch, memoryErrorMessage } from '../../services/api/memoryApi';
import { Button, Card, NativeSelect, TextField } from '../ui';
import { CenteredLoadingState } from '../ui/LoadingState';
import MemoryErrorAlert from './MemoryErrorAlert';
import MemoryHitRow from './MemoryHitRow';
import { brainSourceLabel } from './memoryLifecycleLabels';

const log = debug('openhuman:memory:brain');

const SEARCH_LIMIT = 20;

interface MemoryBrainSearchProps {
  /** The source types the brain holds, for the filter. */
  sources: string[];
}

export default function MemoryBrainSearch({ sources }: MemoryBrainSearchProps) {
  const { t } = useT();
  const [query, setQuery] = useState('');
  const [source, setSource] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [hits, setHits] = useState<Hit[] | null>(null);

  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    const text = query.trim();
    if (!text || busy) return;
    setBusy(true);
    setError(null);
    try {
      log('search: source=%s len=%d', source || 'all', text.length);
      const res = await memoryBrainSearch({
        query: text,
        source: source || undefined,
        limit: SEARCH_LIMIT,
      });
      setHits(res.hits ?? []);
    } catch (err) {
      log('search failed: %o', err);
      setError(memoryErrorMessage(err, t));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Card
      title={t('memoryPage.brain.searchTitle')}
      description={t('memoryPage.brain.searchDescription')}
      data-testid="memory-brain-search">
      <form
        className="flex flex-wrap items-center gap-2 px-4 py-3"
        onSubmit={event => void submit(event)}>
        <div className="min-w-0 flex-1">
          <TextField
            aria-label={t('memoryPage.brain.searchLabel')}
            data-testid="memory-brain-search-input"
            value={query}
            placeholder={t('memoryPage.brain.searchPlaceholder')}
            onChange={e => setQuery(e.target.value)}
          />
        </div>
        <NativeSelect
          aria-label={t('memoryPage.brain.searchSource')}
          data-testid="memory-brain-search-source"
          value={source}
          onChange={e => setSource(e.target.value)}>
          <option value="">{t('memoryPage.brain.allSources')}</option>
          {sources.map(id => (
            <option key={id} value={id}>
              {brainSourceLabel(id, t)}
            </option>
          ))}
        </NativeSelect>
        <Button
          type="submit"
          variant="secondary"
          size="sm"
          analyticsId="memory-brain-search"
          data-testid="memory-brain-search-submit"
          disabled={busy || query.trim().length === 0}>
          {t('memoryPage.ask.search')}
        </Button>
      </form>

      {error !== null && (
        <div className="px-4 pb-3">
          <MemoryErrorAlert message={error} data-testid="memory-brain-search-error" />
        </div>
      )}

      {busy && <CenteredLoadingState label={t('memoryPage.ask.thinking')} />}

      {!busy &&
        hits !== null &&
        (hits.length === 0 ? (
          <p
            className="px-4 py-3 text-sm text-content-muted"
            data-testid="memory-brain-search-empty">
            {t('memoryPage.brain.searchEmpty')}
          </p>
        ) : (
          <ul className="divide-y divide-line-subtle" data-testid="memory-brain-search-hits">
            {hits.map(hit => (
              <MemoryHitRow
                key={hit.id}
                id={hit.id}
                kind={hit.kind}
                text={hit.text}
                meta={hit.meta}
                score={hit.score}
                showScore
              />
            ))}
          </ul>
        ))}
    </Card>
  );
}
