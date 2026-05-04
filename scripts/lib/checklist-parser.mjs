const ITEM_REGEX = /^- \[( |x|X)\] (.*)$/;
const NA_REGEX = /^\s*(?:\(N\/A\)|N\/A\b)[:\s.-]*(.*)$/i;

export function parseChecklist(body) {
  const items = [];
  if (typeof body !== 'string' || body.length === 0) {
    return { items, totalUnchecked: 0 };
  }
  let inFence = false;
  for (const rawLine of body.split(/\r?\n/)) {
    if (/^\s*```/.test(rawLine)) {
      inFence = !inFence;
      continue;
    }
    if (inFence) continue;
    const match = rawLine.match(ITEM_REGEX);
    if (!match) continue;
    const checked = match[1] === 'x' || match[1] === 'X';
    const text = match[2].trim();
    const naMatch = text.match(NA_REGEX);
    const naReason = naMatch ? naMatch[1].trim() || null : null;
    items.push({ checked, naReason, text });
  }
  const totalUnchecked = items.filter((i) => !i.checked && i.naReason === null).length;
  return { items, totalUnchecked };
}

export function summarize(parsed) {
  const total = parsed.items.length;
  const naCount = parsed.items.filter((i) => i.naReason !== null).length;
  const satisfied = parsed.items.filter((i) => i.checked || i.naReason !== null).length;
  const lines = [`Checklist: ${satisfied}/${total} items satisfied (${parsed.totalUnchecked} unchecked, ${naCount} N/A)`];
  if (parsed.totalUnchecked > 0) {
    lines.push('Unchecked items requiring action:');
    for (const item of parsed.items) {
      if (!item.checked && item.naReason === null) {
        lines.push(`  - ${item.text}`);
      }
    }
  }
  return lines.join('\n');
}
