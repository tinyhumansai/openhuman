/**
 * MedullaDemoGraph — the scale showcase for the "Agent graph" tab shown to
 * users without Medulla access. An agent core, two devices, and 100 / 20 agents
 * fanning out under each device — rendered by the live {@link MemoryGraph}
 * force engine (drag / pan / zoom, fluid settle), with a preview banner.
 *
 * The `tuning` prop cranks repulsion and link length so the 120 nodes fan wide
 * out from the core instead of clumping — the "tree fanning out" look.
 */
import { useMemo } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { MemoryGraph } from '../../intelligence/MemoryGraph';
import DemoScaleBanner from './DemoScaleBanner';
import { buildDemoGraph } from './medullaDemoData';

/** Wide fan-out: strong repulsion + long springs, weak centring pull. */
const DEMO_TUNING = {
  chargeScale: 3.2,
  linkDistanceScale: 2.4,
  distanceMaxScale: 3,
  centerScale: 0.4,
} as const;

export default function MedullaDemoGraph() {
  const { t } = useT();
  const deviceLabel = t('orchPage.demo.device');
  const { nodes, edges } = useMemo(() => buildDemoGraph(deviceLabel), [deviceLabel]);

  return (
    <div className="relative h-full p-2" data-testid="orch-demo-graph">
      <div className="pointer-events-none absolute inset-x-0 top-2 z-10 flex justify-center px-3">
        <DemoScaleBanner className="pointer-events-auto max-w-xl shadow-soft" />
      </div>
      <MemoryGraph
        nodes={nodes}
        edges={edges}
        mode="contacts"
        rootLabel={t('orchPage.overview.core')}
        emptyHint={t('orchPage.overview.empty')}
        tuning={DEMO_TUNING}
        fill
        fitToBounds
        showLabels
      />
    </div>
  );
}
