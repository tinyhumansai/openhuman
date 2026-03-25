import { describe, expect, it } from 'vitest';

import { deriveSkillSyncUiState } from '../skillsSyncUi';

describe('deriveSkillSyncUiState', () => {
  it('uses explicit progress and message for gmail', () => {
    const result = deriveSkillSyncUiState('gmail', {
      syncInProgress: true,
      syncProgress: 42,
      syncProgressMessage: 'Fetching page 2...',
      totalEmails: 120,
      newEmailsCount: 5,
    });

    expect(result.isSyncing).toBe(true);
    expect(result.progressPercent).toBe(42);
    expect(result.progressMessage).toBe('Fetching page 2...');
    expect(result.metricsText).toContain('120 emails');
    expect(result.metricsText).toContain('5 new emails');
  });

  it('falls back to indeterminate mode and default message when no numeric progress', () => {
    const result = deriveSkillSyncUiState('notion', {
      syncInProgress: true,
      totalPages: 34,
      pagesWithSummary: 12,
      summariesPending: 4,
    });

    expect(result.isSyncing).toBe(true);
    expect(result.progressPercent).toBeNull();
    expect(result.progressMessage).toBe('Syncing Notion documents...');
    expect(result.metricsText).toContain('34 pages');
    expect(result.metricsText).toContain('12 pages summarized');
  });

  it('returns no sync UI when sync is idle', () => {
    const result = deriveSkillSyncUiState('google-drive', {
      syncInProgress: false,
      syncProgress: 80,
      syncProgressMessage: 'Syncing',
      totalDocuments: 11,
    });

    expect(result.isSyncing).toBe(false);
    expect(result.progressPercent).toBeNull();
    expect(result.progressMessage).toBeNull();
    expect(result.metricsText).toBeNull();
  });

  it('clamps out-of-range progress values', () => {
    const high = deriveSkillSyncUiState('gmail', { syncInProgress: true, syncProgress: 150 });
    const low = deriveSkillSyncUiState('gmail', { syncInProgress: true, syncProgress: -20 });

    expect(high.progressPercent).toBe(100);
    expect(low.progressPercent).toBe(0);
  });
});
