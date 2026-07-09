/**
 * NodeConfigDrawer (issue B5b / Phase 3b) — right-hand drawer that opens when a
 * single node is selected on the editable canvas. Renders the node's per-kind
 * config form ({@link NODE_CONFIG_FORMS}) with a raw-JSON escape hatch for kinds
 * without a dedicated form (and an opt-in "Edit as JSON" toggle for every kind).
 *
 * Chrome mirrors {@link FlowRunInspectorDrawer}: fixed overlay, backdrop click
 * and Escape both close. Unlike that drawer it is NOT full-height-modal — it
 * floats on the right of the canvas so the graph stays visible while editing,
 * but keeps the same close semantics.
 *
 * Controlled: every edit calls `onChange(nodeId, patch)`; the canvas owns node
 * state and re-renders the drawer with the updated `config`, so the form fields
 * always reflect the live draft (no local mirror of config that could drift).
 * The drawer body is keyed by node id so switching nodes cleanly re-seeds the
 * JSON editor's local text buffer.
 */
import createDebug from 'debug';
import { memo, useCallback, useMemo, useState } from 'react';

import { useEscapeKey } from '../../../../hooks/useEscapeKey';
import type { FlowEdge, FlowNode } from '../../../../lib/flows/graphAdapter';
import { nodeKindMeta } from '../../../../lib/flows/nodeKindMeta';
import { describeNode } from '../../../../lib/flows/nodeSummary';
import { useT } from '../../../../lib/i18n/I18nContext';
import type { FlowConnection } from '../../../../services/api/flowsApi';
import { JsonField } from './nodeConfigFields';
import { NODE_CONFIG_FORMS } from './nodeConfigForms';
import { NodeConnections } from './NodeConnections';
import { type UpstreamExpressionOption, upstreamExpressionOptions } from './upstreamOptions';

const log = createDebug('app:flows:nodeConfig:drawer');

export interface NodeConfigPatch {
  name?: string;
  config?: Record<string, unknown>;
}

export interface NodeConfigDrawerProps {
  /** The selected node to edit, or `null` when nothing single-node is selected. */
  node: FlowNode | null;
  onClose: () => void;
  /** Apply a name/config patch to the node identified by `nodeId`. */
  onChange: (nodeId: string, patch: NodeConfigPatch) => void;
  /** Secret-free credential refs for the picker (loaded once by the canvas). */
  connections: FlowConnection[];
  /** All graph nodes — used to derive the upstream `=nodes.…` picker options. */
  nodes?: FlowNode[];
  /** All graph edges — the drawer shows the selected node's incident ones. */
  edges?: FlowEdge[];
  /** Node id → display name, for labelling the other end of each connection. */
  nodeLabelById?: Record<string, string>;
  /** Remove a single edge by id (from the connections list). */
  onRemoveEdge?: (edgeId: string) => void;
}

function NodeConfigBody({
  node,
  onChange,
  connections,
  upstreamOptions,
}: {
  node: FlowNode;
  onChange: (nodeId: string, patch: NodeConfigPatch) => void;
  connections: FlowConnection[];
  upstreamOptions: UpstreamExpressionOption[];
}) {
  const { t } = useT();
  const config = useMemo(() => node.data.config ?? {}, [node.data.config]);
  const Form = NODE_CONFIG_FORMS[node.data.kind];
  // Kinds with no dedicated form start on the raw editor; kinds with a form
  // start on the form but can flip to raw via the toggle.
  const [rawMode, setRawMode] = useState(!Form);

  const mergeConfig = useCallback(
    (patch: Record<string, unknown>) => {
      log('mergeConfig: node=%s keys=%o', node.id, Object.keys(patch));
      onChange(node.id, { config: { ...config, ...patch } });
    },
    [node.id, config, onChange]
  );

  const replaceConfig = useCallback(
    (value: unknown) => {
      const next =
        value && typeof value === 'object' && !Array.isArray(value)
          ? (value as Record<string, unknown>)
          : {};
      log('replaceConfig: node=%s keys=%o', node.id, Object.keys(next));
      onChange(node.id, { config: next });
    },
    [node.id, onChange]
  );

  return (
    <div className="space-y-3">
      {Form && (
        <div className="flex justify-end">
          <button
            type="button"
            className="rounded-md border border-line px-2 py-0.5 text-[11px] font-medium text-content-muted hover:bg-surface-hover"
            data-testid="node-config-raw-toggle"
            onClick={() => setRawMode(m => !m)}>
            {rawMode ? t('flows.nodeConfig.editForm') : t('flows.nodeConfig.editJson')}
          </button>
        </div>
      )}

      {Form && !rawMode ? (
        <Form
          config={config}
          onChange={mergeConfig}
          connections={connections}
          upstreamOptions={upstreamOptions}
        />
      ) : (
        <JsonField
          label={t('flows.nodeConfig.rawJsonLabel')}
          hint={t('flows.nodeConfig.rawJsonHint')}
          value={config}
          onChange={replaceConfig}
          rows={12}
          testId="node-config-raw-json"
        />
      )}
    </div>
  );
}

function NodeConfigDrawer({
  node,
  onClose,
  onChange,
  connections,
  nodes = [],
  edges = [],
  nodeLabelById = {},
  onRemoveEdge = () => {},
}: NodeConfigDrawerProps) {
  const { t } = useT();

  useEscapeKey(() => {
    log('escape: closing');
    onClose();
  }, node !== null);

  // Upstream `=nodes.…` picker options for the selected node's expression
  // fields — its transitive ancestors' outputs (Feature: `nodes` scope).
  const upstreamOptions = useMemo(
    () => (node ? upstreamExpressionOptions(node.id, nodes, edges) : []),
    [node, nodes, edges]
  );

  if (!node) return null;

  const meta = nodeKindMeta(node.data.kind);
  const kindLabel = t(`flows.nodeKind.${node.data.kind}`, node.data.kind);
  // Dynamic "what this node will do", derived from the live config — updates as
  // the fields below are edited (same summary shown on the node card).
  const summary = describeNode(node.data.kind, node.data.config ?? {}, node.data.outputPorts);

  return (
    // `pointer-events-none` wrapper so the drawer floats over the canvas
    // without a backdrop — the graph stays fully interactive, and clicking an
    // empty canvas area deselects the node (closing the drawer) on its own.
    <div
      className="pointer-events-none absolute inset-0 z-20 flex justify-end"
      data-testid="node-config-drawer">
      <aside className="pointer-events-auto relative flex h-full w-full max-w-xs flex-col border-l border-line bg-surface shadow-xl">
        <header className="flex items-start gap-2 border-b border-line px-3.5 py-3">
          <span className="text-lg leading-none" aria-hidden="true">
            {meta.emoji}
          </span>
          <div className="min-w-0 flex-1">
            {/* Kind eyebrow — hidden when it just repeats the name (a default,
                unrenamed node), so the header doesn't show the title twice. */}
            {node.data.name.trim() !== kindLabel && (
              <div className="text-[11px] font-semibold uppercase tracking-wide text-content-faint">
                {kindLabel}
              </div>
            )}
            <input
              type="text"
              className="w-full border-0 bg-transparent p-0 text-sm font-semibold text-content focus:outline-none focus:ring-0"
              value={node.data.name}
              aria-label={t('flows.nodeConfig.nameLabel')}
              placeholder={t('flows.nodeConfig.namePlaceholder')}
              data-testid="node-config-name"
              onChange={e => onChange(node.id, { name: e.target.value })}
            />
          </div>
          <button
            type="button"
            data-testid="node-config-close"
            onClick={onClose}
            aria-label={t('flows.nodeConfig.close')}
            className="shrink-0 rounded-full p-1.5 text-content-faint hover:bg-surface-hover hover:text-content-secondary">
            ✕
          </button>
        </header>

        <div className="flex-1 space-y-4 overflow-y-auto px-3.5 py-3.5">
          {/* Live, config-derived description of what this node will do. */}
          {summary && (
            <p
              className="rounded-lg border border-line bg-surface-muted px-2.5 py-1.5 text-[11px] leading-snug text-content-muted"
              data-testid="node-config-summary">
              {summary}
            </p>
          )}
          {/* Incoming/outgoing edge connections — inspect + remove them here. */}
          <NodeConnections
            nodeId={node.id}
            edges={edges}
            nodeLabelById={nodeLabelById}
            onRemoveEdge={onRemoveEdge}
          />
          {/* Keyed by node id so the JSON editor's local buffer re-seeds on switch. */}
          <NodeConfigBody
            key={node.id}
            node={node}
            onChange={onChange}
            connections={connections}
            upstreamOptions={upstreamOptions}
          />
        </div>
      </aside>
    </div>
  );
}

export default memo(NodeConfigDrawer);
