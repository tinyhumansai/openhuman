import type { GraphNode } from '../../utils/tauriCommands';

export const MEMORY_CONTENT_WORKSPACE_PATH = 'memory_tree/content';

/** Mirror of `paths::slugify_source_id` (Rust). */
function slugify(s: string): string {
  const lower = s.toLowerCase();
  let out = '';
  let lastDash = true;
  let pendingUnderscore = false;
  for (const ch of lower) {
    if (ch === '_') {
      if (!lastDash) pendingUnderscore = true;
    } else if (/[a-z0-9]/.test(ch)) {
      if (pendingUnderscore) {
        out += '_';
        pendingUnderscore = false;
      }
      out += ch;
      lastDash = false;
    } else {
      pendingUnderscore = false;
      if (!lastDash) {
        out += '-';
        lastDash = true;
      }
    }
  }
  return out.replace(/[-_]+$/, '').slice(0, 120) || 'unknown';
}

function dateFromMs(ms?: number): string {
  if (typeof ms !== 'number' || !Number.isFinite(ms)) return 'unknown-date';
  const date = new Date(ms);
  if (Number.isNaN(date.getTime())) return 'unknown-date';
  return date.toISOString().slice(0, 10);
}

function joinWorkspaceContentPath(relPath: string): string {
  const rel = relPath.replace(/^\/+/, '');
  return rel ? `${MEMORY_CONTENT_WORKSPACE_PATH}/${rel}` : MEMORY_CONTENT_WORKSPACE_PATH;
}

function summaryScopeSlug(node: GraphNode): string {
  const scope = node.tree_scope ?? '';
  if (node.tree_kind === 'source' && scope.startsWith('gmail:')) {
    return slugify(scope.slice('gmail:'.length));
  }
  return slugify(scope);
}

export function summaryWorkspacePath(node: GraphNode): string | null {
  if (node.kind !== 'summary' || !node.tree_kind || node.level == null || !node.file_basename) {
    return null;
  }

  const rel =
    node.tree_kind === 'global'
      ? `wiki/summaries/global-${dateFromMs(node.time_range_start_ms)}/L${node.level}/${node.file_basename}.md`
      : `wiki/summaries/${node.tree_kind}-${summaryScopeSlug(node)}/L${node.level}/${node.file_basename}.md`;

  return joinWorkspaceContentPath(rel);
}
