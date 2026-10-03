/**
 * Small, pure formatting helpers shared by the Memory page's tabs.
 */
import type { ItemKind, MemoryMeta } from '../../services/api/memoryApi';
import type { BadgeVariant } from '../ui';

type Translate = (key: string, fallback?: string) => string;

/** Badge tone per item kind, so the three stores read apart at a glance. */
export const KIND_VARIANT: Record<ItemKind, BadgeVariant> = {
  document: 'primary',
  conversation: 'neutral',
  learning: 'success',
};

/** An item kind's badge label. Literal keys so the i18n scanner sees them. */
export function kindLabel(kind: ItemKind, t: Translate): string {
  switch (kind) {
    case 'document':
      return t('memoryPage.kind.document');
    case 'conversation':
      return t('memoryPage.kind.conversation');
    case 'learning':
      return t('memoryPage.kind.learning');
    default:
      return String(kind);
  }
}

/** Replace `{name}` placeholders in a translated string. */
export function fill(template: string, values: Record<string, string | number>): string {
  return Object.entries(values).reduce(
    (out, [name, value]) => out.split(`{${name}}`).join(String(value)),
    template
  );
}

/** One labelled metadata fact shown under a hit. */
export interface MetaFact {
  key: 'file' | 'folder' | 'thread' | 'url' | 'repo';
  label: string;
  value: string;
}

/**
 * The metadata worth showing for a hit, in reading order: where the item lives
 * (file, folder, repo), which conversation it came from, and its link.
 */
export function metaFacts(meta: MemoryMeta | null | undefined, t: Translate): MetaFact[] {
  if (!meta) return [];
  const facts: MetaFact[] = [];
  if (meta.file_path)
    facts.push({ key: 'file', label: t('memoryPage.meta.file'), value: meta.file_path });
  else if (meta.folder)
    facts.push({ key: 'folder', label: t('memoryPage.meta.folder'), value: meta.folder });
  if (meta.repo) facts.push({ key: 'repo', label: t('memoryPage.meta.repo'), value: meta.repo });
  if (meta.thread_id)
    facts.push({ key: 'thread', label: t('memoryPage.meta.thread'), value: meta.thread_id });
  if (meta.url) facts.push({ key: 'url', label: t('memoryPage.meta.url'), value: meta.url });
  return facts;
}

/** A timestamp as a short local date-time, or `null` for a missing/invalid one. */
export function formatTimestamp(value: string | null | undefined): string | null {
  if (!value) return null;
  const ms = Date.parse(value);
  if (Number.isNaN(ms)) return null;
  return new Date(ms).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

/** A relevance score rendered to two decimals. */
export function formatScore(score: number | null | undefined): string | null {
  if (typeof score !== 'number' || !Number.isFinite(score)) return null;
  return score.toFixed(2);
}

/** Parse a positive integer from a form field, or `null` when it is not one. */
export function parsePositiveInt(raw: string): number | null {
  if (!/^\s*\d+\s*$/.test(raw)) return null;
  const n = Number.parseInt(raw, 10);
  return n > 0 ? n : null;
}
