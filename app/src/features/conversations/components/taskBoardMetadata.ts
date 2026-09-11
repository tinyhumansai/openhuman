/**
 * Pure helpers shared by the task-board surfaces: reading the loosely-typed
 * `sourceMetadata` blob off a card, labelling providers, and the small
 * text/line conversions the brief editor round-trips through.
 *
 * Extracted verbatim out of `TaskKanbanBoard.tsx` when that file was split —
 * no behaviour changed, only the location.
 */
import type { FetchOutcome } from '../../../utils/tauriCommands';

export interface TaskSourceMetadata {
  provider?: string;
  sourceId?: string;
  externalId?: string;
  url?: string;
  repo?: string;
  urgency?: number;
}

export function readString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined;
}

export function readNumber(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string') {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return undefined;
}

export function readSourceMetadata(
  value: Record<string, unknown> | null | undefined
): TaskSourceMetadata | null {
  if (!value || typeof value !== 'object') return null;
  const provider = readString(value.provider);
  const sourceId = readString(value.source_id) ?? readString(value.sourceId);
  const externalId = readString(value.external_id) ?? readString(value.externalId);
  const url = readString(value.url);
  const repo = readString(value.repo);
  const urgency = readNumber(value.urgency);
  if (!provider && !sourceId && !externalId && !url && !repo && urgency === undefined) {
    return null;
  }
  return { provider, sourceId, externalId, url, repo, urgency };
}

export function providerLabel(provider: string | undefined, t: (key: string) => string): string {
  switch (provider) {
    case 'github':
      return t('settings.taskSources.providers.github');
    case 'notion':
      return t('settings.taskSources.providers.notion');
    case 'linear':
      return t('settings.taskSources.providers.linear');
    case 'clickup':
      return t('settings.taskSources.providers.clickup');
    default:
      return provider ?? t('conversations.taskKanban.source.unknownProvider');
  }
}

export function sourceBadgeLabel(source: TaskSourceMetadata, t: (key: string) => string): string {
  const provider = providerLabel(source.provider, t);
  if (source.repo && source.externalId) return `${provider} · ${source.repo}#${source.externalId}`;
  if (source.externalId) return `${provider} · ${source.externalId}`;
  return provider;
}

export function formatUrgency(
  urgency: number | undefined,
  t: (key: string) => string
): string | undefined {
  if (urgency === undefined) return undefined;
  const percent = Math.round(Math.max(0, Math.min(1, urgency)) * 100);
  return t('conversations.taskKanban.source.urgencyValue').replace('{percent}', String(percent));
}

export function formatFetchNotice(outcome: FetchOutcome, t: (key: string) => string): string {
  return t('settings.taskSources.fetchResult')
    .replace('{routed}', String(outcome.routed))
    .replace('{fetched}', String(outcome.fetched))
    .replace('{pruned}', String(outcome.pruned ?? 0));
}

export function formatSyncNotice(outcomes: FetchOutcome[], t: (key: string) => string): string {
  const totals = outcomes.reduce(
    (acc, outcome) => ({
      fetched: acc.fetched + outcome.fetched,
      routed: acc.routed + outcome.routed,
      pruned: acc.pruned + (outcome.pruned ?? 0),
    }),
    { fetched: 0, routed: 0, pruned: 0 }
  );
  return t('settings.taskSources.fetchResult')
    .replace('{routed}', String(totals.routed))
    .replace('{fetched}', String(totals.fetched))
    .replace('{pruned}', String(totals.pruned));
}

export function joinLines(values?: string[]): string {
  return values?.join('\n') ?? '';
}

export function splitLines(value: string): string[] {
  return value
    .split('\n')
    .map(line => line.trim())
    .filter(Boolean);
}

export function emptyToNull(value: string): string | null {
  const trimmed = value.trim();
  return trimmed ? trimmed : null;
}
