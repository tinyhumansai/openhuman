/**
 * NodePalette — the editable Workflow Canvas's insert palette (issue B5b.2 /
 * Phase 3a). Lists all 12 tinyflows `NodeKind`s with the same emoji + accent
 * `FlowNodeComponent` renders (both pull from `lib/flows/nodeKindMeta.ts`), and
 * offers two ways to add a node:
 *
 *  - **click** an entry → `onAdd(kind)` (the canvas drops it at a default
 *    position). Keyboard-accessible and the path the unit tests drive, since
 *    jsdom can't produce real drag geometry.
 *  - **drag** an entry onto the canvas → sets a `application/tinyflows-node`
 *    dataTransfer payload the canvas's `onDrop` reads to place the node under
 *    the cursor. Pointer-only affordance, layered on top of click.
 */
import { memo } from 'react';

import { COLOR_CLASSES, NODE_KINDS, nodeKindMeta } from '../../../lib/flows/nodeKindMeta';
import type { NodeKind } from '../../../lib/flows/types';
import { useT } from '../../../lib/i18n/I18nContext';

/** dataTransfer MIME key for a palette drag — read by the canvas `onDrop`. */
export const PALETTE_DND_MIME = 'application/tinyflows-node';

export interface NodePaletteProps {
  /** Add a node of `kind` at the canvas's default insert position (click path). */
  onAdd: (kind: NodeKind) => void;
}

function NodePalette({ onAdd }: NodePaletteProps) {
  const { t } = useT();

  return (
    <aside
      className="pointer-events-auto absolute left-3 top-3 z-10 flex max-h-[calc(100%-1.5rem)] w-44 flex-col overflow-hidden rounded-xl border border-line bg-surface/95 shadow-sm backdrop-blur"
      data-testid="flow-node-palette"
      aria-label={t('flows.palette.title')}>
      <div className="border-b border-line px-3 py-2 text-[11px] font-semibold uppercase tracking-wide text-content-faint">
        {t('flows.palette.title')}
      </div>
      <div className="flex flex-col gap-1 overflow-y-auto p-2">
        {NODE_KINDS.map(kind => {
          const meta = nodeKindMeta(kind);
          const colors = COLOR_CLASSES[meta.color];
          const label = t(`flows.nodeKind.${kind}`, kind);
          return (
            <button
              key={kind}
              type="button"
              draggable
              data-testid={`flow-palette-item-${kind}`}
              data-node-kind={kind}
              onClick={() => onAdd(kind)}
              onDragStart={event => {
                event.dataTransfer.setData(PALETTE_DND_MIME, kind);
                event.dataTransfer.effectAllowed = 'copy';
              }}
              title={t('flows.palette.addNode').replace('{kind}', label)}
              className={`flex items-center gap-2 rounded-lg border px-2 py-1.5 text-left text-xs text-content transition-colors hover:bg-surface-hover ${colors.border}`}>
              <span className="text-base leading-none" aria-hidden="true">
                {meta.emoji}
              </span>
              <span className="truncate">{label}</span>
            </button>
          );
        })}
      </div>
    </aside>
  );
}

export default memo(NodePalette);
