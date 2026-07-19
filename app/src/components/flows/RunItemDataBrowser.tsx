/**
 * RunItemDataBrowser (Phase 6)
 * ----------------------------
 *
 * Per-item data browser for a single run step's output, extracted from
 * {@link FlowRunInspectorDrawer} to keep both files small. Renders the n8n
 * signature **table ⟷ JSON** toggle over the step's normalized output items
 * (see `lib/flows/runItems.ts`):
 *
 *   - **Table view** — one row per item, columns derived from the union of the
 *     items' `json` keys. Long cell values truncate (full value on hover via
 *     `title`); the whole table scrolls horizontally when wide.
 *   - **JSON view** — the items' `json` payloads pretty-printed, vertically
 *     scrollable.
 *
 * Binary attachments are never inlined — they render as placeholder chips
 * (name / MIME). When an output item carries a resolved `paired_item` and the
 * caller supplied the step's `inputItems`, a "Source" affordance reveals the
 * input item that produced it; absent pairing, no affordance is offered.
 */
import debug from 'debug';
import { useMemo, useState } from 'react';

import {
  cellValue,
  collectColumns,
  type FlowRunItem,
  formatCell,
  formatJson,
  hasObjectRows,
} from '../../lib/flows/runItems';
import { useT } from '../../lib/i18n/I18nContext';

const log = debug('flows:run-item-data-browser');

/** Cap a single table cell's rendered text so one huge value can't blow out the row. */
const MAX_CELL_CHARS = 200;

type ViewMode = 'table' | 'json';

function truncate(text: string): string {
  return text.length > MAX_CELL_CHARS ? `${text.slice(0, MAX_CELL_CHARS)}…` : text;
}

interface BinaryChipsProps {
  binary: FlowRunItem['binary'];
  testId: string;
}

/** Placeholder chips for an item's binary attachments — metadata only, no bytes. */
function BinaryChips({ binary, testId }: BinaryChipsProps) {
  const { t } = useT();
  if (binary.length === 0) return null;
  return (
    <div className="flex flex-wrap gap-1" data-testid={testId}>
      {binary.map(ref => (
        <span
          key={ref.key}
          className="inline-flex items-center gap-1 rounded-md border border-line bg-surface px-1.5 py-0.5 text-[10px] font-medium text-content-muted"
          title={ref.mimeType ?? undefined}>
          <span aria-hidden>📎</span>
          <span className="font-mono">{ref.fileName ?? ref.key}</span>
          <span className="rounded bg-surface-muted px-1 text-[9px] uppercase text-content-faint">
            {ref.mimeType ?? t('flowRuns.inspector.binaryLabel')}
          </span>
        </span>
      ))}
    </div>
  );
}

interface Props {
  /** Normalized output items of the step being inspected. */
  items: FlowRunItem[];
  /**
   * Normalized items of the step's *input* (typically the upstream step's
   * output). When present, output items carrying a resolved `paired_item` gain
   * a "Source" affordance that reveals the input item at that index. Omitted →
   * no pairing affordance is offered.
   */
  inputItems?: FlowRunItem[];
  /** Stable prefix for `data-testid`s so multiple browsers on one screen don't collide. */
  testIdPrefix: string;
}

export function RunItemDataBrowser({ items, inputItems, testIdPrefix }: Props) {
  const { t } = useT();
  const [view, setView] = useState<ViewMode>('table');
  // Which output row currently has its paired source input revealed (single-open).
  const [revealedSource, setRevealedSource] = useState<number | null>(null);

  const columns = useMemo(() => collectColumns(items), [items]);
  const useColumns = hasObjectRows(items) && columns.length > 0;
  const showActions = useMemo(
    () =>
      items.some(
        item => item.binary.length > 0 || (item.pairedIndex !== null && inputItems !== undefined)
      ),
    [items, inputItems]
  );

  const jsonText = useMemo(() => formatJson(items.map(item => item.json)), [items]);

  const totalColSpan = 1 + (useColumns ? columns.length : 1) + (showActions ? 1 : 0);

  if (items.length === 0) {
    return (
      <p className="text-[11px] italic text-content-faint" data-testid={`${testIdPrefix}-no-items`}>
        {t('flowRuns.inspector.noItems')}
      </p>
    );
  }

  const toggleSource = (index: number, pairedIndex: number) => {
    const next = revealedSource === index ? null : index;
    log(
      'toggleSource: prefix=%s row=%d paired=%d open=%s',
      testIdPrefix,
      index,
      pairedIndex,
      next !== null
    );
    setRevealedSource(next);
  };

  return (
    <div data-testid={`${testIdPrefix}-data-browser`}>
      {/* Header: view toggle + item count. */}
      <div className="mb-1.5 flex items-center justify-between gap-2">
        <div
          className="inline-flex overflow-hidden rounded-md border border-line"
          role="group"
          aria-label={t('flowRuns.inspector.dataViewLabel')}>
          <button
            type="button"
            data-testid={`${testIdPrefix}-view-table`}
            aria-pressed={view === 'table'}
            onClick={() => setView('table')}
            className={`px-2 py-0.5 text-[11px] font-medium ${
              view === 'table'
                ? 'bg-ocean-500 text-white'
                : 'bg-surface text-content-muted hover:bg-surface-hover'
            }`}>
            {t('flowRuns.inspector.dataTable')}
          </button>
          <button
            type="button"
            data-testid={`${testIdPrefix}-view-json`}
            aria-pressed={view === 'json'}
            onClick={() => setView('json')}
            className={`border-l border-line px-2 py-0.5 text-[11px] font-medium ${
              view === 'json'
                ? 'bg-ocean-500 text-white'
                : 'bg-surface text-content-muted hover:bg-surface-hover'
            }`}>
            {t('flowRuns.inspector.dataJson')}
          </button>
        </div>
        <span className="text-[10px] text-content-faint" data-testid={`${testIdPrefix}-item-count`}>
          {t('flowRuns.inspector.itemCount').replace('{count}', String(items.length))}
        </span>
      </div>

      {view === 'json' ? (
        <pre
          data-testid={`${testIdPrefix}-json`}
          className="max-h-72 overflow-auto whitespace-pre-wrap break-words rounded bg-surface px-2 py-1.5 font-mono text-[11px] leading-relaxed text-content-secondary">
          {jsonText}
        </pre>
      ) : (
        <div
          className="max-h-72 overflow-auto rounded border border-line"
          data-testid={`${testIdPrefix}-table-scroll`}>
          <table
            className="w-full border-collapse text-left text-[11px]"
            data-testid={`${testIdPrefix}-table`}>
            <thead>
              <tr className="bg-surface-muted text-content-muted">
                <th className="border-b border-line px-1.5 py-1 font-medium" aria-hidden />
                {useColumns ? (
                  columns.map(column => (
                    <th
                      key={column}
                      className="border-b border-line px-1.5 py-1 font-mono font-medium"
                      scope="col">
                      {column}
                    </th>
                  ))
                ) : (
                  <th className="border-b border-line px-1.5 py-1 font-medium" scope="col">
                    {t('flowRuns.inspector.dataJson')}
                  </th>
                )}
                {showActions && <th className="border-b border-line px-1.5 py-1" aria-hidden />}
              </tr>
            </thead>
            <tbody>
              {items.map((item, index) => {
                const canPair = item.pairedIndex !== null && inputItems !== undefined;
                const sourceItem =
                  canPair && item.pairedIndex !== null ? inputItems?.[item.pairedIndex] : undefined;
                const isRevealed = revealedSource === index;
                return (
                  <FragmentRow
                    key={index}
                    item={item}
                    index={index}
                    columns={columns}
                    useColumns={useColumns}
                    showActions={showActions}
                    canPair={canPair}
                    isRevealed={isRevealed}
                    sourceItem={sourceItem}
                    totalColSpan={totalColSpan}
                    testIdPrefix={testIdPrefix}
                    onToggleSource={() =>
                      item.pairedIndex !== null && toggleSource(index, item.pairedIndex)
                    }
                  />
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

interface FragmentRowProps {
  item: FlowRunItem;
  index: number;
  columns: string[];
  useColumns: boolean;
  showActions: boolean;
  canPair: boolean;
  isRevealed: boolean;
  sourceItem: FlowRunItem | undefined;
  totalColSpan: number;
  testIdPrefix: string;
  onToggleSource: () => void;
}

function FragmentRow({
  item,
  index,
  columns,
  useColumns,
  showActions,
  canPair,
  isRevealed,
  sourceItem,
  totalColSpan,
  testIdPrefix,
  onToggleSource,
}: FragmentRowProps) {
  const { t } = useT();
  return (
    <>
      <tr
        data-testid={`${testIdPrefix}-row-${index}`}
        className="border-b border-line last:border-b-0 align-top">
        <th scope="row" className="px-1.5 py-1 text-left font-mono font-normal text-content-faint">
          {index + 1}
        </th>
        {useColumns ? (
          columns.map(column => {
            const text = formatCell(cellValue(item, column));
            return (
              <td
                key={column}
                className="max-w-[16rem] truncate px-1.5 py-1 font-mono text-content-secondary"
                title={text || undefined}>
                {truncate(text)}
              </td>
            );
          })
        ) : (
          <td
            className="max-w-[16rem] truncate px-1.5 py-1 font-mono text-content-secondary"
            title={formatCell(item.json) || undefined}>
            {truncate(formatCell(item.json))}
          </td>
        )}
        {showActions && (
          <td className="whitespace-nowrap px-1.5 py-1">
            <div className="flex items-center justify-end gap-1.5">
              <BinaryChips binary={item.binary} testId={`${testIdPrefix}-binary-${index}`} />
              {canPair && (
                <button
                  type="button"
                  data-testid={`${testIdPrefix}-source-toggle-${index}`}
                  aria-pressed={isRevealed}
                  onClick={onToggleSource}
                  className="rounded border border-line px-1.5 py-0.5 text-[10px] font-medium text-content-muted hover:bg-surface-hover">
                  {isRevealed
                    ? t('flowRuns.inspector.hideSource')
                    : t('flowRuns.inspector.showSource')}
                </button>
              )}
            </div>
          </td>
        )}
      </tr>
      {canPair && isRevealed && (
        <tr data-testid={`${testIdPrefix}-source-${index}`}>
          <td colSpan={totalColSpan} className="bg-surface-muted px-1.5 py-1.5">
            <div className="text-[10px] font-semibold uppercase tracking-wide text-content-faint">
              {t('flowRuns.inspector.sourceInputTitle')}
            </div>
            <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded bg-surface px-2 py-1.5 font-mono text-[11px] leading-relaxed text-content-secondary">
              {sourceItem ? formatJson(sourceItem.json) : t('flowRuns.inspector.emptyValue')}
            </pre>
          </td>
        </tr>
      )}
    </>
  );
}

export default RunItemDataBrowser;
