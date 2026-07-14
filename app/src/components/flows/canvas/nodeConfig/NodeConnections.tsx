/**
 * NodeConnections — the "Connections" section of the node-config drawer. Lists
 * the selected node's incident edges as rows (Inputs = edges arriving at this
 * node, Outputs = edges leaving it), each showing the other node's name + the
 * ports involved, with a remove (✕) button. Handy for debugging a wiring at a
 * glance and detaching an edge without hunting for it on the canvas.
 *
 * Presentational: the canvas owns the edge state and passes `onRemoveEdge`.
 */
import { useMemo } from 'react';

import type { FlowEdge } from '../../../../lib/flows/graphAdapter';
import { useT } from '../../../../lib/i18n/I18nContext';

export interface NodeConnectionsProps {
  nodeId: string;
  edges: FlowEdge[];
  nodeLabelById: Record<string, string>;
  onRemoveEdge: (edgeId: string) => void;
}

/** A port name worth showing — `main` is the implicit default, so hide it. */
function portSuffix(handle: string | null | undefined): string {
  return handle && handle !== 'main' ? `:${handle}` : '';
}

function ConnectionRow({
  edgeId,
  label,
  onRemove,
  removeLabel,
}: {
  edgeId: string;
  label: string;
  onRemove: () => void;
  removeLabel: string;
}) {
  return (
    <li
      className="flex items-center justify-between gap-2 rounded-md border border-line bg-surface-muted px-2 py-1"
      data-testid={`node-connection-${edgeId}`}>
      <span className="min-w-0 truncate font-mono text-[11px] text-content-secondary">{label}</span>
      <button
        type="button"
        aria-label={removeLabel}
        title={removeLabel}
        data-testid={`node-connection-remove-${edgeId}`}
        onClick={onRemove}
        className="shrink-0 rounded p-0.5 text-content-faint transition-colors hover:bg-surface-hover hover:text-coral-600 dark:hover:text-coral-400">
        ✕
      </button>
    </li>
  );
}

export function NodeConnections({
  nodeId,
  edges,
  nodeLabelById,
  onRemoveEdge,
}: NodeConnectionsProps) {
  const { t } = useT();

  const { inputs, outputs } = useMemo(() => {
    const label = (id: string) => nodeLabelById[id] ?? id;
    return {
      // Incoming: this node is the target. Read as "<source>[:port] → :thisPort".
      inputs: edges
        .filter(e => e.target === nodeId)
        .map(e => ({
          id: e.id,
          label: `${label(e.source)}${portSuffix(e.sourceHandle)} → ${portSuffix(e.targetHandle) || 'in'}`,
        })),
      // Outgoing: this node is the source. Read as "thisPort: → <target>[:port]".
      outputs: edges
        .filter(e => e.source === nodeId)
        .map(e => ({
          id: e.id,
          label: `${portSuffix(e.sourceHandle) || 'out'} → ${label(e.target)}${portSuffix(e.targetHandle)}`,
        })),
    };
  }, [edges, nodeId, nodeLabelById]);

  const removeLabel = t('flows.nodeConfig.connections.remove');
  const hasAny = inputs.length > 0 || outputs.length > 0;

  return (
    <section className="space-y-2" data-testid="node-connections">
      <h3 className="text-[11px] font-semibold uppercase tracking-wide text-content-muted">
        {t('flows.nodeConfig.connections.title')}
      </h3>

      {!hasAny && (
        <p className="text-[11px] text-content-faint" data-testid="node-connections-empty">
          {t('flows.nodeConfig.connections.none')}
        </p>
      )}

      {inputs.length > 0 && (
        <div className="space-y-1">
          <div className="text-[10px] font-medium uppercase tracking-wide text-content-faint">
            {t('flows.nodeConfig.connections.inputs')}
          </div>
          <ul className="space-y-1" data-testid="node-connections-inputs">
            {inputs.map(edge => (
              <ConnectionRow
                key={edge.id}
                edgeId={edge.id}
                label={edge.label}
                removeLabel={removeLabel}
                onRemove={() => onRemoveEdge(edge.id)}
              />
            ))}
          </ul>
        </div>
      )}

      {outputs.length > 0 && (
        <div className="space-y-1">
          <div className="text-[10px] font-medium uppercase tracking-wide text-content-faint">
            {t('flows.nodeConfig.connections.outputs')}
          </div>
          <ul className="space-y-1" data-testid="node-connections-outputs">
            {outputs.map(edge => (
              <ConnectionRow
                key={edge.id}
                edgeId={edge.id}
                label={edge.label}
                removeLabel={removeLabel}
                onRemove={() => onRemoveEdge(edge.id)}
              />
            ))}
          </ul>
        </div>
      )}
    </section>
  );
}
