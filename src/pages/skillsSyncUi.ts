import type { SkillHostConnectionState } from '../lib/skills/types';

export interface SkillSyncUiState {
  isSyncing: boolean;
  progressPercent: number | null;
  progressMessage: string | null;
  metricsText: string | null;
}

type SkillStateRecord = SkillHostConnectionState & Record<string, unknown>;

function readNumber(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string' && value.trim() !== '') {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

function readBoolean(value: unknown): boolean | null {
  return typeof value === 'boolean' ? value : null;
}

function clampPercent(value: number): number {
  if (value < 0) return 0;
  if (value > 100) return 100;
  return value;
}

function formatNumber(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

function buildMetricsText(state: SkillStateRecord): string | null {
  const values = {
    newEmailsCount: readNumber(state.newEmailsCount),
    totalEmails: readNumber(state.totalEmails),
    totalDocuments: readNumber(state.totalDocuments),
    totalPages: readNumber(state.totalPages),
    pagesWithSummary: readNumber(state.pagesWithSummary),
    summariesPending: readNumber(state.summariesPending),
    totalFiles: readNumber(state.totalFiles),
    itemsDone: readNumber(state.itemsDone),
    itemsTotal: readNumber(state.itemsTotal),
  };

  const parts: string[] = [];
  if (values.newEmailsCount != null) parts.push(`${formatNumber(values.newEmailsCount)} new emails`);
  if (values.totalEmails != null) parts.push(`${formatNumber(values.totalEmails)} emails`);
  if (values.totalDocuments != null) parts.push(`${formatNumber(values.totalDocuments)} docs`);
  if (values.totalPages != null) parts.push(`${formatNumber(values.totalPages)} pages`);
  if (values.pagesWithSummary != null)
    parts.push(`${formatNumber(values.pagesWithSummary)} pages summarized`);
  if (values.summariesPending != null)
    parts.push(`${formatNumber(values.summariesPending)} summaries pending`);
  if (values.totalFiles != null) parts.push(`${formatNumber(values.totalFiles)} files`);
  if (values.itemsDone != null && values.itemsTotal != null && values.itemsTotal > 0) {
    parts.push(`${formatNumber(values.itemsDone)}/${formatNumber(values.itemsTotal)} items`);
  }

  if (parts.length === 0) return null;
  return parts.slice(0, 3).join(' · ');
}

function defaultProgressMessage(skillId: string): string {
  if (skillId === 'gmail') return 'Syncing emails...';
  if (skillId === 'google-drive') return 'Syncing documents...';
  if (skillId === 'notion') return 'Syncing Notion documents...';
  return 'Syncing...';
}

export function deriveSkillSyncUiState(
  skillId: string,
  skillState: SkillStateRecord | undefined
): SkillSyncUiState {
  if (!skillState) {
    return {
      isSyncing: false,
      progressPercent: null,
      progressMessage: null,
      metricsText: null,
    };
  }

  const isSyncing = readBoolean(skillState.syncInProgress) === true;

  const explicitProgress =
    readNumber(skillState.syncProgress) ??
    readNumber(skillState.progressPercent) ??
    readNumber(skillState.progress);

  const itemDone = readNumber(skillState.itemsDone);
  const itemTotal = readNumber(skillState.itemsTotal);
  const ratioProgress =
    explicitProgress == null && itemDone != null && itemTotal != null && itemTotal > 0
      ? (itemDone / itemTotal) * 100
      : null;

  const progressPercent =
    explicitProgress != null
      ? clampPercent(explicitProgress)
      : ratioProgress != null
        ? clampPercent(ratioProgress)
        : null;

  const progressMessageRaw =
    typeof skillState.syncProgressMessage === 'string' ? skillState.syncProgressMessage.trim() : '';

  return {
    isSyncing,
    progressPercent: isSyncing ? progressPercent : null,
    progressMessage: isSyncing ? progressMessageRaw || defaultProgressMessage(skillId) : null,
    metricsText: isSyncing ? buildMetricsText(skillState) : null,
  };
}
