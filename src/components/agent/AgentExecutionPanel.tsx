/**
 * Agent Execution Panel Component
 *
 * Detailed view of agent execution progress, tool executions, and results.
 * Expandable panel that shows real-time execution details.
 */

import { memo, useMemo } from 'react';
import { useAppSelector } from '../../store/hooks';
import {
  selectActiveExecutionForThread,
  selectExecutionHistoryForThread,
  selectAgentModeForThread
} from '../../store/agentSlice';
import type { AgentToolExecution } from '../../types/agent';

interface AgentExecutionPanelProps {
  threadId: string;
  className?: string;
  maxHeight?: string;
}

const formatDuration = (ms: number): string => {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.floor(ms / 60000)}m ${Math.floor((ms % 60000) / 1000)}s`;
};

const getStatusIcon = (status: string) => {
  switch (status) {
    case 'pending':
      return (
        <svg className="w-4 h-4 text-canvas-400" fill="currentColor" viewBox="0 0 20 20">
          <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z" clipRule="evenodd" />
        </svg>
      );
    case 'running':
      return (
        <div className="w-4 h-4 border-2 border-primary-500 border-t-transparent rounded-full animate-spin" />
      );
    case 'success':
      return (
        <svg className="w-4 h-4 text-sage-500" fill="currentColor" viewBox="0 0 20 20">
          <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm3.707-9.293a1 1 0 00-1.414-1.414L9 10.586 7.707 9.293a1 1 0 00-1.414 1.414l2 2a1 1 0 001.414 0l4-4z" clipRule="evenodd" />
        </svg>
      );
    case 'error':
      return (
        <svg className="w-4 h-4 text-coral-500" fill="currentColor" viewBox="0 0 20 20">
          <path fillRule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zM8.707 7.293a1 1 0 00-1.414 1.414L8.586 10l-1.293 1.293a1 1 0 101.414 1.414L10 11.414l1.293 1.293a1 1 0 001.414-1.414L11.414 10l1.293-1.293a1 1 0 00-1.414-1.414L10 8.586 8.707 7.293z" clipRule="evenodd" />
        </svg>
      );
    default:
      return (
        <div className="w-4 h-4 bg-canvas-300 rounded-full" />
      );
  }
};

const ToolExecutionItem = memo<{ toolExecution: AgentToolExecution }>(({ toolExecution }) => {
  const duration = toolExecution.executionTimeMs || (toolExecution.endTime ? toolExecution.endTime - toolExecution.startTime : null);

  return (
    <div className="flex items-start gap-3 p-3 bg-canvas-50 rounded-lg border border-canvas-200">
      <div className="flex-shrink-0 mt-0.5">
        {getStatusIcon(toolExecution.status)}
      </div>

      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2 mb-1">
          <span className="font-medium text-canvas-900">{toolExecution.toolName}</span>
          <span className="text-xs text-canvas-500 bg-canvas-200 px-2 py-0.5 rounded">
            {toolExecution.skillId}
          </span>
          {duration && (
            <span className="text-xs text-canvas-500">
              {formatDuration(duration)}
            </span>
          )}
        </div>

        {/* Arguments */}
        {toolExecution.arguments && (
          <div className="mb-2">
            <span className="text-xs font-medium text-canvas-600 block mb-1">Arguments:</span>
            <pre className="text-xs text-canvas-700 bg-canvas-100 p-2 rounded border overflow-x-auto">
              {JSON.stringify(JSON.parse(toolExecution.arguments), null, 2)}
            </pre>
          </div>
        )}

        {/* Result */}
        {toolExecution.result && (
          <div className="mb-2">
            <span className="text-xs font-medium text-canvas-600 block mb-1">Result:</span>
            <div className="text-sm text-canvas-800 bg-white p-2 rounded border">
              {toolExecution.result}
            </div>
          </div>
        )}

        {/* Error */}
        {toolExecution.errorMessage && (
          <div className="mb-2">
            <span className="text-xs font-medium text-coral-600 block mb-1">Error:</span>
            <div className="text-sm text-coral-700 bg-coral-50 p-2 rounded border border-coral-200">
              {toolExecution.errorMessage}
            </div>
          </div>
        )}
      </div>
    </div>
  );
});

ToolExecutionItem.displayName = 'ToolExecutionItem';

const AgentExecutionPanel = memo<AgentExecutionPanelProps>(({
  threadId,
  className = '',
  maxHeight = '400px'
}) => {
  const agentMode = useAppSelector(state => selectAgentModeForThread(state, threadId));
  const activeExecution = useAppSelector(state => selectActiveExecutionForThread(state, threadId));
  const executionHistory = useAppSelector(state => selectExecutionHistoryForThread(state, threadId));

  const sortedToolExecutions = useMemo(() => {
    if (!activeExecution) return [];
    return [...activeExecution.toolExecutions].sort((a, b) => a.startTime - b.startTime);
  }, [activeExecution]);

  const recentHistory = useMemo(() => {
    return executionHistory.slice(0, 3); // Show last 3 completed executions
  }, [executionHistory]);

  if (!agentMode) {
    return null;
  }

  return (
    <div className={`border border-canvas-200 rounded-lg bg-white ${className}`}>
      <div className="p-4 border-b border-canvas-200">
        <h3 className="text-sm font-semibold text-canvas-900">Agent Execution Details</h3>
      </div>

      <div className={`overflow-y-auto`} style={{ maxHeight }}>
        {/* Active Execution */}
        {activeExecution && (
          <div className="p-4 border-b border-canvas-200">
            <div className="flex items-center justify-between mb-3">
              <h4 className="text-sm font-medium text-canvas-800">Current Execution</h4>
              <span className="text-xs text-canvas-500">
                Running for {formatDuration(Date.now() - activeExecution.startTime)}
              </span>
            </div>

            <div className="mb-3">
              <div className="text-xs text-canvas-600 mb-1">Progress:</div>
              <div className="flex items-center gap-2">
                <div className="flex-1 bg-canvas-200 rounded-full h-2">
                  <div
                    className="bg-primary-500 h-2 rounded-full transition-all duration-300"
                    style={{
                      width: `${(activeExecution.currentIteration / activeExecution.maxIterations) * 100}%`
                    }}
                  />
                </div>
                <span className="text-xs text-canvas-600 font-medium">
                  {activeExecution.currentIteration}/{activeExecution.maxIterations}
                </span>
              </div>
            </div>

            {/* Tool Executions */}
            {sortedToolExecutions.length > 0 && (
              <div className="space-y-2">
                <div className="text-xs font-medium text-canvas-600 mb-2">
                  Tool Executions ({sortedToolExecutions.length}):
                </div>
                {sortedToolExecutions.map(toolExecution => (
                  <ToolExecutionItem key={toolExecution.id} toolExecution={toolExecution} />
                ))}
              </div>
            )}
          </div>
        )}

        {/* Execution History */}
        {recentHistory.length > 0 && (
          <div className="p-4">
            <h4 className="text-sm font-medium text-canvas-800 mb-3">Recent Executions</h4>
            <div className="space-y-2">
              {recentHistory.map(entry => (
                <div
                  key={entry.executionId}
                  className="flex items-center justify-between p-2 bg-canvas-50 rounded border"
                >
                  <div className="flex items-center gap-2">
                    <div className={`w-2 h-2 rounded-full ${
                      entry.result.status === 'completed' ? 'bg-sage-500' :
                      entry.result.status === 'error' ? 'bg-coral-500' : 'bg-amber-500'
                    }`} />
                    <span className="text-xs text-canvas-700 font-medium">
                      {entry.result.status}
                    </span>
                    <span className="text-xs text-canvas-500">
                      {entry.result.toolExecutions.length} tools
                    </span>
                  </div>
                  <div className="text-xs text-canvas-500">
                    {formatDuration(entry.duration)}
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Empty State */}
        {!activeExecution && recentHistory.length === 0 && (
          <div className="p-6 text-center text-canvas-500">
            <svg className="w-8 h-8 mx-auto mb-2 text-canvas-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M13 10V3L4 14h7v7l9-11h-7z" />
            </svg>
            <p className="text-sm">No agent executions yet</p>
            <p className="text-xs mt-1">Send a message to start an agent task</p>
          </div>
        )}
      </div>
    </div>
  );
});

AgentExecutionPanel.displayName = 'AgentExecutionPanel';

export default AgentExecutionPanel;