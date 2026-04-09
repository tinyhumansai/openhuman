import { useMemo } from 'react';

import { useScreenIntelligenceState } from '../features/screen-intelligence/useScreenIntelligenceState';
import type { ActionableItem, ActionableItemPriority } from '../types/intelligence';

function confidenceToPriority(confidence: number): ActionableItemPriority {
  if (confidence > 0.9) return 'critical';
  if (confidence > 0.7) return 'important';
  return 'normal';
}

export function useScreenIntelligenceItems() {
  const { recentVisionSummaries, isLoadingVision, refreshVision } = useScreenIntelligenceState({
    loadVision: true,
    visionLimit: 20,
    pollMs: 2000,
  });

  const items: ActionableItem[] = useMemo(() => {
    return recentVisionSummaries.map(summary => ({
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
    }));
  }, [recentVisionSummaries]);

  return { items, loading: isLoadingVision, refresh: () => refreshVision(20) };
}
