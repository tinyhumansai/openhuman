import type { ActionableItemStatus } from '../types/intelligence';
import { apiClient } from './apiClient';

/**
 * Backend API response types for Intelligence system
 */
export interface BackendActionableItem {
  id: string;
  title: string;
  description?: string;
  source: string;
  priority: string;
  status: string;
  createdAt: string;
  updatedAt: string;
  expiresAt?: string;
  snoozeUntil?: string;
  actionable: boolean;
  requiresConfirmation?: boolean;
  hasComplexAction?: boolean;
  dismissedAt?: string;
  completedAt?: string;
  reminderCount?: number;
  // Backend-specific fields
  threadId?: string;
  executionStatus?: 'idle' | 'running' | 'completed' | 'failed';
  currentSessionId?: string;
}

export interface BackendChatMessage {
  id: string;
  content: string;
  role: 'user' | 'assistant';
  timestamp: string;
  threadId: string;
}

export interface BackendThreadResponse {
  threadId: string;
  messages: BackendChatMessage[];
}

export interface BackendExecutionResponse {
  executionId: string;
  sessionId: string;
  status: 'started' | 'running' | 'completed' | 'failed';
}

export interface ConnectedTool {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
  skillId: string;
  enabled: boolean;
}

/**
 * Intelligence API Service - handles all REST API calls for the Intelligence system
 */
export class IntelligenceApiService {
  /**
   * Get all actionable items from the backend
   */
  async getActionableItems(): Promise<BackendActionableItem[]> {
    try {
      const response = await apiClient.get<{ items: BackendActionableItem[] }>(
        '/telegram/actionable-items'
      );
      return response.items || [];
    } catch (error) {
      console.error('Failed to fetch actionable items:', error);
      throw error;
    }
  }

  /**
   * Update the status of an actionable item
   */
  async updateItemStatus(itemId: string, status: ActionableItemStatus): Promise<void> {
    try {
      await apiClient.patch(`/actionable-items/${itemId}`, { status });
    } catch (error) {
      console.error('Failed to update item status:', error);
      throw error;
    }
  }

  /**
   * Snooze an actionable item until a specific time
   */
  async snoozeItem(itemId: string, snoozeUntil: Date): Promise<void> {
    try {
      await apiClient.patch(`/actionable-items/${itemId}`, {
        status: 'snoozed',
        snoozeUntil: snoozeUntil.toISOString(),
      });
    } catch (error) {
      console.error('Failed to snooze item:', error);
      throw error;
    }
  }

  /**
   * Get or create a conversation thread for an actionable item
   */
  async getOrCreateThread(itemId: string): Promise<BackendThreadResponse> {
    try {
      const response = await apiClient.get<BackendThreadResponse>(`/${itemId}/thread`);
      return response;
    } catch (error) {
      console.error('Failed to get or create thread:', error);
      throw error;
    }
  }

  /**
   * Get chat history for a specific thread
   */
  async getChatHistory(threadId: string): Promise<BackendChatMessage[]> {
    try {
      const response = await apiClient.get<{ messages: BackendChatMessage[] }>(
        `/threads/${threadId}/messages`
      );
      return response.messages || [];
    } catch (error) {
      console.error('Failed to get chat history:', error);
      throw error;
    }
  }

  /**
   * Start task execution for an actionable item with connected tools
   */
  async executeTask(
    itemId: string,
    connectedTools: ConnectedTool[]
  ): Promise<BackendExecutionResponse> {
    try {
      const response = await apiClient.post<BackendExecutionResponse>(`/${itemId}/execute`, {
        connectedTools,
      });
      return response;
    } catch (error) {
      console.error('Failed to execute task:', error);
      throw error;
    }
  }

  /**
   * Get execution status for a specific execution ID
   */
  async getExecutionStatus(
    executionId: string
  ): Promise<{
    status: 'running' | 'completed' | 'failed';
    progress?: Array<Record<string, unknown>>;
    result?: unknown;
  }> {
    try {
      const response = await apiClient.get(`/executions/${executionId}/status`);
      return response as {
        status: 'running' | 'completed' | 'failed';
        progress?: Array<Record<string, unknown>>;
        result?: unknown;
      };
    } catch (error) {
      console.error('Failed to get execution status:', error);
      throw error;
    }
  }

  /**
   * Cancel a running execution
   */
  async cancelExecution(executionId: string): Promise<void> {
    try {
      await apiClient.post(`/executions/${executionId}/cancel`);
    } catch (error) {
      console.error('Failed to cancel execution:', error);
      throw error;
    }
  }
}

// Export singleton instance
export const intelligenceApi = new IntelligenceApiService();
