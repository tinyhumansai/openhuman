/**
 * Workflow export (Phase 4d) — pure client-side serialization of a flow's
 * `WorkflowGraph` into a downloadable JSON file. No RPC: the frontend already
 * holds the graph (from `flows_list` / `flows_get`), so exporting is just
 * serialize + trigger a browser download. The counterpart import path
 * (`flowsApi.importFlow`) is host-validated because migrate/validate must be
 * authoritative; export has no such constraint.
 */
import createDebug from 'debug';

const log = createDebug('app:flows:export');

/**
 * Turns a flow name into a safe, lower-kebab file stem. Non-alphanumeric runs
 * collapse to a single `-`; empty results fall back to `workflow`.
 */
export function exportFileName(name: string): string {
  const stem = name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
  return `${stem || 'workflow'}.flow.json`;
}

/**
 * Serializes a flow's `graph` to pretty-printed JSON. Kept separate from the
 * download side effect so it's trivially unit-testable.
 */
export function serializeFlowGraph(graph: unknown): string {
  return `${JSON.stringify(graph, null, 2)}\n`;
}

/**
 * Downloads a flow's `WorkflowGraph` as a `<name>.flow.json` file via a
 * transient object-URL anchor. Guarded for non-DOM environments (SSR / tests
 * without a document) — returns `false` when it can't run, `true` once the
 * download is triggered.
 */
export function downloadFlowGraph(name: string, graph: unknown): boolean {
  if (typeof document === 'undefined' || typeof URL === 'undefined' || !URL.createObjectURL) {
    log('downloadFlowGraph: no DOM/URL available — skipping');
    return false;
  }
  const fileName = exportFileName(name);
  log('downloadFlowGraph: name=%s file=%s', name, fileName);
  const blob = new Blob([serializeFlowGraph(graph)], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  try {
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = fileName;
    document.body.appendChild(anchor);
    anchor.click();
    document.body.removeChild(anchor);
  } finally {
    URL.revokeObjectURL(url);
  }
  return true;
}
