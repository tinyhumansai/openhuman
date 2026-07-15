/**
 * NodePalette — the editable Workflow Canvas's insert palette (issue B5b.2 /
 * Phase 3a). Lists the tinyflows node kinds as {@link PaletteEntry}s grouped
 * into labelled sections (Triggers / Actions / Logic). `tool_call` splits into
 * two entries — an "App action" (Composio OAuth) and a "Tool" (native OpenHuman)
 * — so the two are distinct nodes in the palette. Two ways to add:
 *
 *  - **click** an entry → `onAdd(entry)` (the canvas drops it at a default
 *    position). Keyboard-accessible and the path the unit tests drive.
 *  - **drag** an entry onto the canvas → sets the entry `key` on a
 *    `application/tinyflows-node` payload the canvas's `onDrop` resolves.
 */
import { memo } from 'react';

import {
  COLOR_CLASSES,
  NODE_GROUP_ORDER,
  PALETTE_ENTRIES_BY_GROUP,
  type PaletteEntry,
} from '../../../lib/flows/nodeKindMeta';
import { useT } from '../../../lib/i18n/I18nContext';

/** dataTransfer MIME key for a palette drag — read by the canvas `onDrop`. */
export const PALETTE_DND_MIME = 'application/tinyflows-node';

export interface NodePaletteProps {
  /** Add a node from the given palette entry at the canvas's default position. */
  onAdd: (entry: PaletteEntry) => void;
}

function NodePalette({ onAdd }: NodePaletteProps) {
  const { t } = useT();

  return (
    <aside
      className="pointer-events-auto absolute right-3 top-14 z-10 flex max-h-[calc(100%-4rem)] w-48 flex-col overflow-hidden rounded-xl border border-line bg-surface/95 shadow-sm backdrop-blur"
      data-testid="flow-node-palette"
      aria-label={t('flows.palette.title')}>
      <div className="flex flex-col gap-2 overflow-y-auto p-2">
        {NODE_GROUP_ORDER.map(group => (
          <div key={group} className="flex flex-col gap-1">
            <div className="px-1 text-[10px] font-semibold uppercase tracking-wide text-content-faint">
              {t(`flows.palette.group.${group}`)}
            </div>
            {PALETTE_ENTRIES_BY_GROUP[group].map(entry => {
              const colors = COLOR_CLASSES[entry.color];
              const label = t(entry.labelKey, entry.kind);
              return (
                <button
                  key={entry.key}
                  type="button"
                  draggable
                  data-testid={`flow-palette-item-${entry.key}`}
                  data-node-kind={entry.kind}
                  onClick={() => onAdd(entry)}
                  onDragStart={event => {
                    event.dataTransfer.setData(PALETTE_DND_MIME, entry.key);
                    event.dataTransfer.effectAllowed = 'copy';
                  }}
                  title={t('flows.palette.addNode').replace('{kind}', label)}
                  className={`flex items-center gap-2 rounded-lg border px-2 py-1.5 text-left text-xs text-content transition-colors hover:bg-surface-hover ${colors.tint} ${colors.border}`}>
                  <span className="text-base leading-none" aria-hidden="true">
                    {entry.emoji}
                  </span>
                  <span className="truncate">{label}</span>
                </button>
              );
            })}
          </div>
        ))}
      </div>
    </aside>
  );
}

export default memo(NodePalette);
