import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import {
  exportMimeType,
  serializeGraph,
  toCsvExport,
  toExportRows,
  toJsonExport,
} from './graphExport';

function rel(
  subject: string,
  predicate: string,
  object: string,
  namespace: string | null = 'work',
  evidenceCount = 1,
  updatedAt = 100
): GraphRelation {
  return {
    namespace,
    subject,
    predicate,
    object,
    attrs: {},
    updatedAt,
    evidenceCount,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
  };
}

describe('toExportRows', () => {
  it('projects the stable export shape and drops malformed rows', () => {
    const malformed = { ...rel('A', 'p', 'B'), predicate: null as unknown as string };
    const rows = toExportRows([rel('A', 'knows', 'B', 'work', 2, 100), malformed]);
    expect(rows).toEqual([
      {
        subject: 'A',
        predicate: 'knows',
        object: 'B',
        namespace: 'work',
        evidenceCount: 2,
        updatedAt: 100,
      },
    ]);
  });

  it('normalizes null namespace and non-finite numbers', () => {
    const rows = toExportRows([rel('A', 'p', 'B', null, Number.NaN, Number.POSITIVE_INFINITY)]);
    expect(rows[0].namespace).toBeNull();
    expect(rows[0].evidenceCount).toBe(0);
    expect(rows[0].updatedAt).toBe(0);
  });
});

describe('toJsonExport', () => {
  it('produces valid, round-trippable JSON', () => {
    const json = toJsonExport([rel('A', 'knows', 'B')]);
    const parsed = JSON.parse(json);
    expect(parsed).toHaveLength(1);
    expect(parsed[0]).toMatchObject({ subject: 'A', predicate: 'knows', object: 'B' });
  });
});

describe('toCsvExport', () => {
  it('emits a header and CRLF-terminated rows', () => {
    const csv = toCsvExport([rel('A', 'knows', 'B', 'work', 1, 0)]);
    const lines = csv.split('\r\n');
    expect(lines[0]).toBe('subject,predicate,object,namespace,evidenceCount,updatedAt');
    expect(lines[1]).toBe('A,knows,B,work,1,0');
  });

  it('renders a null namespace as an empty field', () => {
    const csv = toCsvExport([rel('A', 'knows', 'B', null, 1, 0)]);
    expect(csv.split('\r\n')[1]).toBe('A,knows,B,,1,0');
  });

  it('escapes fields containing commas, quotes, or newlines (RFC 4180)', () => {
    const csv = toCsvExport([
      rel('New York, NY', 'is_in', 'USA', 'geo', 1, 0),
      rel('he said "hi"', 'note', 'line1\nline2', 'x', 1, 0),
    ]);
    // Split on the CRLF row terminator only — the embedded bare LF inside the
    // quoted field does NOT start a new logical row.
    const lines = csv.split('\r\n');
    expect(lines).toHaveLength(3); // header + 2 rows
    expect(lines[1]).toBe('"New York, NY",is_in,USA,geo,1,0');
    // embedded quotes doubled; the newline field is quoted (LF kept inside it).
    expect(lines[2]).toBe('"he said ""hi""",note,"line1\nline2",x,1,0');
  });

  it('returns just the header for an empty graph', () => {
    expect(toCsvExport([])).toBe('subject,predicate,object,namespace,evidenceCount,updatedAt');
  });
});

describe('serializeGraph / exportMimeType', () => {
  it('dispatches to the requested format', () => {
    const relations = [rel('A', 'knows', 'B')];
    expect(serializeGraph(relations, 'json')).toBe(toJsonExport(relations));
    expect(serializeGraph(relations, 'csv')).toBe(toCsvExport(relations));
  });

  it('maps formats to MIME types', () => {
    expect(exportMimeType('json')).toBe('application/json');
    expect(exportMimeType('csv')).toBe('text/csv');
  });
});
