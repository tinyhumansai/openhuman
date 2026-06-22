/**
 * Shared path-string helpers for display in the UI.
 *
 * These operate on plain strings (not the filesystem) so they work for both
 * POSIX and Windows-style paths surfaced by the Rust core.
 */

/**
 * Last path segment of a path string, for a compact label.
 *
 * Trims trailing slashes/backslashes, then returns everything after the final
 * `/` or `\`. Returns the (trimmed) input unchanged when it has no separator.
 */
export function basename(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, '');
  const idx = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'));
  return idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
}
