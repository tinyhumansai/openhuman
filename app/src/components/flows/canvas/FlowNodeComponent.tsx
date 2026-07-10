/**
 * FlowNodeComponent — the custom xyflow node renderer for the Workflow Canvas
 * (issue B5b.1). Renders one rounded card per `WorkflowNode`: a per-kind emoji +
 * colored accent header, the node's name, a dynamic one-line summary of what the
 * node will do (derived from its live config via {@link describeNode}), and a
 * labelled row per input port (left) / output port (right).
 *
 * Ports read as labelled handle rows rather than a plaintext list: each port's
 * `Handle` sits inline next to its name so it's unambiguous which dot carries
 * which input/output (e.g. a `condition`'s `true`/`false` outputs). Branch ports
 * are colour-coded (true → sage, false/error → coral). A lone implicit `main`
 * port shows just its handle dot — left = input, right = output.
 *
 * When the card is selected in the editable canvas, an in-card action row
 * (Validate / Delete) appears via {@link useCanvasActions} — the read-only
 * viewer has no actions context, so it never shows them.
 *
 * An unrecognized `kind` renders as a plain neutral node rather than throwing,
 * since a thrown render error here has no error boundary around `<ReactFlow>`.
 */
import { Handle, type NodeProps, Position } from '@xyflow/react';
import { type CSSProperties, memo } from 'react';

import type { FlowNode } from '../../../lib/flows/graphAdapter';
import { COLOR_CLASSES, nodeKindMeta } from '../../../lib/flows/nodeKindMeta';
import { describeNode } from '../../../lib/flows/nodeSummary';
import { useT } from '../../../lib/i18n/I18nContext';
import { useCanvasActions } from './canvasActions';

/**
 * Inline the handle into the port row instead of React Flow's default absolute
 * edge placement, so each dot flows next to its label. React Flow still derives
 * the connection point from the handle's measured position, so edges attach
 * correctly.
 */
const INLINE_HANDLE_STYLE: CSSProperties = {
  position: 'relative',
  top: 'auto',
  left: 'auto',
  right: 'auto',
  transform: 'none',
};

/** The implicit single port; shown as a bare dot with no redundant label. */
const IMPLICIT_PORT = 'main';

/** Semantic colours for the well-known branch ports so routing reads at a glance. */
function portPillClass(port: string): string {
  const base = 'rounded px-1.5 py-0.5 text-[10px] font-medium leading-none';
  const key = port.toLowerCase();
  if (key === 'true') {
    return `${base} bg-sage-100 text-sage-700 dark:bg-sage-500/20 dark:text-sage-300`;
  }
  if (key === 'false' || key === 'error') {
    return `${base} bg-coral-100 text-coral-700 dark:bg-coral-500/20 dark:text-coral-300`;
  }
  return `${base} bg-surface-subtle text-content-secondary`;
}

function FlowNodeComponent({ id, data, selected }: NodeProps<FlowNode>) {
  const { t } = useT();
  const actions = useCanvasActions();
  const baseMeta = nodeKindMeta(data.kind);
  // A native "Tool" node (provider=openhuman / oh: slug) reads differently from
  // the Composio "App action" node even though both are `tool_call`.
  const isNativeTool =
    data.kind === 'tool_call' &&
    (data.config?.provider === 'openhuman' ||
      (typeof data.config?.slug === 'string' && data.config.slug.startsWith('oh:')));
  const meta = isNativeTool ? { ...baseMeta, emoji: '🛠️', color: 'primary' as const } : baseMeta;
  const colors = COLOR_CLASSES[meta.color];
  const kindLabel = t(`flows.nodeKind.${data.kind}`, data.kind);
  const summary = describeNode(data.kind, data.config ?? {}, data.outputPorts);

  // Only label ports when there's something to disambiguate: more than one port,
  // or a single explicitly-named (non-`main`) port. A lone implicit `main` shows
  // just its dot.
  const labelInputs = data.inputPorts.length > 1 || data.inputPorts.some(p => p !== IMPLICIT_PORT);
  const labelOutputs =
    data.outputPorts.length > 1 || data.outputPorts.some(p => p !== IMPLICIT_PORT);
  const hasPorts = data.inputPorts.length > 0 || data.outputPorts.length > 0;
  const showActions = Boolean(actions) && selected;

  return (
    <div
      data-testid="flow-node"
      data-node-kind={data.kind}
      className={`relative min-w-[180px] max-w-[240px] rounded-xl border-2 bg-surface shadow-sm ${colors.border} ${
        selected ? 'ring-2 ring-primary-500/40' : ''
      }`}>
      <div className={`flex items-center gap-2 rounded-t-[10px] px-3 py-2 ${colors.chip}`}>
        <span className="text-base leading-none" aria-hidden="true">
          {meta.emoji}
        </span>
        <div className="min-w-0 truncate text-sm font-semibold text-content">{data.name}</div>
      </div>

      {/* Dynamic "what this does" line, derived from the node's live config. The
          emoji already conveys the kind, so we show the description here rather
          than repeating the kind label (which duplicates a default node's name).
          Falls back to the kind label only when there's no config summary. */}
      <div
        className="px-3 pt-2 text-[11px] leading-snug text-content-muted"
        data-testid="flow-node-summary">
        {summary || kindLabel}
      </div>

      {hasPorts && (
        <div className="flex items-start justify-between gap-4 px-2 py-2">
          {/* Inputs — handle on the left edge, label to its right. */}
          <div className="flex min-w-0 flex-col gap-1.5">
            {data.inputPorts.map(port => (
              <div key={`in-${port}`} className="flex items-center gap-1.5">
                <Handle
                  id={port}
                  type="target"
                  position={Position.Left}
                  style={INLINE_HANDLE_STYLE}
                  title={port}
                />
                {labelInputs && <span className={`truncate ${portPillClass(port)}`}>{port}</span>}
              </div>
            ))}
          </div>

          {/* Outputs — label first, handle on the right edge. */}
          <div className="flex min-w-0 flex-col items-end gap-1.5">
            {data.outputPorts.map(port => (
              <div key={`out-${port}`} className="flex items-center gap-1.5">
                {labelOutputs && <span className={`truncate ${portPillClass(port)}`}>{port}</span>}
                <Handle
                  id={port}
                  type="source"
                  position={Position.Right}
                  style={INLINE_HANDLE_STYLE}
                  title={port}
                />
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Per-node actions on the selected card (editable canvas only). */}
      {showActions && actions && (
        <div className="flex items-center justify-end gap-1 border-t border-line px-2 py-1.5">
          <button
            type="button"
            data-testid="flow-node-validate"
            disabled={actions.validating}
            onClick={() => actions.validate()}
            className="rounded-md px-2 py-1 text-[11px] font-medium text-content-secondary transition-colors hover:bg-surface-hover disabled:opacity-50">
            {actions.validating ? t('flows.editor.validating') : t('flows.editor.validate')}
          </button>
          <button
            type="button"
            data-testid="flow-node-delete"
            onClick={() => actions.deleteNode(id)}
            className="rounded-md px-2 py-1 text-[11px] font-medium text-coral-600 transition-colors hover:bg-coral-50 dark:text-coral-400 dark:hover:bg-coral-500/10">
            {t('flows.editor.deleteNode')}
          </button>
        </div>
      )}
    </div>
  );
}

export default memo(FlowNodeComponent);
