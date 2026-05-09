// Intelligence System Types
// Actionable items and AI insights for the Intelligence page
import type React from 'react';

export type ActionableItemSource =
  | 'email'
  | 'calendar'
  | 'telegram'
  | 'ai_insight'
  | 'system'
  | 'trading'
  | 'security';

export type ActionableItemPriority = 'critical' | 'important' | 'normal';

export type ActionableItemStatus = 'active' | 'dismissed' | 'completed' | 'snoozed';

export interface ActionableItem {
  id: string;
  title: string;
  description?: string;
  source: ActionableItemSource;
  priority: ActionableItemPriority;
  status: ActionableItemStatus;
  createdAt: Date;
  updatedAt: Date;
  expiresAt?: Date;
  snoozeUntil?: Date;

  // Action metadata
  actionable: boolean;
  requiresConfirmation?: boolean;
  hasComplexAction?: boolean;

  // Visual presentation
  icon?: React.ReactElement;
  sourceLabel?: string;

  // Interaction tracking
  dismissedAt?: Date;
  completedAt?: Date;
  reminderCount?: number;

  // Backend integration fields
  threadId?: string;
  executionStatus?: 'idle' | 'running' | 'completed' | 'failed';
  currentSessionId?: string;
}

export interface ActionableItemAction {
  type: 'complete' | 'dismiss' | 'snooze';
  timestamp: Date;
  itemId: string;
  metadata?: Record<string, unknown>;
}

export interface TimeGroup {
  label: string;
  items: ActionableItem[];
  count: number;
}

export interface IntelligencePageState {
  items: ActionableItem[];
  loading: boolean;
  error: string | null;
  lastUpdate: Date | null;

  // UI state
  showCompleted: boolean;
  filter: ActionableItemSource | 'all';
}

// Snooze time options
export type SnoozeOption = {
  label: string;
  duration: number; // milliseconds
  customTime?: Date;
};

// Toast notification types
export interface ToastNotification {
  id: string;
  type: 'success' | 'error' | 'warning' | 'info';
  title: string;
  message?: string;
  duration?: number;
  action?: { label: string; handler: () => void };
}

// Confirmation modal data
export interface ConfirmationModal {
  isOpen: boolean;
  title: string;
  message: string;
  confirmText?: string;
  cancelText?: string;
  onConfirm: (dontShowAgain?: boolean) => void;
  onCancel: () => void;
  destructive?: boolean;
  showDontShowAgain?: boolean;
  preferenceKey?: string;
}

// Chat message type
export interface ChatMessage {
  id: string;
  content: string;
  sender: 'user' | 'ai';
  timestamp: Date;
}
