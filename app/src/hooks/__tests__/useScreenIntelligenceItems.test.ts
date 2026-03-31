import { describe, expect, it } from 'vitest';

import type { AccessibilityVisionSummary } from '../../utils/tauriCommands';

// Test the mapping logic directly (extracted from the hook for testability)
function confidenceToPriority(confidence: number): 'critical' | 'important' | 'normal' {
  if (confidence > 0.9) return 'critical';
  if (confidence > 0.7) return 'important';
  return 'normal';
}

function mapSummaryToItem(summary: AccessibilityVisionSummary) {
  return {
    id: `si-${summary.id}`,
    title: summary.actionable_notes.slice(0, 120),
    description: [summary.ui_state, summary.key_text].filter(Boolean).join(' - '),
    source: 'ai_insight' as const,
    priority: confidenceToPriority(summary.confidence),
    status: 'active' as const,
    createdAt: new Date(summary.captured_at_ms),
    updatedAt: new Date(summary.captured_at_ms),
    actionable: true,
    sourceLabel: summary.app_name ?? 'Screen Intelligence',
  };
}

const makeSummary = (
  overrides: Partial<AccessibilityVisionSummary> = {}
): AccessibilityVisionSummary => ({
  id: 'vision-123',
  captured_at_ms: 1700000000000,
  app_name: 'Safari',
  window_title: 'GitHub',
  ui_state: 'editor open',
  key_text: 'fn main()',
  actionable_notes: 'Consider adding tests',
  confidence: 0.85,
  ...overrides,
});

describe('useScreenIntelligenceItems mapping', () => {
  it('maps VisionSummary to ActionableItem correctly', () => {
    const summary = makeSummary();
    const item = mapSummaryToItem(summary);

    expect(item.id).toBe('si-vision-123');
    expect(item.title).toBe('Consider adding tests');
    expect(item.description).toBe('editor open - fn main()');
    expect(item.source).toBe('ai_insight');
    expect(item.priority).toBe('important');
    expect(item.status).toBe('active');
    expect(item.sourceLabel).toBe('Safari');
    expect(item.actionable).toBe(true);
  });

  it('handles empty array', () => {
    const items: AccessibilityVisionSummary[] = [];
    const mapped = items.map(mapSummaryToItem);
    expect(mapped).toEqual([]);
  });

  it('derives critical priority from high confidence', () => {
    const item = mapSummaryToItem(makeSummary({ confidence: 0.95 }));
    expect(item.priority).toBe('critical');
  });

  it('derives normal priority from low confidence', () => {
    const item = mapSummaryToItem(makeSummary({ confidence: 0.5 }));
    expect(item.priority).toBe('normal');
  });

  it('derives important priority from medium confidence', () => {
    const item = mapSummaryToItem(makeSummary({ confidence: 0.8 }));
    expect(item.priority).toBe('important');
  });

  it('uses Screen Intelligence as default sourceLabel when app_name is null', () => {
    const item = mapSummaryToItem(makeSummary({ app_name: null }));
    expect(item.sourceLabel).toBe('Screen Intelligence');
  });

  it('filters empty strings from description parts', () => {
    const item = mapSummaryToItem(makeSummary({ ui_state: '', key_text: 'some text' }));
    expect(item.description).toBe('some text');
  });

  it('truncates long actionable_notes in title', () => {
    const longNotes = 'A'.repeat(200);
    const item = mapSummaryToItem(makeSummary({ actionable_notes: longNotes }));
    expect(item.title.length).toBe(120);
  });
});

describe('confidenceToPriority', () => {
  it('returns critical for > 0.9', () => {
    expect(confidenceToPriority(0.91)).toBe('critical');
    expect(confidenceToPriority(1.0)).toBe('critical');
  });

  it('returns important for > 0.7 and <= 0.9', () => {
    expect(confidenceToPriority(0.71)).toBe('important');
    expect(confidenceToPriority(0.9)).toBe('important');
  });

  it('returns normal for <= 0.7', () => {
    expect(confidenceToPriority(0.7)).toBe('normal');
    expect(confidenceToPriority(0.0)).toBe('normal');
  });
});
