/**
 * Memory — the Memory v2 surface (docs/specs/memory-v2.md).
 *
 * Rendered as the Memory sub-page of Connections (`/connections?tab=brain`);
 * it has no route of its own. Connections owns `?tab=` and the sidebar; this
 * page keeps its chip in `?brain=`:
 *
 *   engine · ask · learnings · conversations · documents · context
 *
 * With no `?brain=` the page opens on Ask when an engine is active and on
 * Engine otherwise. Old v1 values (`graph`, `goals`, `sources`, `sync`,
 * `history`) are rewritten to their v2 chip. While memory is off every chip
 * but Engine shows an empty state that points there.
 *
 * debug logging: DEBUG=openhuman:memory
 */
import debug from 'debug';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import MemoryAskTab from '../components/memory/MemoryAskTab';
import { type MemoryChip, resolveMemoryChip } from '../components/memory/memoryChips';
import MemoryContextTab from '../components/memory/MemoryContextTab';
import MemoryConversationsTab from '../components/memory/MemoryConversationsTab';
import MemoryDocumentsTab from '../components/memory/MemoryDocumentsTab';
import MemoryEngineTab from '../components/memory/MemoryEngineTab';
import MemoryImportBanner from '../components/memory/MemoryImportBanner';
import MemoryLearningsTab from '../components/memory/MemoryLearningsTab';
import MemoryOffState from '../components/memory/MemoryOffState';
import SettingsTabbedPage from '../components/settings/layout/SettingsTabbedPage';
import { Alert, AlertDescription, Button } from '../components/ui';
import { CenteredLoadingState } from '../components/ui/LoadingState';
import { useT } from '../lib/i18n/I18nContext';
import { useCoreState } from '../providers/CoreStateProvider';
import {
  type EngineDescriptor,
  type EngineState,
  isMemoryOn,
  memoryEngineGet,
  memoryEnginesList,
  memoryErrorMessage,
} from '../services/api/memoryApi';

const log = debug('openhuman:memory');

export default function Memory() {
  const { t } = useT();
  const location = useLocation();
  const navigate = useNavigate();
  const { snapshot } = useCoreState();
  const authUserId = snapshot.auth.userId;

  const [engine, setEngine] = useState<EngineState | null>(null);
  const [engines, setEngines] = useState<EngineDescriptor[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [reloadKey, setReloadKey] = useState(0);

  // The engine depends on who is signed in (hosted memory needs an account),
  // so an identity change re-reads it.
  useEffect(() => {
    let cancelled = false;
    Promise.allSettled([memoryEngineGet(), memoryEnginesList()]).then(([state, list]) => {
      if (cancelled) return;
      if (state.status === 'fulfilled') {
        log('engine: %s status=%s', state.value.engine ?? 'none', state.value.status);
        setEngine(state.value);
        setLoadError(null);
      } else {
        log('engine_get failed: %o', state.reason);
        setLoadError(memoryErrorMessage(state.reason));
        // Treat an unreadable engine as off so the page still has a chip to show.
        setEngine({ engine: null, has_key: false, status: 'off', fetch_modes: [] });
      }
      if (list.status === 'fulfilled') setEngines(list.value.engines ?? []);
    });
    return () => {
      cancelled = true;
    };
  }, [authUserId, reloadKey]);

  const params = useMemo(() => new URLSearchParams(location.search), [location.search]);
  const rawChip = params.get('brain');
  const requested = resolveMemoryChip(rawChip);
  const on = isMemoryOn(engine);
  const chip: MemoryChip | null = requested ?? (engine ? (on ? 'ask' : 'engine') : null);

  const setChip = useCallback(
    (next: MemoryChip, replace = false) => {
      const nextParams = new URLSearchParams(location.search);
      nextParams.set('brain', next);
      // v1's Sync tab kept its sub-view in `?view=`; it has no meaning now.
      nextParams.delete('view');
      navigate({ pathname: location.pathname, search: `?${nextParams.toString()}` }, { replace });
    },
    [location.pathname, location.search, navigate]
  );

  // Rewrite a legacy (or unknown) `?brain=` value to its canonical chip so the
  // address bar, history and analytics all see the v2 name.
  useEffect(() => {
    if (rawChip === null) return;
    if (requested && requested !== rawChip) setChip(requested, true);
  }, [rawChip, requested, setChip]);

  const headers: Record<MemoryChip, { title: string; description: string }> = {
    engine: { title: t('memoryPage.tabs.engine'), description: t('memoryPage.header.engine') },
    ask: { title: t('memoryPage.tabs.ask'), description: t('memoryPage.header.ask') },
    learnings: {
      title: t('memoryPage.tabs.learnings'),
      description: t('memoryPage.header.learnings'),
    },
    conversations: {
      title: t('memoryPage.tabs.conversations'),
      description: t('memoryPage.header.conversations'),
    },
    documents: {
      title: t('memoryPage.tabs.documents'),
      description: t('memoryPage.header.documents'),
    },
    context: { title: t('memoryPage.tabs.context'), description: t('memoryPage.header.context') },
  };

  const activeLabel =
    engines.find(e => e.id === engine?.engine)?.label ?? engine?.engine ?? t('nav.brain');

  const body = (() => {
    if (chip === null || engine === null) {
      return <CenteredLoadingState label={t('memoryPage.loading')} />;
    }
    if (chip === 'engine') {
      return <MemoryEngineTab state={engine} onStateChange={setEngine} />;
    }
    if (!on) {
      return <MemoryOffState reason={engine.reason} onOpenEngine={() => setChip('engine')} />;
    }
    switch (chip) {
      case 'ask':
        return <MemoryAskTab fetchModes={engine.fetch_modes ?? []} />;
      case 'learnings':
        return <MemoryLearningsTab />;
      case 'conversations':
        return <MemoryConversationsTab />;
      case 'documents':
        return <MemoryDocumentsTab />;
      case 'context':
        return <MemoryContextTab />;
      default:
        return null;
    }
  })();

  return (
    <div className="h-full w-full" data-testid="memory-page">
      <SettingsTabbedPage<MemoryChip>
        title={headers[chip ?? 'engine'].title}
        description={headers[chip ?? 'engine'].description}
        tabs={[
          { id: 'engine', label: t('memoryPage.tabs.engine') },
          { id: 'ask', label: t('memoryPage.tabs.ask') },
          { id: 'learnings', label: t('memoryPage.tabs.learnings') },
          { id: 'conversations', label: t('memoryPage.tabs.conversations') },
          { id: 'documents', label: t('memoryPage.tabs.documents') },
          { id: 'context', label: t('memoryPage.tabs.context') },
        ]}
        value={chip ?? undefined}
        onChange={next => setChip(next)}
        tabsAriaLabel={t('nav.brain')}
        tabsTestIdPrefix="brain-tab">
        <div className="w-full space-y-5">
          {loadError !== null && (
            <Alert variant="warning" data-testid="memory-load-error">
              <AlertDescription>
                <span>{loadError}</span>{' '}
                <Button
                  type="button"
                  variant="tertiary"
                  size="xs"
                  data-testid="memory-load-retry"
                  onClick={() => setReloadKey(k => k + 1)}>
                  {t('common.retry')}
                </Button>
              </AlertDescription>
            </Alert>
          )}
          {on && chip !== null && chip !== 'engine' && (
            <MemoryImportBanner engineLabel={activeLabel} />
          )}
          {body}
        </div>
      </SettingsTabbedPage>
    </div>
  );
}
