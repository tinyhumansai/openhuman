import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import ConnectionPathTab from '../../intelligence/ConnectionPathTab';
import DiagramViewerTab from '../../intelligence/DiagramViewerTab';
import EntityAssociationsTab from '../../intelligence/EntityAssociationsTab';
import GraphCentralityTab from '../../intelligence/GraphCentralityTab';
import GraphCohesionTab from '../../intelligence/GraphCohesionTab';
import MemoryFreshnessTab from '../../intelligence/MemoryFreshnessTab';
import MemoryTimelineTab from '../../intelligence/MemoryTimelineTab';
import NamespaceOverviewTab from '../../intelligence/NamespaceOverviewTab';
import ChipTabs from '../../layout/ChipTabs';
import PanelPage from '../../layout/PanelPage';
import SettingsBackButton from '../components/SettingsBackButton';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

/**
 * Analysis views — the 8 parked memory-graph analysis surfaces.
 *
 * These were stripped from the layman Memory tab (#3397) but retained on disk
 * (and still unit-tested). The IA redesign re-surfaces them here, behind the
 * Developer & Diagnostics door → Knowledge & Memory → "Analysis views", so power
 * users can still reach them while laymen never see them.
 */
type AnalysisView =
  | 'diagram'
  | 'centrality'
  | 'cohesion'
  | 'associations'
  | 'freshness'
  | 'timeline'
  | 'paths'
  | 'namespaces';

const AnalysisViewsPanel = () => {
  const { t } = useT();
  const { navigateBack } = useSettingsNavigation();
  const [activeView, setActiveView] = useState<AnalysisView>('diagram');

  const views: { id: AnalysisView; label: string }[] = [
    { id: 'diagram', label: t('memory.tab.diagram') },
    { id: 'centrality', label: t('memory.tab.centrality') },
    { id: 'cohesion', label: t('memory.tab.cohesion') },
    { id: 'associations', label: t('memory.tab.associations') },
    { id: 'freshness', label: t('memory.tab.freshness') },
    { id: 'timeline', label: t('memory.tab.timeline') },
    { id: 'paths', label: t('memory.tab.path') },
    { id: 'namespaces', label: t('memory.tab.namespaces') },
  ];

  return (
    <PanelPage
      className="z-10"
      contentClassName=""
      description={t('settings.analysisViews.menuDesc')}
      leading={<SettingsBackButton onBack={navigateBack} />}>
      <div className="p-4 space-y-5">
        <ChipTabs<AnalysisView>
          items={views.map(view => ({ id: view.id, label: view.label }))}
          value={activeView}
          onChange={setActiveView}
          className="flex flex-wrap gap-1.5 pb-1"
        />

        {activeView === 'diagram' && <DiagramViewerTab />}
        {activeView === 'centrality' && <GraphCentralityTab />}
        {activeView === 'cohesion' && <GraphCohesionTab />}
        {activeView === 'associations' && <EntityAssociationsTab />}
        {activeView === 'freshness' && <MemoryFreshnessTab />}
        {activeView === 'timeline' && <MemoryTimelineTab />}
        {activeView === 'paths' && <ConnectionPathTab />}
        {activeView === 'namespaces' && <NamespaceOverviewTab />}
      </div>
    </PanelPage>
  );
};

export default AnalysisViewsPanel;
