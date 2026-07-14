import createDebug from 'debug';

/**
 * runItems — normalize a `tinyflows` run step's opaque `output` into the
 * n8n-style item-array shape the run inspector's per-item data browser
 * (Phase 6) renders.
 *
 * `FlowRunStep.output` is `unknown` on the wire (see `services/api/flowsApi.ts`)
 * because the durable run record stores whatever the node emitted. The n8n data
 * model each node produces is an array of items, each `{ json, binary?,
 * paired_item? }`:
 *   - `json`        — the item's data payload (usually an object).
 *   - `binary`      — a map of named binary attachments (never inlined in the
 *                     UI; shown as placeholder chips).
 *   - `paired_item` — links this output item back to the input item it derived
 *                     from, so the inspector can reveal the source input.
 *
 * Real runs are messier than that ideal, so this normalizer is deliberately
 * forgiving: a bare object/primitive, a single item, or a full item array all
 * normalize into `FlowRunItem[]`. Anything it can't interpret as item-shaped is
 * treated as a single item whose `json` is the raw value — never throws.
 */
const log = createDebug('app:flows:items');

/** A single binary attachment reference (metadata only — bytes never inlined). */
export interface FlowBinaryRef {
  /** Property name of this attachment in the item's `binary` map. */
  key: string;
  /** Original file name, if the node recorded one. */
  fileName?: string;
  /** MIME type, if the node recorded one. */
  mimeType?: string;
}

/** One normalized output item of a run step. */
export interface FlowRunItem {
  /** The item's `json` data payload (any JSON value; usually an object). */
  json: unknown;
  /** Binary attachments declared on the item (metadata only). */
  binary: FlowBinaryRef[];
  /**
   * Zero-based index of the input item this output derived from, resolved from
   * `paired_item`, or `null` when the item carries no pairing hint.
   */
  pairedIndex: number | null;
}

export function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/**
 * Resolve a `paired_item` field to a single source input index. n8n allows a
 * bare number, a `{ item }` object, or an array of either (fan-in) — we take
 * the first resolvable index and ignore the rest (the UI reveals one source).
 */
function resolvePairedIndex(raw: unknown): number | null {
  if (typeof raw === 'number' && Number.isInteger(raw) && raw >= 0) return raw;
  if (isPlainObject(raw)) {
    const item = raw.item;
    if (typeof item === 'number' && Number.isInteger(item) && item >= 0) return item;
    return null;
  }
  if (Array.isArray(raw)) {
    for (const entry of raw) {
      const resolved = resolvePairedIndex(entry);
      if (resolved !== null) return resolved;
    }
  }
  return null;
}

/** Parse an item's `binary` map into placeholder-chip metadata. */
function parseBinary(raw: unknown): FlowBinaryRef[] {
  if (!isPlainObject(raw)) return [];
  return Object.entries(raw).map(([key, value]) => {
    const meta = isPlainObject(value) ? value : {};
    const fileName = meta.fileName ?? meta.file_name;
    const mimeType = meta.mimeType ?? meta.mime_type;
    return {
      key,
      fileName: typeof fileName === 'string' ? fileName : undefined,
      mimeType: typeof mimeType === 'string' ? mimeType : undefined,
    };
  });
}

/**
 * Known keys of the internal `{ json, raw, text }` payload envelope some nodes
 * (Composio tool calls in particular) persist alongside the n8n item shape —
 * `json` is the parsed payload, `raw` is the same payload pre-parse (always a
 * verbatim duplicate in practice), `text` an optional plain-text fallback
 * (usually `null`). Rendered naked, `json` and `raw` show the identical data
 * twice side by side (issue B19). An object is only treated as this envelope
 * when its keys are a subset of these three AND it actually carries `json` —
 * a real payload that merely happens to have a field called "raw" but no
 * "json" is left untouched.
 */
const PAYLOAD_ENVELOPE_KEYS = new Set(['json', 'raw', 'text']);

/**
 * Structural equality for JSON-like values (objects/arrays/primitives). Used
 * to prove a sibling envelope field (`raw`/`text`) is actually a duplicate of
 * the selected value before we discard it — see {@link unwrapPayloadEnvelope}.
 */
function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    return a.every((item, index) => deepEqual(item, b[index]));
  }
  if (isPlainObject(a) && isPlainObject(b)) {
    const aKeys = Object.keys(a);
    const bKeys = Object.keys(b);
    if (aKeys.length !== bKeys.length) return false;
    return aKeys.every(
      key => Object.prototype.hasOwnProperty.call(b, key) && deepEqual(a[key], b[key])
    );
  }
  return false;
}

/**
 * A sibling envelope field is safe to discard when it's absent/null, or when
 * it's a proven structural duplicate of the value we're keeping. Anything
 * else means the sibling carries distinct, meaningful data.
 */
function isDuplicateOrEmpty(sibling: unknown, kept: unknown): boolean {
  return sibling === undefined || sibling === null || deepEqual(sibling, kept);
}

/**
 * Collapse the internal `{ json, raw, text }` payload envelope (see
 * {@link PAYLOAD_ENVELOPE_KEYS}) down to a single canonical value, so the
 * inspector renders one copy of the data instead of the same payload twice
 * under sibling `json`/`raw` keys. Anything that doesn't match the envelope
 * shape passes through unchanged.
 *
 * A node can intentionally return a payload shaped exactly like this envelope
 * (e.g. parsed content plus a genuinely different raw body) — collapsing
 * unconditionally would silently drop that data. So we only ever discard a
 * sibling field when it's absent/null or a proven duplicate of the value
 * we're keeping ({@link isDuplicateOrEmpty}); otherwise the object is left
 * intact.
 */
function unwrapPayloadEnvelope(value: unknown): unknown {
  if (!isPlainObject(value)) {
    log('unwrapPayloadEnvelope: non-object value (type=%s) — pass through', typeof value);
    return value;
  }
  const keys = Object.keys(value);
  if (keys.length === 0 || !keys.every(key => PAYLOAD_ENVELOPE_KEYS.has(key))) {
    log('unwrapPayloadEnvelope: non-envelope object (keys=%o) — pass through', keys);
    return value;
  }
  if (!('json' in value)) {
    log('unwrapPayloadEnvelope: envelope-shaped keys=%o but missing `json` — pass through', keys);
    return value;
  }
  if (value.json !== undefined && value.json !== null) {
    if (isDuplicateOrEmpty(value.raw, value.json) && isDuplicateOrEmpty(value.text, value.json)) {
      log('unwrapPayloadEnvelope: envelope keys=%o — selected `json` branch', keys);
      return value.json;
    }
    log(
      'unwrapPayloadEnvelope: envelope keys=%o — `raw`/`text` carry distinct data, not collapsing',
      keys
    );
    return value;
  }
  if (value.raw !== undefined && value.raw !== null) {
    if (isDuplicateOrEmpty(value.text, value.raw)) {
      log('unwrapPayloadEnvelope: envelope keys=%o — `json` empty, selected `raw` branch', keys);
      return value.raw;
    }
    log(
      'unwrapPayloadEnvelope: envelope keys=%o — `json` empty but `text` carries distinct data, not collapsing',
      keys
    );
    return value;
  }
  if (value.text !== undefined && value.text !== null) {
    log(
      'unwrapPayloadEnvelope: envelope keys=%o — `json`/`raw` empty, selected `text` branch',
      keys
    );
    return value.text;
  }
  log('unwrapPayloadEnvelope: envelope keys=%o — all fields null, falling back to `json`', keys);
  return value.json;
}

/** Normalize one raw element into a {@link FlowRunItem}. */
function toItem(raw: unknown): FlowRunItem {
  // Item-shaped: `{ json, binary?, paired_item? }`. `json` present as an own key
  // is the discriminant — a plain data object without it is treated as the
  // payload itself (see below).
  if (isPlainObject(raw) && 'json' in raw) {
    log('toItem: item-shaped input (has `json` key) — selected item branch');
    return {
      json: unwrapPayloadEnvelope(raw.json),
      binary: parseBinary(raw.binary),
      pairedIndex: resolvePairedIndex(raw.paired_item),
    };
  }
  log('toItem: payload-shaped input (type=%s) — wrapping raw value as item json', typeof raw);
  return { json: unwrapPayloadEnvelope(raw), binary: [], pairedIndex: null };
}

/**
 * Normalize a run step's `output` into an array of {@link FlowRunItem}. Returns
 * `[]` for `null`/`undefined` output; wraps a single value as one item.
 */
export function normalizeItems(output: unknown): FlowRunItem[] {
  if (output === null || output === undefined) return [];
  if (Array.isArray(output)) return output.map(toItem);
  return [toItem(output)];
}

/**
 * Union of the `json` object keys across all items, in first-seen order — the
 * column set for the table view. Items whose `json` is not a plain object
 * contribute no columns (they render in a synthetic single-value column).
 */
export function collectColumns(items: FlowRunItem[]): string[] {
  const seen = new Set<string>();
  const columns: string[] = [];
  for (const item of items) {
    if (!isPlainObject(item.json)) continue;
    for (const key of Object.keys(item.json)) {
      if (!seen.has(key)) {
        seen.add(key);
        columns.push(key);
      }
    }
  }
  return columns;
}

/** True when at least one item's `json` is a plain object (table has columns). */
export function hasObjectRows(items: FlowRunItem[]): boolean {
  return items.some(item => isPlainObject(item.json));
}

/** Read a single column's value from an item's object `json` (undefined if absent). */
export function cellValue(item: FlowRunItem, column: string): unknown {
  return isPlainObject(item.json) ? item.json[column] : undefined;
}

/**
 * Render a value for a table cell: primitives verbatim, objects/arrays as
 * compact JSON, `undefined` as an empty string (missing column for this item).
 */
export function formatCell(value: unknown): string {
  if (value === undefined) return '';
  if (value === null) return 'null';
  if (typeof value === 'string') return value;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

/** Pretty-print an item's `json` payload for the JSON view / source panel. */
export function formatJson(value: unknown): string {
  if (value === undefined) return '';
  if (typeof value === 'string') return value;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}
