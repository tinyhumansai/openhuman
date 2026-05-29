/**
 * Graph Export — pure serializers for memory-graph data portability.
 *
 * Lets the user take their accumulated knowledge graph OUT of the assistant as
 * a clean JSON or CSV file. Both serializers are pure and deterministic; the
 * download side-effect lives in the container.
 *
 * Malformed rows (a non-string subject/predicate/object) are dropped. Rows are
 * emitted in input order. The CSV follows RFC 4180: a field is quoted when it
 * contains a comma, double-quote, CR, or LF, and embedded double-quotes are
 * doubled — so entity names with commas/quotes/newlines round-trip safely.
 */
import type { GraphRelation } from '../../utils/tauriCommands/memory';

export type ExportFormat = 'json' | 'csv';

export interface ExportRow {
  subject: string;
  predicate: string;
  object: string;
  namespace: string | null;
  evidenceCount: number;
  updatedAt: number;
}

const CSV_COLUMNS: Array<keyof ExportRow> = [
  'subject',
  'predicate',
  'object',
  'namespace',
  'evidenceCount',
  'updatedAt',
];

/** Keep only well-formed rows, projecting to the stable export shape. */
export function toExportRows(relations: GraphRelation[]): ExportRow[] {
  const rows: ExportRow[] = [];
  for (const relation of relations) {
    const { subject, predicate, object } = relation;
    if (
      typeof subject !== 'string' ||
      typeof predicate !== 'string' ||
      typeof object !== 'string'
    ) {
      continue;
    }
    rows.push({
      subject,
      predicate,
      object,
      namespace: typeof relation.namespace === 'string' ? relation.namespace : null,
      evidenceCount: Number.isFinite(relation.evidenceCount) ? relation.evidenceCount : 0,
      updatedAt: Number.isFinite(relation.updatedAt) ? relation.updatedAt : 0,
    });
  }
  return rows;
}

/** Pretty-printed JSON array of the export rows. */
export function toJsonExport(relations: GraphRelation[]): string {
  return JSON.stringify(toExportRows(relations), null, 2);
}

/** RFC 4180 field escaping: quote when needed, double embedded quotes. */
function escapeCsvField(value: string): string {
  if (/[",\r\n]/.test(value)) {
    return `"${value.replace(/"/g, '""')}"`;
  }
  return value;
}

/** CSV export with a header row. Line terminator is CRLF per RFC 4180. */
export function toCsvExport(relations: GraphRelation[]): string {
  const lines = [CSV_COLUMNS.join(',')];
  for (const row of toExportRows(relations)) {
    lines.push(CSV_COLUMNS.map(col => escapeCsvField(String(row[col] ?? ''))).join(','));
  }
  return lines.join('\r\n');
}

/** Serialize the graph in the requested format. */
export function serializeGraph(relations: GraphRelation[], format: ExportFormat): string {
  return format === 'csv' ? toCsvExport(relations) : toJsonExport(relations);
}

/** MIME type for a given export format. */
export function exportMimeType(format: ExportFormat): string {
  return format === 'csv' ? 'text/csv' : 'application/json';
}
