/**
 * Agent Toggle Component
 *
 * Toggle switch to enable/disable agent mode for a thread.
 * Shows agent status and allows configuration when enabled.
 */

import { memo, useCallback, useState } from 'react';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import {
  selectAgentModeForThread,
  selectAgentConfigForThread,
  selectActiveExecutionForThread,
  setAgentModeForThread,
  loadAgentTools
} from '../../store/agentSlice';

interface AgentToggleProps {
  threadId: string;
  className?: string;
  size?: 'sm' | 'md' | 'lg';
}

const AgentToggle = memo<AgentToggleProps>(({
  threadId,
  className = '',
  size = 'md'
}) => {
  const dispatch = useAppDispatch();
  const agentMode = useAppSelector(state => selectAgentModeForThread(state, threadId));
  const agentConfig = useAppSelector(state => selectAgentConfigForThread(state, threadId));
  const activeExecution = useAppSelector(state => selectActiveExecutionForThread(state, threadId));
  const [isLoading, setIsLoading] = useState(false);

  const handleToggle = useCallback(async () => {
    if (activeExecution) {
      // Can't disable while agent is running
      return;
    }

    setIsLoading(true);

    try {
      const newMode = !agentMode;

      // Enable agent mode
      if (newMode) {
        // Load tools when enabling agent mode
        await dispatch(loadAgentTools()).unwrap();
      }

      dispatch(setAgentModeForThread({
        threadId,
        enabled: newMode
      }));
    } catch (error) {
      console.error('Failed to toggle agent mode:', error);
    } finally {
      setIsLoading(false);
    }
  }, [dispatch, threadId, agentMode, activeExecution]);

  const getSizeClasses = () => {
    switch (size) {
      case 'sm':
        return {
          container: 'w-8 h-5',
          toggle: 'w-3 h-3',
          translate: 'translate-x-3'
        };
      case 'lg':
        return {
          container: 'w-12 h-7',
          toggle: 'w-5 h-5',
          translate: 'translate-x-5'
        };
      default: // md
        return {
          container: 'w-10 h-6',
          toggle: 'w-4 h-4',
          translate: 'translate-x-4'
        };
    }
  };

  const sizeClasses = getSizeClasses();
  const isDisabled = isLoading || Boolean(activeExecution);

  return (
    <div className={`flex items-center gap-3 ${className}`}>
      {/* Toggle Switch */}
      <button
        onClick={handleToggle}
        disabled={isDisabled}
        className={`
          relative inline-flex items-center ${sizeClasses.container} rounded-full
          transition-colors duration-200 ease-in-out
          ${agentMode
            ? 'bg-primary-500 hover:bg-primary-600'
            : 'bg-canvas-300 hover:bg-canvas-400'
          }
          ${isDisabled
            ? 'opacity-50 cursor-not-allowed'
            : 'cursor-pointer focus:outline-none focus:ring-2 focus:ring-primary-500 focus:ring-offset-2'
          }
        `}
        aria-label={`${agentMode ? 'Disable' : 'Enable'} agent mode`}
      >
        <span
          className={`
            ${sizeClasses.toggle} inline-block rounded-full bg-white shadow-sm
            transform transition-transform duration-200 ease-in-out
            ${agentMode ? sizeClasses.translate : 'translate-x-0.5'}
          `}
        />

        {/* Loading indicator */}
        {isLoading && (
          <div className="absolute inset-0 flex items-center justify-center">
            <div className="w-3 h-3 border-2 border-white border-t-transparent rounded-full animate-spin" />
          </div>
        )}
      </button>

      {/* Label and Status */}
      <div className="flex flex-col">
        <div className="flex items-center gap-2">
          <span className={`text-sm font-medium ${agentMode ? 'text-canvas-900' : 'text-canvas-600'}`}>
            Agent Mode
          </span>

          {agentMode && (
            <span className="px-2 py-0.5 text-xs font-medium bg-primary-100 text-primary-700 rounded-full">
              Active
            </span>
          )}
        </div>

        {/* Configuration hint */}
        {agentMode && !activeExecution && (
          <div className="text-xs text-canvas-500 mt-0.5">
            {agentConfig.maxIterations ? `Max ${agentConfig.maxIterations} iterations` : 'Default settings'}
            {agentConfig.allowedSkills && agentConfig.allowedSkills.length > 0 &&
              ` • ${agentConfig.allowedSkills.length} skills allowed`
            }
          </div>
        )}

        {/* Active execution status */}
        {activeExecution && (
          <div className="text-xs text-primary-600 mt-0.5 font-medium">
            Running iteration {activeExecution.currentIteration}/{activeExecution.maxIterations}
          </div>
        )}
      </div>
    </div>
  );
});

AgentToggle.displayName = 'AgentToggle';

export default AgentToggle;