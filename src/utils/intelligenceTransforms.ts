import type { MCPTool } from '../lib/mcp';
import type {
  BackendActionableItem,
  BackendChatMessage,
  ConnectedTool,
} from '../services/intelligenceApi';
import type {
  ActionableItem,
  ActionableItemPriority,
  ActionableItemSource,
  ActionableItemStatus,
  ChatMessage,
} from '../types/intelligence';

/**
 * Transform backend actionable item to frontend format
 */
export function transformBackendItemToFrontend(backendItem: BackendActionableItem): ActionableItem {
  return {
    id: backendItem.id,
    title: backendItem.title,
    description: backendItem.description,
    source: backendItem.source as ActionableItemSource,
    priority: backendItem.priority as ActionableItemPriority,
    status: backendItem.status as ActionableItemStatus,
    createdAt: new Date(backendItem.createdAt),
    updatedAt: new Date(backendItem.updatedAt),
    expiresAt: backendItem.expiresAt ? new Date(backendItem.expiresAt) : undefined,
    snoozeUntil: backendItem.snoozeUntil ? new Date(backendItem.snoozeUntil) : undefined,
    actionable: backendItem.actionable,
    requiresConfirmation: backendItem.requiresConfirmation,
    hasComplexAction: backendItem.hasComplexAction,
    dismissedAt: backendItem.dismissedAt ? new Date(backendItem.dismissedAt) : undefined,
    completedAt: backendItem.completedAt ? new Date(backendItem.completedAt) : undefined,
    reminderCount: backendItem.reminderCount,
    // Backend integration fields
    threadId: backendItem.threadId,
    executionStatus: backendItem.executionStatus,
    currentSessionId: backendItem.currentSessionId,
  };
}

/**
 * Transform multiple backend items to frontend format
 */
export function transformBackendItemsToFrontend(
  backendItems: BackendActionableItem[]
): ActionableItem[] {
  return backendItems.map(transformBackendItemToFrontend);
}

/**
 * Transform backend chat message to frontend format
 */
export function transformBackendMessageToFrontend(backendMessage: BackendChatMessage): ChatMessage {
  return {
    id: backendMessage.id,
    content: backendMessage.content,
    sender: backendMessage.role === 'user' ? 'user' : 'ai',
    timestamp: new Date(backendMessage.timestamp),
  };
}

/**
 * Transform multiple backend messages to frontend format
 */
export function transformBackendMessagesToFrontend(
  backendMessages: BackendChatMessage[]
): ChatMessage[] {
  return backendMessages.map(transformBackendMessageToFrontend);
}

/**
 * Transform frontend chat message to backend format
 */
export function transformFrontendMessageToBackend(
  message: ChatMessage,
  threadId: string
): Omit<BackendChatMessage, 'id'> {
  return {
    content: message.content,
    role: message.sender === 'user' ? 'user' : 'assistant',
    timestamp: message.timestamp.toISOString(),
    threadId,
  };
}

/**
 * Transform MCP tools to connected tools format for backend
 */
export function transformMCPToConnectedTools(mcpTools: MCPTool[]): ConnectedTool[] {
  return mcpTools.map(tool => {
    const [skillId, toolName] = tool.name.split('__');

    return {
      name: toolName || tool.name,
      description: tool.description,
      parameters: (tool.inputSchema || {}) as unknown as Record<string, unknown>,
      skillId: skillId || 'unknown',
      enabled: true,
    };
  });
}

/**
 * Transform connected tools back to MCP format
 */
export function transformConnectedToolsToMCP(connectedTools: ConnectedTool[]): MCPTool[] {
  return connectedTools.map(tool => ({
    name: `${tool.skillId}__${tool.name}`,
    description: tool.description,
    inputSchema: { type: 'object', properties: tool.parameters || {} },
  }));
}

/**
 * Validate actionable item data from backend
 */
export function validateBackendItem(item: unknown): item is BackendActionableItem {
  if (typeof item !== 'object' || item === null) return false;
  const o = item as Record<string, unknown>;
  return (
    typeof o.id === 'string' &&
    typeof o.title === 'string' &&
    typeof o.source === 'string' &&
    typeof o.priority === 'string' &&
    typeof o.status === 'string' &&
    typeof o.createdAt === 'string' &&
    typeof o.updatedAt === 'string' &&
    typeof o.actionable === 'boolean'
  );
}

/**
 * Create a new chat message in frontend format
 */
export function createChatMessage(
  content: string,
  sender: 'user' | 'ai',
  id?: string
): ChatMessage {
  return {
    id: id || `${sender}-${Date.now()}-${Math.random().toString(36).substr(2, 9)}`,
    content,
    sender,
    timestamp: new Date(),
  };
}

/**
 * Map execution status to user-friendly labels
 */
export function getExecutionStatusLabel(status?: string): string {
  switch (status) {
    case 'idle':
      return 'Ready';
    case 'running':
      return 'In Progress';
    case 'completed':
      return 'Completed';
    case 'failed':
      return 'Failed';
    default:
      return 'Unknown';
  }
}

/**
 * Get priority display information
 */
export function getPriorityInfo(priority: ActionableItemPriority): {
  label: string;
  className: string;
  color: string;
} {
  switch (priority) {
    case 'critical':
      return { label: 'Critical', className: 'text-coral-400 bg-coral-500/10', color: 'coral' };
    case 'important':
      return { label: 'Important', className: 'text-amber-400 bg-amber-500/10', color: 'amber' };
    case 'normal':
      return { label: 'Normal', className: 'text-sage-400 bg-sage-500/10', color: 'sage' };
  }
}

/**
 * Get source display information
 */
export function getSourceInfo(source: ActionableItemSource): {
  label: string;
  icon: string;
  className: string;
} {
  switch (source) {
    case 'email':
      return { label: 'Email', icon: '📧', className: 'text-blue-400 bg-blue-500/10' };
    case 'calendar':
      return { label: 'Calendar', icon: '📅', className: 'text-green-400 bg-green-500/10' };
    case 'telegram':
      return { label: 'Telegram', icon: '💬', className: 'text-blue-400 bg-blue-500/10' };
    case 'ai_insight':
      return { label: 'AI Insight', icon: '🤖', className: 'text-purple-400 bg-purple-500/10' };
    case 'system':
      return { label: 'System', icon: '⚙️', className: 'text-stone-400 bg-stone-500/10' };
    case 'trading':
      return { label: 'Trading', icon: '📈', className: 'text-yellow-400 bg-yellow-500/10' };
    case 'security':
      return { label: 'Security', icon: '🔒', className: 'text-red-400 bg-red-500/10' };
  }
}
