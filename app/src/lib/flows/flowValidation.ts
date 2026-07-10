/**
 * Pure helpers for the editable Workflow Canvas's validation UX (Phase 3c).
 *
 * The `openhuman.flows_validate` RPC returns a graph-level
 * {@link FlowValidation} — a `valid` flag plus opaque `errors[]` / `warnings[]`
 * message strings (see `services/api/flowsApi.ts` and, upstream, the
 * `tinyflows::error::ValidationError` `Display` impls). Several of those error
 * strings name the offending node id inline, e.g.:
 *
 *   - `"invalid config for node <id>: <reason>"`
 *   - `"illegal cycle detected involving node: <id>"`
 *   - `"edge references unknown node id: <id>"`
 *   - `"duplicate node id: <id>"`
 *   - `"workflow has multiple trigger nodes: ["<id>", "<id>"]"`
 *
 * To highlight the culprit node(s) on the canvas we can't parse each distinct
 * message shape robustly (they're free-form host strings), so instead we test
 * every *known* node id against every error message: a node is flagged when its
 * id appears as a whole token in any error. Graph-level errors that name no
 * node (e.g. `"workflow has no trigger node"`) simply flag nothing — they still
 * surface in the banner and still block Save.
 *
 * Kept pure + dependency-free so it's trivially unit-testable and reusable.
 */

/** Escape a string for literal use inside a `RegExp`. */
function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * The subset of `nodeIds` that appear as a whole token in at least one of the
 * `errors` messages. "Whole token" boundaries treat word characters AND `-` as
 * non-boundaries (node ids are commonly hyphenated, e.g. `new-agent-0`), so a
 * shorter id embedded in a longer hyphenated id is never a false positive.
 */
export function erroredNodeIds(errors: string[], nodeIds: string[]): Set<string> {
  const flagged = new Set<string>();
  if (errors.length === 0) return flagged;
  for (const id of nodeIds) {
    if (!id) continue;
    const re = new RegExp(`(^|[^\\w-])${escapeRegExp(id)}([^\\w-]|$)`);
    if (errors.some(message => re.test(message))) {
      flagged.add(id);
    }
  }
  return flagged;
}
