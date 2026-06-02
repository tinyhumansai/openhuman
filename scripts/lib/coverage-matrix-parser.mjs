const ROW_REGEX =
  /^\|\s*(\d+(?:\.\d+){2,3})\s*\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|\s*([^|]+?)\s*\|\s*([^|]*?)\s*\|\s*$/u;

const ID_REGEX = /^\d+(?:\.\d+){2,3}$/;

const VALID_STATUS = new Set(["✅", "🟡", "❌", "🚫"]);

export function parseMatrix(markdown) {
  if (typeof markdown !== "string") {
    return {
      rows: [],
      errors: ["Input must be a string"],
    };
  }

  const rows = [];
  const errors = [];

  const lines = markdown.split(/\r?\n/);

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    const match = ROW_REGEX.exec(line);
    if (!match) continue;

    const [, id, name, layer, path, rawStatus, notes] = match;

    const status = rawStatus.trim();

    if (!ID_REGEX.test(id)) {
      errors.push(`Line ${i + 1}: Invalid ID format "${id}"`);
      continue;
    }

    if (!VALID_STATUS.has(status)) {
      errors.push(`Line ${i + 1} (${id}): invalid status "${status}"`);
      continue;
    }

    rows.push({
      id,
      name: name.trim(),
      layer: layer.trim(),
      path: path.trim(),
      status,
      notes: notes.trim(),
    });
  }

  return { rows, errors };
}

export function validateAgainstCatalog(rows, catalogIds) {
  const counts = new Map();

  for (const { id } of rows) {
    counts.set(id, (counts.get(id) ?? 0) + 1);
  }

  const duplicates = [];

  for (const [id, count] of counts) {
    if (count > 1) {
      duplicates.push(id);
    }
  }

  const catalogSet =
    catalogIds instanceof Set ? catalogIds : new Set(catalogIds);

  const missingFromMatrix = [];

  for (const id of catalogSet) {
    if (!counts.has(id)) {
      missingFromMatrix.push(id);
    }
  }

  return {
    missingFromMatrix,
    duplicates,
    totalRows: rows.length,
    uniqueRows: counts.size,
  };
}
