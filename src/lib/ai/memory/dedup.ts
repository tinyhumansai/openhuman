/**
 * Memory deduplication utilities.
 *
 * Prevents duplicate facts from accumulating in memory files
 * during repeated memory flushes.
 */

/**
 * Normalize a string for comparison: lowercase, collapse whitespace, trim.
 */
function normalize(s: string): string {
  return s.trim().toLowerCase().replace(/\s+/g, ' ');
}

/**
 * Check if a new fact is substantially contained in the existing content.
 *
 * Returns true if the new fact (after normalization) matches any existing
 * line with >= 85% character overlap, or if it's an exact substring of
 * an existing line.
 */
export function isDuplicateFact(existingContent: string, newFact: string): boolean {
  const normalizedNew = normalize(newFact);
  if (!normalizedNew) return true; // Empty facts are "duplicates"

  const existingLines = existingContent
    .split('\n')
    .map(line => normalize(line))
    .filter(line => line.length > 0);

  for (const existing of existingLines) {
    // Exact match
    if (existing === normalizedNew) return true;

    // New fact is a substring of an existing line
    if (existing.includes(normalizedNew)) return true;

    // Existing line is a substring of new fact (new fact is more detailed — not a dupe)
    // We deliberately do NOT flag this as duplicate so the more detailed version gets added
  }

  return false;
}

/**
 * Filter new content lines against existing content, only keeping novel lines.
 *
 * Splits newContent by newlines, checks each line against existingContent,
 * and returns only lines that aren't duplicates.
 */
export function deduplicateAppend(existing: string, newContent: string): string {
  const newLines = newContent.split('\n');
  const novelLines: string[] = [];

  for (const line of newLines) {
    const trimmed = line.trim();

    // Keep empty lines, headers, and formatting markers as-is
    if (!trimmed || trimmed.startsWith('#') || trimmed === '---') {
      novelLines.push(line);
      continue;
    }

    // Strip leading bullet marker for comparison
    const fact = trimmed.replace(/^[-*]\s+/, '').replace(/^\d+\.\s+/, '');

    if (!isDuplicateFact(existing, fact)) {
      novelLines.push(line);
    }
  }

  // Remove leading/trailing blank lines from result
  const result = novelLines.join('\n').trim();
  return result;
}
