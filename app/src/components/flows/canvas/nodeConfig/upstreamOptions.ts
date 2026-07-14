/**
 * Upstream-output picker options for the node-config drawer (`nodes` scope).
 *
 * The tinyflows engine exposes every upstream node's output to expressions as
 * `=nodes.<node_id>.item` (and `=nodes.<node_id>.item.<field>`). This module
 * walks the graph's edges backward from the selected node to find its
 * transitive ancestors and turns each into insertable expression options,
 * labelled with the node's human name so authors pick "Extract recipient →
 * email" instead of hand-typing node ids.
 *
 * Per-field options are only offered where the upstream node's output shape is
 * statically knowable from its config:
 *  - `agent` with `config.output_parser.schema.properties` → one per property;
 *  - `transform` with `config.set` → one per set key;
 *  - everything else → just the node-level `item` option.
 */
import createDebug from 'debug';

import type { FlowEdge, FlowNode } from '../../../../lib/flows/graphAdapter';
import type { NodeKind } from '../../../../lib/flows/types';

const log = createDebug('app:flows:nodeConfig:upstreamOptions');

/** One insertable `=nodes.…` expression, labelled for the picker dropdown. */
export interface UpstreamExpressionOption {
  /** The full expression to insert, e.g. `=nodes.extract.item.email`. */
  value: string;
  /** Human label, e.g. "Extract recipient → email". */
  label: string;
}

/** Narrow an unknown to a plain (non-array) object, else `null`. */
function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/**
 * Output field names statically knowable from an upstream node's config.
 * Empty when the shape can't be derived — the caller then offers only the
 * node-level `item` option.
 */
function knownOutputFields(kind: NodeKind, config: Record<string, unknown>): string[] {
  if (kind === 'agent') {
    const schema = asRecord(asRecord(config.output_parser)?.schema);
    const properties = asRecord(schema?.properties);
    return properties ? Object.keys(properties) : [];
  }
  if (kind === 'transform') {
    const set = asRecord(config.set);
    return set ? Object.keys(set) : [];
  }
  return [];
}

/**
 * Build the `=nodes.…` picker options for the node identified by `nodeId`:
 * a breadth-first backward walk over `edges` collects the node's transitive
 * upstream ancestors (nearest first, cycle-safe), then each ancestor yields a
 * node-level option plus one option per statically-known output field.
 */
export function upstreamExpressionOptions(
  nodeId: string,
  nodes: FlowNode[],
  edges: FlowEdge[]
): UpstreamExpressionOption[] {
  const sourcesByTarget = new Map<string, string[]>();
  for (const edge of edges) {
    const sources = sourcesByTarget.get(edge.target) ?? [];
    sources.push(edge.source);
    sourcesByTarget.set(edge.target, sources);
  }

  // BFS backward from the selected node — ancestors surface nearest-first.
  const visited = new Set<string>([nodeId]);
  const ancestors: string[] = [];
  const queue: string[] = [nodeId];
  while (queue.length > 0) {
    const current = queue.shift();
    if (current === undefined) break;
    for (const source of sourcesByTarget.get(current) ?? []) {
      if (visited.has(source)) continue;
      visited.add(source);
      ancestors.push(source);
      queue.push(source);
    }
  }

  const nodeById = new Map(nodes.map(node => [node.id, node]));
  const options: UpstreamExpressionOption[] = [];
  for (const id of ancestors) {
    const node = nodeById.get(id);
    if (!node) continue;
    const name = node.data.name.trim() || id;
    options.push({ value: `=nodes.${id}.item`, label: `${name} → item` });
    for (const field of knownOutputFields(node.data.kind, node.data.config ?? {})) {
      options.push({ value: `=nodes.${id}.item.${field}`, label: `${name} → ${field}` });
    }
  }
  log(
    'upstreamExpressionOptions: node=%s ancestors=%d options=%d',
    nodeId,
    ancestors.length,
    options.length
  );
  return options;
}
