/**
 * Brain — the centerpiece memory + subconscious surface.
 *
 * Two sub-tabs:
 *   - **Memory**: knowledge graph, tree status, and connected sources.
 *   - **Subconscious**: background thinking engine controls.
 */
import { useCallback, useEffect, useState } from 'react';

import IntelligenceSubconsciousTab from '../components/intelligence/IntelligenceSubconsciousTab';
import { MemoryControls } from '../components/intelligence/MemoryControls';
import { MemoryGraph } from '../components/intelligence/MemoryGraph';
import { MemorySourcesRegistry } from '../components/intelligence/MemorySourcesRegistry';
import { MemoryTreeStatusPanel } from '../components/intelligence/MemoryTreeStatusPanel';
import { ToastContainer } from '../components/intelligence/Toast';
import PillTabBar from '../components/PillTabBar';
import BetaBanner from '../components/ui/BetaBanner';
import { useSubconscious } from '../hooks/useSubconscious';
import { useT } from '../lib/i18n/I18nContext';
import type { ToastNotification } from '../types/intelligence';
import {
  type GraphExportResponse,
  type GraphMode,
  memoryTreeGraphExport,
} from '../utils/tauriCommands';

type BrainTab = 'memory' | 'subconscious';

export default function Brain() {
  const { t } = useT();
  const [activeTab, setActiveTab] = useState<BrainTab>('memory');
  const [graph, setGraph] = useState<GraphExportResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<GraphMode>('tree');
  const [refreshKey, setRefreshKey] = useState(0);
  const [toasts, setToasts] = useState<ToastNotification[]>([]);

  const sub = useSubconscious();

  const addToast = useCallback((toast: Omit<ToastNotification, 'id'>) => {
    setToasts(prev => [...prev, { ...toast, id: `toast-${Date.now()}-${Math.random()}` }]);
  }, []);
  const removeToast = useCallback((id: string) => {
    setToasts(prev => prev.filter(toast => toast.id !== id));
  }, []);
  const refresh = useCallback(() => setRefreshKey(k => k + 1), []);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      console.debug('[brain] graph fetch: entry mode=%s', mode);
      setError(null);
      try {
        const resp = await memoryTreeGraphExport(mode);
        if (cancelled) return;
        console.debug(
          '[brain] graph fetch: exit n=%d edges=%d',
          resp.nodes.length,
          resp.edges.length
        );
        setGraph(resp);
      } catch (err) {
        if (cancelled) return;
        console.error('[brain] graph fetch failed', err);
        setError(err instanceof Error ? err.message : String(err));
      }
    };
    void load();
    const onTreeDone = () => {
      console.debug('[brain] memory-tree-completed → refetch');
      void load();
    };
    window.addEventListener('openhuman:memory-tree-completed', onTreeDone);
    return () => {
      cancelled = true;
      window.removeEventListener('openhuman:memory-tree-completed', onTreeDone);
    };
  }, [mode, refreshKey]);

  const cardClass =
    'rounded-lg border border-stone-200 bg-white p-4 dark:border-neutral-800 dark:bg-neutral-900';

  return (
    <div className="relative min-h-full p-4 pt-6">
      <div className="mx-auto max-w-4xl space-y-5">
        <header className="min-w-0">
          <h1 className="text-xl font-bold text-stone-900 dark:text-neutral-100">
            {t('nav.brain')}
          </h1>
          <p className="mt-1 text-sm text-stone-500 dark:text-neutral-400">{t('brain.subtitle')}</p>
        </header>

        <PillTabBar<BrainTab>
          selected={activeTab}
          onChange={setActiveTab}
          items={[
            { value: 'memory', label: t('brain.tabs.memory') },
            { value: 'subconscious', label: t('brain.tabs.subconscious') },
          ]}
        />

        {activeTab === 'memory' && (
          <div className="space-y-5 animate-fade-up">
            <MemoryControls
              mode={mode}
              onModeChange={setMode}
              onRefresh={refresh}
              onToast={addToast}
              contentRootAbs={graph?.content_root_abs}
            />

            {graph ? (
              <MemoryGraph
                nodes={graph.nodes}
                edges={graph.edges}
                mode={mode}
                emptyHint={t('brain.empty')}
              />
            ) : error ? (
              <div
                className={`${cardClass} text-sm text-coral-600 dark:text-coral-400`}
                role="alert">
                {t('brain.error')}
              </div>
            ) : null}

            <div className="space-y-5">
              <div className={cardClass}>
                <MemoryTreeStatusPanel onToast={addToast} />
              </div>
              <MemorySourcesRegistry onToast={addToast} />
            </div>
          </div>
        )}

        {activeTab === 'subconscious' && (
          <div className="space-y-3 animate-fade-up">
            <BetaBanner />
            <div className={cardClass}>
              <IntelligenceSubconsciousTab
                status={sub.status}
                mode={sub.mode}
                intervalMinutes={sub.intervalMinutes}
                triggerTick={sub.triggerTick}
                triggering={sub.triggering}
                settingMode={sub.settingMode}
                setMode={sub.setMode}
                setIntervalMinutes={sub.setIntervalMinutes}
              />
            </div>
          </div>
        )}
      </div>

      <ToastContainer notifications={toasts} onRemove={removeToast} />
    </div>
  );
}
