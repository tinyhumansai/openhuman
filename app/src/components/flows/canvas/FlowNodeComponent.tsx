/**
 * FlowNodeComponent — the custom xyflow node renderer for the Workflow Canvas
 * (issue B5b.1). Renders one card per `WorkflowNode`: a kind glyph on a colour-
 * coded tile, the node's name over its execution-order number and kind, a
 * dynamic one-line summary of what the node will do (derived from its live
 * config via {@link describeNode}), and a labelled row per input port (left) /
 * output port (right).
 *
 * **Identity is a filled shape; status is an outline.** Kind colour is confined
 * to the 32px glyph tile (see `NODE_KIND_TILE`); the card body stays a neutral
 * elevated surface. State therefore always has somewhere uncontested to draw:
 * the live run overlay's rings (`.flow-node-running` / `-success` / `-failed`),
 * the validation ring (`.flow-node-error`), the copilot diff overlay, and
 * selection all render on the card's border, never on the tile.
 *
 * This split replaced a per-kind coloured *border* + header chip that cycled 14
 * node kinds through 4 semantic ramps. Because those ramps carry meaning
 * elsewhere in the product (coral = error, sage = success), a `merge` node
 * rendered coral and a `tool_call` amber read as status rather than type — a
 * canvas of healthy nodes looked like a canvas of warnings, and the real state
 * rings had nothing left to contrast against. Moving the same hues onto a small
 * filled tile keeps kinds recognisable before the label is readable without
 * ever impersonating run state.
 *
 * The card is deliberately elevated rather than flat: on the dark theme the
 * canvas is pure black and `surface` is `#171717`, so a flat card with a
 * default `line` border is nearly invisible. It uses a top-lit gradient,
 * `line-strong`, and a two-layer shadow to separate from the canvas in both
 * themes.
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
import { LuSparkles } from 'react-icons/lu';

import type { FlowNode } from '../../../lib/flows/graphAdapter';
import { NodeKindTile } from '../../../lib/flows/nodeKindIcons';
import { describeNode } from '../../../lib/flows/nodeSummary';
import { useT } from '../../../lib/i18n/I18nContext';
import { useCanvasActions } from './canvasActions';
import { useStepNumber } from './stepNumbers';

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
  const { t, locale } = useT();
  const actions = useCanvasActions();
  const stepNumber = useStepNumber(id);
  // A native "Tool" node (provider=openhuman / oh: slug) reads differently from
  // the Composio "App action" node even though both are `tool_call` — the
  // former calls one of the assistant's own built-ins, the latter reaches a
  // connected third-party account. Distinguish them with the sparkles glyph on
  // the `agent` tile, grouping the assistant's own capabilities under one hue.
  const isNativeTool =
    data.kind === 'tool_call' &&
    (data.config?.provider === 'openhuman' ||
      (typeof data.config?.slug === 'string' && data.config.slug.startsWith('oh:')));
  const kindLabel = t(`flows.nodeKind.${data.kind}`, data.kind);
  const summary = describeNode(data.kind, data.config ?? {}, data.outputPorts, t, locale);

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
      className={`relative w-[264px] rounded-xl border bg-surface shadow-[0_1px_2px_rgba(0,0,0,0.06),0_4px_12px_rgba(0,0,0,0.08)] transition-all duration-150 dark:shadow-[0_2px_6px_rgba(0,0,0,0.5),0_8px_24px_rgba(0,0,0,0.45)] ${
        selected
          ? 'border-primary-500 ring-2 ring-primary-500/40'
          : 'border-line-strong hover:-translate-y-px hover:border-primary-500/40 hover:shadow-[0_2px_4px_rgba(0,0,0,0.08),0_10px_28px_rgba(0,0,0,0.12)] dark:hover:shadow-[0_4px_10px_rgba(0,0,0,0.55),0_14px_36px_rgba(0,0,0,0.5)]'
      }`}>
      {/* Title band — the card's only secondary fill. Everything below the
          divider shares the card's own `bg-surface`, so the body reads as one
          uninterrupted surface: a separately-tinted summary well plus an
          untinted connector row gave the card three competing bands. */}
      <div className="flex items-center gap-3 rounded-t-xl border-b border-line-strong/60 bg-surface-muted px-3 py-2.5">
        {/* Kind glyph on a saturated tile — the card's only filled colour.
            Status stays legible because it is drawn as a ring around the whole
            card, so identity (filled square) and state (outline) never share
            pixels. See NODE_KIND_TILE for the hue grouping. */}
        <NodeKindTile
          kind={isNativeTool ? 'agent' : data.kind}
          icon={isNativeTool ? LuSparkles : undefined}
          testId="flow-node-icon"
        />
        <div className="min-w-0 flex-1">
          <div
            className="min-w-0 truncate text-[13px] font-semibold leading-tight text-content"
            title={data.name}>
            {data.name}
          </div>
          <div className="mt-1 flex items-center gap-1.5 text-[10px] leading-none text-content-muted">
            {stepNumber !== undefined && (
              <>
                <span data-testid="flow-node-step" className="font-semibold tabular-nums">
                  {stepNumber}
                </span>
                <span aria-hidden="true" className="opacity-40">
                  ·
                </span>
              </>
            )}
            <span className="truncate font-medium uppercase tracking-[0.08em]">{kindLabel}</span>
          </div>
        </div>
      </div>

      {/* Dynamic "what this does" line, derived from the node's live config.
          Untinted on purpose — the title band's divider already separates
          identity from behaviour, so a second fill here would only add a band. */}
      {summary && (
        <div
          className="px-3 pt-2 text-[11px] leading-snug text-content-secondary"
          data-testid="flow-node-summary">
          {summary}
        </div>
      )}

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
