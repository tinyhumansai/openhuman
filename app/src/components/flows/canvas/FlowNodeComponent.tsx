/**
 * FlowNodeComponent — the custom xyflow node renderer for the read-only
 * Workflow Canvas (issue B5b.1). Renders one rounded card per `WorkflowNode`:
 * a per-kind emoji + colored accent, the node's name, and a `Handle` per
 * effective input port (left) / output port (right) — see
 * `graphAdapter.ts`'s `FlowNodeData` for why "effective" ports aren't simply
 * `data.ports`.
 *
 * Emoji (not an icon library) matches the repo's existing convention —
 * there is no `lucide-react` (or any icon-font) dependency in this app today
 * (icons are hand-rolled inline SVG, see `components/ui/icons.tsx`), and
 * adding one is out of scope for this slice's single approved dependency
 * (`@xyflow/react`).
 *
 * An unrecognized `kind` (not one of the 12 `NodeKind` values — e.g. a future
 * tinyflows addition, since `Flow.graph` is `unknown` on the wire) renders as
 * a plain neutral node rather than throwing, since a thrown render error here
 * has no error boundary around `<ReactFlow>` and would take down the whole
 * canvas.
 */
import { Handle, type NodeProps, Position } from '@xyflow/react';
import { memo } from 'react';

import type { FlowNode } from '../../../lib/flows/graphAdapter';
import { COLOR_CLASSES, handleOffsets, nodeKindMeta } from '../../../lib/flows/nodeKindMeta';
import { useT } from '../../../lib/i18n/I18nContext';

function FlowNodeComponent({ data, selected }: NodeProps<FlowNode>) {
  const { t } = useT();
  const meta = nodeKindMeta(data.kind);
  const colors = COLOR_CLASSES[meta.color];
  const inputOffsets = handleOffsets(data.inputPorts.length);
  const outputOffsets = handleOffsets(data.outputPorts.length);
  const kindLabel = t(`flows.nodeKind.${data.kind}`, data.kind);

  return (
    <div
      data-testid="flow-node"
      data-node-kind={data.kind}
      className={`relative min-w-[180px] max-w-[240px] rounded-xl border-2 bg-surface shadow-sm ${colors.border} ${
        selected ? 'ring-2 ring-primary-500/40' : ''
      }`}>
      {data.inputPorts.map((port, i) => (
        <Handle
          key={`in-${port}`}
          id={port}
          type="target"
          position={Position.Left}
          style={{ top: `${inputOffsets[i]}%` }}
          title={port}
        />
      ))}

      <div className={`flex items-center gap-2 rounded-t-[10px] px-3 py-2 ${colors.chip}`}>
        <span className="text-base leading-none" aria-hidden="true">
          {meta.emoji}
        </span>
        <div className="min-w-0">
          <div className="truncate text-sm font-semibold text-content">{data.name}</div>
          <div className="truncate text-[10px] uppercase tracking-wide text-content-faint">
            {kindLabel}
          </div>
        </div>
      </div>

      {data.outputPorts.length > 1 && (
        <div className="space-y-0.5 px-3 py-2 text-[10px] text-content-faint">
          {data.outputPorts.map(port => (
            <div key={port} className="truncate">
              {port}
            </div>
          ))}
        </div>
      )}

      {data.outputPorts.map((port, i) => (
        <Handle
          key={`out-${port}`}
          id={port}
          type="source"
          position={Position.Right}
          style={{ top: `${outputOffsets[i]}%` }}
          title={port}
        />
      ))}
    </div>
  );
}

export default memo(FlowNodeComponent);
