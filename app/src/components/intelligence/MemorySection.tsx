import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { ToastNotification } from '../../types/intelligence';
import { IS_DEV } from '../../utils/config';
import PillTabBar from '../PillTabBar';
import ConnectionPathTab from './ConnectionPathTab';
import DiagramViewerTab from './DiagramViewerTab';
import EntityAssociationsTab from './EntityAssociationsTab';
import GraphCentralityTab from './GraphCentralityTab';
import GraphCohesionTab from './GraphCohesionTab';
import MemoryFreshnessTab from './MemoryFreshnessTab';
import MemoryTimelineTab from './MemoryTimelineTab';
import { MemoryWorkspace } from './MemoryWorkspace';
import NamespaceOverviewTab from './NamespaceOverviewTab';

/**
 * Memory sub-tabs.
 *
 * All graph/memory-analysis surfaces that previously lived as top-level
 * Intelligence tabs are nested here under the "Memory" tab. The first sub-tab
 * (`memoryTree`) is the former top-level "Memory" tab (the MemoryWorkspace).
 */
type MemorySubTab =
  | 'memoryTree'
  | 'diagram'
  | 'centrality'
  | 'cohesion'
  | 'associations'
  | 'freshness'
  | 'timeline'
  | 'paths'
  | 'namespaces';

interface MemorySectionProps {
  onToast: (toast: Omit<ToastNotification, 'id'>) => void;
}

export default function MemorySection({ onToast }: MemorySectionProps) {
  const { t } = useT();
  const [activeSubTab, setActiveSubTab] = useState<MemorySubTab>('memoryTree');

  const allSubTabs: { id: MemorySubTab; label: string; devOnly?: boolean }[] = [
    { id: 'memoryTree', label: t('memory.tab.memoryTree') },
    { id: 'diagram', label: t('memory.tab.diagram'), devOnly: true },
    { id: 'centrality', label: t('memory.tab.centrality'), devOnly: true },
    { id: 'cohesion', label: t('memory.tab.cohesion'), devOnly: true },
    { id: 'associations', label: t('memory.tab.associations'), devOnly: true },
    { id: 'freshness', label: t('memory.tab.freshness'), devOnly: true },
    { id: 'timeline', label: t('memory.tab.timeline'), devOnly: true },
    { id: 'paths', label: t('memory.tab.path'), devOnly: true },
    { id: 'namespaces', label: t('memory.tab.namespaces'), devOnly: true },
  ];
  const subTabs = allSubTabs.filter(tab => !tab.devOnly || IS_DEV);

  return (
    <div className="space-y-4">
      <PillTabBar
        items={subTabs.map(tab => ({ label: tab.label, value: tab.id }))}
        selected={activeSubTab}
        onChange={setActiveSubTab}
        containerClassName="flex flex-wrap gap-2 pb-1"
      />

      {activeSubTab === 'memoryTree' && <MemoryWorkspace onToast={onToast} />}
      {activeSubTab === 'diagram' && <DiagramViewerTab />}
      {activeSubTab === 'centrality' && <GraphCentralityTab />}
      {activeSubTab === 'cohesion' && <GraphCohesionTab />}
      {activeSubTab === 'associations' && <EntityAssociationsTab />}
      {activeSubTab === 'freshness' && <MemoryFreshnessTab />}
      {activeSubTab === 'timeline' && <MemoryTimelineTab />}
      {activeSubTab === 'paths' && <ConnectionPathTab />}
      {activeSubTab === 'namespaces' && <NamespaceOverviewTab />}
    </div>
  );
}
