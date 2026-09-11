import Badge from '../../../components/ui/Badge';
import type { ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import { formatTimelineEntry } from '../../../utils/toolTimelineFormatting';
import type { WorkerThreadStatus } from './WorkerThreadRefCard';

/**
 * Map a parent timeline entry's status to the worker-thread lifecycle
 * phase rendered on `WorkerThreadRefCard`. The parent entry is what the
 * subagent_spawned / subagent_completed / subagent_failed socket events
 * mutate, so reading from it keeps the badge and the surrounding
 * disclosure's status pill in lockstep without a second source of truth.
 *
 * Returns `undefined` for the rare ambiguous case so the card stays
 * label-only rather than render a misleading state.
 */
export function workerStatusFromEntry(
  status: ToolTimelineEntry['status']
): WorkerThreadStatus | undefined {
  if (status === 'running') return 'running';
  if (status === 'success') return 'completed';
  if (status === 'error') return 'failed';
  return undefined;
}

/** Treat empty / structurally-empty tool bodies as absent. */
export function normalizeToolBody(value?: string): string | undefined {
  if (!value) return undefined;
  const trimmed = value.trim();
  if (trimmed.length === 0) return undefined;
  if (trimmed === '{}' || trimmed === '[]' || trimmed === 'null') return undefined;
  return value;
}

/**
 * Whether a timeline entry carries any unique body worth its own row — a
 * sub-agent's live activity, a returned result, a prompt/detail bubble, or a
 * structured failure. A row with none of these renders as a bare label + status
 * and is therefore indistinguishable from any sibling with the same title, so it
 * is safe to coalesce (see {@link coalesceTimelineEntries}). Mirrors the
 * `expandable` predicate in the row renderer so the two never disagree.
 */
export function entryHasUniqueBody(entry: ToolTimelineEntry): boolean {
  const formatted = formatTimelineEntry(entry);
  const detailContent = normalizeToolBody(formatted.detail) ?? normalizeToolBody(entry.argsBuffer);
  const resultContent = normalizeToolBody(entry.result);
  return (
    detailContent != null ||
    resultContent != null ||
    entry.subagent != null ||
    entry.failure != null
  );
}

/** A rendered timeline row: a representative entry plus how many identical,
 * body-less entries it stands in for (`count === 1` for an ordinary row). */
export interface CoalescedRow {
  entry: ToolTimelineEntry;
  count: number;
}

/**
 * Collapse runs of consecutive, identical, body-less rows into a single row
 * carrying an `×N` count. A retry loop (e.g. the orchestrator re-spawning the
 * integrations agent 25×, each surfacing the same "Checking your connected app"
 * label with no distinguishing detail) would otherwise flood the timeline with
 * indistinguishable nodes. Only truly interchangeable rows merge: same title,
 * same status, no unique body (result/detail/sub-agent/failure), and never the
 * live `running` row — so no information is lost, only duplication.
 */
export function coalesceTimelineEntries(entries: ToolTimelineEntry[]): CoalescedRow[] {
  const rows: CoalescedRow[] = [];
  for (const entry of entries) {
    const mergeable = entry.status !== 'running' && !entryHasUniqueBody(entry);
    const previous = rows[rows.length - 1];
    if (
      mergeable &&
      previous != null &&
      previous.entry.status === entry.status &&
      !entryHasUniqueBody(previous.entry) &&
      previous.entry.status !== 'running' &&
      formatTimelineEntry(previous.entry).title === formatTimelineEntry(entry).title
    ) {
      previous.count += 1;
      continue;
    }
    rows.push({ entry, count: 1 });
  }
  return rows;
}

/** Compact "×N" badge appended to a coalesced row's label. */
export function RepeatCount({ count }: { count: number }) {
  if (count <= 1) return null;
  return (
    <Badge className="shrink-0 rounded-full text-[10px]" data-testid="timeline-repeat-count">
      ×{count}
    </Badge>
  );
}
