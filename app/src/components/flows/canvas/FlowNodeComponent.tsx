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
 * The card is 224px wide and tightly padded. It was 264px, sized when ports
 * were labelled rows INSIDE the card and needed the horizontal room; with the
 * ports on the top and bottom edges that room is dead space, and a narrower
 * card means more of the graph fits on screen at a readable zoom. The name
 * still gets ~156px after the 32px kind tile and the padding, which is what it
 * truncates against.
 *
 * The card is deliberately elevated rather than flat: on the dark theme the
 * canvas is pure black and `surface` is `#171717`, so a flat card with a
 * default `line` border is nearly invisible. It uses a top-lit gradient,
 * `line-strong`, and a two-layer shadow to separate from the canvas in both
 * themes.
 *
 * **Ports connect vertically: inputs on the top edge, outputs on the bottom.**
 * They used to sit on the left and right edges, which fought the graph's own
 * shape — `autoLayout` has always laid flows out top-to-bottom (`y = depth *
 * 132`, siblings spread across `x`), so every edge left a node sideways and
 * doubled back to enter the next one sideways. Straightening the handles onto
 * the axis the layout already uses is what makes an edge a straight drop.
 *
 * Each port's dot straddles its edge and its label, when it has one, sits
 * OUTSIDE the card — above the top dots, below the bottom ones. Inside the card
 * they would have to overlap the title band or the summary. Labels only appear
 * when there is something to disambiguate (more than one port, or a single
 * explicitly-named one), so the common node is a bare dot top and bottom and
 * the labelled case is mostly a `condition`'s `true`/`false`, which is exactly
 * where a label under the outgoing dot reads best. Branch ports stay
 * colour-coded (true → sage, false/error → coral).
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
import { Button } from '../../ui';
import { useCanvasActions } from './canvasActions';
import { useStepNumber } from './stepNumbers';

/**
 * Inline the handle into its port column instead of React Flow's default
 * absolute edge placement, so each dot flows above/below its own label. React
 * Flow still derives the connection point from the handle's measured position,
 * so edges attach correctly.
 */
const INLINE_HANDLE_STYLE: CSSProperties = {
  position: 'relative',
  top: 'auto',
  left: 'auto',
  right: 'auto',
  transform: 'none',
};

/**
 * The connector dot. React Flow's default is a 6px square in its own palette,
 * which on this canvas reads as a speck rather than something you can grab.
 * This is a 12px token-coloured circle ringed in the card's own surface, so it
 * stays visible against both the card and the canvas behind it, and it grows on
 * hover to advertise that it is a drag target.
 *
 * `!` on each utility because React Flow ships its own `.react-flow__handle`
 * rule for these properties and loads its stylesheet after ours.
 */
const HANDLE_CLASS =
  '!h-3 !w-3 !rounded-full !border-2 !border-surface !bg-primary-500 !transition-transform hover:!scale-125';

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
  const showActions = Boolean(actions) && selected;

  return (
    <div
      data-testid="flow-node"
      data-node-kind={data.kind}
      className={`relative w-[224px] rounded-2xl border bg-surface shadow-[0_1px_2px_rgba(0,0,0,0.06),0_4px_12px_rgba(0,0,0,0.08)] transition-all duration-150 dark:shadow-[0_2px_6px_rgba(0,0,0,0.5),0_8px_24px_rgba(0,0,0,0.45)] ${
        selected
          ? 'border-primary-500 ring-2 ring-primary-500/40'
          : 'border-line-strong hover:-translate-y-px hover:border-primary-500/40 hover:shadow-[0_2px_4px_rgba(0,0,0,0.08),0_10px_28px_rgba(0,0,0,0.12)] dark:hover:shadow-[0_4px_10px_rgba(0,0,0,0.55),0_14px_36px_rgba(0,0,0,0.5)]'
      }`}>
      {/* Inputs — dots straddling the TOP edge, each label (when there is one)
          stacked above its own dot and outside the card, where it cannot
          overlap the title band. `-translate-y-1/2` centres the dot on the
          border rather than resting it against the inside. */}
      {data.inputPorts.length > 0 && (
        <div
          className="absolute inset-x-0 top-0 flex -translate-y-1/2 items-center justify-center gap-6"
          data-testid="flow-node-inputs">
          {data.inputPorts.map(port => (
            <div key={`in-${port}`} className="relative flex flex-col items-center">
              {/* Absolute, so the label's height stays OUT of the column box.
                  In flow it made the column taller than the dot, and the row's
                  `-translate-y-1/2` then centred that taller box on the border
                  — which pushed the dot itself down inside the card. Only the
                  unlabelled ports looked right. */}
              {labelInputs && (
                <span className={`absolute bottom-full mb-1 ${portPillClass(port)}`}>{port}</span>
              )}
              <Handle
                id={port}
                type="target"
                position={Position.Top}
                style={INLINE_HANDLE_STYLE}
                className={HANDLE_CLASS}
                title={port}
              />
            </div>
          ))}
        </div>
      )}

      {/* Title band — the card's only secondary fill. Everything below the
          divider shares the card's own `bg-surface`, so the body reads as one
          uninterrupted surface: a separately-tinted summary well plus an
          untinted connector row gave the card three competing bands. */}
      <div className="flex items-center gap-2.5 rounded-t-2xl border-b border-line-strong/60 bg-surface-muted px-2.5 py-2">
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
          className="px-2.5 pb-2.5 pt-1.5 text-[11px] leading-snug text-content-secondary"
          data-testid="flow-node-summary">
          {summary}
        </div>
      )}

      {/* Per-node actions on the selected card (editable canvas only). */}
      {showActions && actions && (
        <div className="flex items-center justify-end gap-1 border-t border-line px-2 py-1.5">
          <Button
            type="button"
            variant="tertiary"
            size="xs"
            data-testid="flow-node-validate"
            disabled={actions.validating}
            onClick={() => actions.validate()}
            className="h-auto px-2 py-1 text-[11px]">
            {actions.validating ? t('flows.editor.validating') : t('flows.editor.validate')}
          </Button>
          <Button
            type="button"
            variant="tertiary"
            tone="danger"
            size="xs"
            data-testid="flow-node-delete"
            onClick={() => actions.deleteNode(id)}
            className="h-auto px-2 py-1 text-[11px]">
            {t('flows.editor.deleteNode')}
          </Button>
        </div>
      )}

      {/* Outputs — dots straddling the BOTTOM edge, labels below them. This is
          where the labelled case actually lands in practice: a `condition`'s
          `true` / `false` sit under their own outgoing dots, so which branch a
          given edge leaves by is readable without tracing it. */}
      {data.outputPorts.length > 0 && (
        <div
          className="absolute inset-x-0 bottom-0 flex translate-y-1/2 items-center justify-center gap-6"
          data-testid="flow-node-outputs">
          {data.outputPorts.map(port => (
            <div key={`out-${port}`} className="relative flex flex-col items-center">
              <Handle
                id={port}
                type="source"
                position={Position.Bottom}
                style={INLINE_HANDLE_STYLE}
                className={HANDLE_CLASS}
                title={port}
              />
              {/* Absolute for the same reason as the input labels above. */}
              {labelOutputs && (
                <span className={`absolute top-full mt-1 ${portPillClass(port)}`}>{port}</span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export default memo(FlowNodeComponent);
