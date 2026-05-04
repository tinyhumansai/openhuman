const ROW_REGEX = /^\| (\d+(?:\.\d+){2,3}) \| ([^|]+) \| ([^|]+) \| ([^|]+) \| ([^|]+) \| ([^|]+) \|/;
const ID_REGEX = /^\d+(?:\.\d+){2,3}$/;
const VALID_STATUS = new Set(['✅', '🟡', '❌', '🚫']);

export function parseMatrix(markdown) {
  const rows = [];
  const errors = [];
  if (typeof markdown !== 'string') {
    return { rows, errors };
  }
  for (const line of markdown.split(/\r?\n/)) {
    const match = line.match(ROW_REGEX);
    if (!match) continue;
    const [, id, name, layer, path, status, notes] = match.map((v) => (typeof v === 'string' ? v.trim() : v));
    if (!ID_REGEX.test(id)) {
      errors.push(`Invalid ID format: ${id}`);
      continue;
    }
    if (!VALID_STATUS.has(status)) {
      errors.push(`Row ${id}: invalid status "${status}" (must be one of ${[...VALID_STATUS].join(' ')})`);
      continue;
    }
    rows.push({ id, name, layer, path, status, notes });
  }
  return { rows, errors };
}

export function validateAgainstCatalog(parsedRows, catalogIds) {
  const seen = new Map();
  for (const row of parsedRows) {
    seen.set(row.id, (seen.get(row.id) ?? 0) + 1);
  }
  const duplicates = [...seen.entries()].filter(([, count]) => count > 1).map(([id]) => id);
  const present = new Set(seen.keys());
  const missingFromMatrix = [...catalogIds].filter((id) => !present.has(id));
  return { missingFromMatrix, duplicates };
}
