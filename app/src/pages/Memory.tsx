/**
 * Memory — the Memory v2 surface (docs/specs/memory-v2.md).
 *
 * Rendered as the Memory sub-page of Connections (`/connections?tab=brain`);
 * it has no route of its own. Connections owns `?tab=` and the sidebar; this
 * page keeps its chip in `?brain=`:
 *
 *   engine · migration · ask · explorer · learnings · conversations · brain · background · settings
 *
 * With no `?brain=` the page opens on the Provider (engine) chip. Retired
 * values (v1's `graph`, `goals`, `sources`, `sync`, `history`; v2's
 * `documents` and `context`) are rewritten to their current chip. While
 * memory is off every chip but Engine shows an empty state that points there.
 * Importing and organizing memory from earlier versions lives on its own
 * Migration chip, not as a banner over the others.
 *
 * debug logging: DEBUG=openhuman:memory
 */
import debug from 'debug';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import MemoryAskTab from '../components/memory/MemoryAskTab';
import MemoryBackgroundTab from '../components/memory/MemoryBackgroundTab';
import MemoryBrainTab from '../components/memory/MemoryBrainTab';
import { type MemoryChip, resolveMemoryChip } from '../components/memory/memoryChips';
import MemoryConversationsTab from '../components/memory/MemoryConversationsTab';
import MemoryEngineTab from '../components/memory/MemoryEngineTab';
import MemoryExplorerTab from '../components/memory/MemoryExplorerTab';
import MemoryLearningsTab from '../components/memory/MemoryLearningsTab';
import MemoryMigrationTab from '../components/memory/MemoryMigrationTab';
import MemoryOffState from '../components/memory/MemoryOffState';
import MemorySettingsTab from '../components/memory/MemorySettingsTab';
import SettingsTabbedPage from '../components/settings/layout/SettingsTabbedPage';
import { Alert, AlertDescription, Button } from '../components/ui';
import { CenteredLoadingState } from '../components/ui/LoadingState';
import { useT } from '../lib/i18n/I18nContext';
import { useCoreState } from '../providers/CoreStateProvider';
import {
  type EngineDescriptor,
  type EngineState,
  isMemoryDisabled,
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
        setLoadError(memoryErrorMessage(state.reason, t));
        // Treat an unreadable engine as off so the page still has a chip to show.
        setEngine({ engine: null, has_key: false, status: 'off', fetch_modes: [] });
      }
      if (list.status === 'fulfilled') setEngines(list.value.engines ?? []);
    });
    return () => {
      cancelled = true;
    };
  }, [authUserId, reloadKey, t]);

  const params = useMemo(() => new URLSearchParams(location.search), [location.search]);
  const rawChip = params.get('brain');
  const requested = resolveMemoryChip(rawChip);
  const on = isMemoryOn(engine);
  const chip: MemoryChip | null = requested ?? (engine ? 'engine' : null);

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
    if (!rawChip) return;
    if (requested) {
      if (requested !== rawChip) setChip(requested, true);
    } else if (engine) {
      // Unknown value: land on the default chip once it is known.
      setChip('engine', true);
    }
  }, [rawChip, requested, engine, setChip]);

  const headers: Record<MemoryChip, { title: string; description: string }> = {
    engine: { title: t('memoryPage.tabs.engine'), description: t('memoryPage.header.engine') },
    migration: {
      title: t('memoryPage.tabs.migration'),
      description: t('memoryPage.header.migration'),
    },
    ask: { title: t('memoryPage.tabs.ask'), description: t('memoryPage.header.ask') },
    explorer: {
      title: t('memoryPage.tabs.explorer'),
      description: t('memoryPage.header.explorer'),
    },
    learnings: {
      title: t('memoryPage.tabs.learnings'),
      description: t('memoryPage.header.learnings'),
    },
    conversations: {
      title: t('memoryPage.tabs.conversations'),
      description: t('memoryPage.header.conversations'),
    },
    brain: { title: t('memoryPage.tabs.brain'), description: t('memoryPage.header.brain') },
    background: {
      title: t('memoryPage.tabs.background'),
      description: t('memoryPage.header.background'),
    },
    settings: {
      title: t('memoryPage.tabs.settings'),
      description: t('memoryPage.header.settings'),
    },
  };

  const activeLabel = isMemoryDisabled(engine)
    ? t('memoryPage.engine.disabled.title')
    : (engines.find(e => e.id === engine?.engine)?.label ?? engine?.engine ?? t('nav.brain'));

  const body = (() => {
    if (chip === null || engine === null) {
      return <CenteredLoadingState label={t('memoryPage.loading')} />;
    }
    if (chip === 'engine') {
      return <MemoryEngineTab state={engine} onStateChange={setEngine} />;
    }
    const offState = on ? undefined : (
      <MemoryOffState
        reason={engine.reason}
        disabled={isMemoryDisabled(engine)}
        onOpenEngine={() => setChip('engine')}
      />
    );
    if (chip === 'migration') {
      return (
        <MemoryMigrationTab
          key={authUserId ?? 'signed-out'}
          engineLabel={activeLabel}
          offState={offState}
        />
      );
    }
    if (offState) return offState;
    switch (chip) {
      case 'ask':
        return <MemoryAskTab fetchModes={engine.fetch_modes ?? []} />;
      case 'explorer':
        return <MemoryExplorerTab />;
      case 'learnings':
        return <MemoryLearningsTab />;
      case 'conversations':
        return <MemoryConversationsTab />;
      case 'brain':
        return <MemoryBrainTab />;
      case 'background':
        return <MemoryBackgroundTab />;
      case 'settings':
        return <MemorySettingsTab onMemoryErased={() => setReloadKey(k => k + 1)} />;
      default:
        return null;
    }
  })();

  return (
    <div className="h-full w-full" data-testid="memory-page">
      <SettingsTabbedPage<MemoryChip>
        title={t('nav.brain')}
        description={headers[chip ?? 'engine'].description}
        tabs={[
          { id: 'engine', label: t('memoryPage.tabs.engine') },
          { id: 'migration', label: t('memoryPage.tabs.migration') },
          { id: 'ask', label: t('memoryPage.tabs.ask') },
          { id: 'explorer', label: t('memoryPage.tabs.explorer') },
          { id: 'learnings', label: t('memoryPage.tabs.learnings') },
          { id: 'conversations', label: t('memoryPage.tabs.conversations') },
          { id: 'brain', label: t('memoryPage.tabs.brain') },
          { id: 'background', label: t('memoryPage.tabs.background') },
          { id: 'settings', label: t('memoryPage.tabs.settings') },
        ]}
        value={chip ?? undefined}
        onChange={next => setChip(next)}
        tabsAriaLabel={t('nav.brain')}
        tabsTestIdPrefix="brain-tab">
        <div className="w-full space-y-5">
          <Alert
            variant="warning"
            density="compact"
            role={undefined}
            data-testid="memory-alpha-notice">
            <AlertDescription>{t('memoryPage.alphaNotice')}</AlertDescription>
          </Alert>
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
          {body}
        </div>
      </SettingsTabbedPage>
    </div>
  );
}
