/**
 * Agent Status Indicator Component
 *
 * Shows the current status of agent execution within thread UI.
 * Displays real-time agent activity, tool executions, and completion status.
 */

import { memo } from 'react';
import { useAppSelector } from '../../store/hooks';
import { selectActiveExecutionForThread, selectAgentModeForThread } from '../../store/agentSlice';

interface AgentStatusIndicatorProps {
  threadId: string;
  className?: string;
}

const AgentStatusIndicator = memo<AgentStatusIndicatorProps>(({ threadId, className = '' }) => {
  const agentMode = useAppSelector(state => selectAgentModeForThread(state, threadId));
  const activeExecution = useAppSelector(state => selectActiveExecutionForThread(state, threadId));

  // Don't render if agent mode is disabled
  if (!agentMode) {
    return null;
  }

  // No active execution
  if (!activeExecution) {
    return (
      <div className={`flex items-center gap-2 text-canvas-600 ${className}`}>
        <div className="w-2 h-2 bg-sage-500 rounded-full"></div>
        <span className="text-xs font-medium">Agent Ready</span>
      </div>
    );
  }

  const getStatusColor = () => {
    switch (activeExecution.status) {
      case 'initializing':
        return 'bg-amber-500';
      case 'running':
        return 'bg-primary-500 animate-pulse';
      case 'completing':
        return 'bg-sage-500';
      default:
        return 'bg-canvas-400';
    }
  };

  const getStatusText = () => {
    switch (activeExecution.status) {
      case 'initializing':
        return 'Starting...';
      case 'running':
        return `Iteration ${activeExecution.currentIteration}/${activeExecution.maxIterations}`;
      case 'completing':
        return 'Finishing...';
      default:
        return 'Agent Active';
    }
  };

  const toolCount = activeExecution.toolExecutions.length;
  const runningTools = activeExecution.toolExecutions.filter(t => t.status === 'running').length;

  return (
    <div className={`flex items-center gap-3 ${className}`}>
      {/* Status indicator */}
      <div className="flex items-center gap-2">
        <div className={`w-2 h-2 rounded-full ${getStatusColor()}`}></div>
        <span className="text-xs font-medium text-canvas-700">{getStatusText()}</span>
      </div>

      {/* Tool execution info */}
      {toolCount > 0 && (
        <div className="flex items-center gap-1.5 text-xs text-canvas-600">
          <svg className="w-3 h-3" fill="currentColor" viewBox="0 0 20 20">
            <path d="M13.586 3.586a2 2 0 112.828 2.828l-.793.793-2.828-2.828.793-.793zM11.379 5.793L3 14.172V17h2.828l8.38-8.379-2.83-2.828z" />
          </svg>
          <span>{toolCount} tools</span>
          {runningTools > 0 && (
            <span className="text-primary-600">• {runningTools} running</span>
          )}
        </div>
      )}

      {/* Execution time */}
      <div className="text-xs text-canvas-500">
        {Math.floor((Date.now() - activeExecution.startTime) / 1000)}s
      </div>
    </div>
  );
});

AgentStatusIndicator.displayName = 'AgentStatusIndicator';

export default AgentStatusIndicator;