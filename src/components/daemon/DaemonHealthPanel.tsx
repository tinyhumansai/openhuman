/**
 * Agent Health Panel
 *
 * Detailed health breakdown component showing agent status, component health,
 * and providing manual control buttons for agent lifecycle management.
 */
import {
  ArrowPathIcon,
  CheckCircleIcon,
  ClockIcon,
  PlayIcon,
  StopIcon,
  XCircleIcon,
  XMarkIcon,
} from '@heroicons/react/24/outline';
import { useState } from 'react';

import { formatRelativeTime, useDaemonHealth } from '../../hooks/useDaemonHealth';
import type { ComponentHealth, DaemonStatus } from '../../store/daemonSlice';
import { IS_DEV } from '../../utils/config';

interface Props {
  userId?: string;
  onClose?: () => void;
  className?: string;
}

const DaemonHealthPanel = ({ userId, onClose, className = '' }: Props) => {
  const daemonHealth = useDaemonHealth(userId);
  const [operationLoading, setOperationLoading] = useState<string | null>(null);

  // Handle agent operations with loading states
  const handleOperation = async (operation: () => Promise<unknown>, operationName: string) => {
    setOperationLoading(operationName);
    try {
      await operation();
    } catch (error) {
      console.error(`[AgentHealthPanel] ${operationName} failed:`, error);
    } finally {
      setOperationLoading(null);
    }
  };

  // Status styling
  const getStatusStyling = (status: DaemonStatus) => {
    switch (status) {
      case 'running':
        return {
          bg: 'bg-green-900/20 border-green-500/30',
          text: 'text-green-400',
          icon: CheckCircleIcon,
        };
      case 'starting':
        return {
          bg: 'bg-yellow-900/20 border-yellow-500/30',
          text: 'text-yellow-400',
          icon: ClockIcon,
        };
      case 'error':
        return { bg: 'bg-red-900/20 border-red-500/30', text: 'text-red-400', icon: XCircleIcon };
      case 'disconnected':
      default:
        return {
          bg: 'bg-gray-900/20 border-gray-500/30',
          text: 'text-gray-400',
          icon: XCircleIcon,
        };
    }
  };

  // Component status styling
  const getComponentStyling = (component: ComponentHealth) => {
    switch (component.status) {
      case 'ok':
        return { bg: 'bg-green-500', text: 'text-green-400', icon: CheckCircleIcon };
      case 'error':
        return { bg: 'bg-red-500', text: 'text-red-400', icon: XCircleIcon };
      case 'starting':
        return { bg: 'bg-yellow-500', text: 'text-yellow-400', icon: ClockIcon };
    }
  };

  const statusStyling = getStatusStyling(daemonHealth.status);
  const StatusIcon = statusStyling.icon;

  return (
    <div className={`bg-stone-900 rounded-lg border border-stone-700 p-6 space-y-6 ${className}`}>
      {/* Header */}
      <div className="flex items-center justify-between">
        <h3 className="text-lg font-semibold text-white">Agent Status</h3>
        {onClose && (
          <button onClick={onClose} className="text-gray-400 hover:text-white transition-colors">
            <XMarkIcon className="w-5 h-5" />
          </button>
        )}
      </div>

      {/* Overall Status */}
      <div className={`p-4 rounded-lg border ${statusStyling.bg}`}>
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <StatusIcon className={`w-6 h-6 ${statusStyling.text}`} />
            <div>
              <div className={`font-medium ${statusStyling.text}`}>
                Status: {daemonHealth.status.charAt(0).toUpperCase() + daemonHealth.status.slice(1)}
              </div>
              {daemonHealth.healthSnapshot && (
                <div className="text-sm text-gray-400">
                  PID: {daemonHealth.healthSnapshot.pid} • Uptime: {daemonHealth.uptimeText}
                </div>
              )}
              {daemonHealth.lastUpdate && (
                <div className="text-xs text-gray-500">
                  Last update: {formatRelativeTime(daemonHealth.lastUpdate)}
                </div>
              )}
            </div>
          </div>

          {/* Recovery indicator */}
          {daemonHealth.isRecovering && (
            <div className="flex items-center gap-2 text-yellow-400">
              <ArrowPathIcon className="w-4 h-4 animate-spin" />
              <span className="text-sm">Recovering...</span>
            </div>
          )}
        </div>
      </div>

      {/* Component Health */}
      {daemonHealth.componentCount > 0 && (
        <div className="space-y-3">
          <h4 className="text-sm font-medium text-gray-300">
            Components ({daemonHealth.healthyComponentCount}/{daemonHealth.componentCount} healthy)
          </h4>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            {Object.entries(daemonHealth.components).map(([name, component]) => {
              const componentStyling = getComponentStyling(component);
              const ComponentIcon = componentStyling.icon;

              return (
                <div
                  key={name}
                  className="flex items-center gap-3 p-3 rounded-lg bg-stone-800/40 border border-stone-700/60">
                  <div className={`w-2 h-2 rounded-full ${componentStyling.bg}`} />
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2">
                      <ComponentIcon className={`w-4 h-4 ${componentStyling.text}`} />
                      <span className="capitalize text-gray-300 font-medium">{name}</span>
                    </div>
                    <div className="text-xs text-gray-500">
                      Updated: {formatRelativeTime(component.updated_at)}
                    </div>
                    {component.restart_count > 0 && (
                      <div className="text-xs text-yellow-400">
                        Restarts: {component.restart_count}
                      </div>
                    )}
                    {component.last_error && component.status === 'error' && (
                      <div className="text-xs text-red-400 truncate" title={component.last_error}>
                        Error: {component.last_error}
                      </div>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}

      {/* Auto-start Toggle */}
      <div className="flex items-center justify-between p-3 rounded-lg bg-stone-800/40 border border-stone-700/60">
        <div>
          <div className="text-sm font-medium text-gray-300">Auto-start Agent</div>
          <div className="text-xs text-gray-500">Automatically start agent on app launch</div>
        </div>
        <label className="relative inline-flex items-center cursor-pointer">
          <input
            type="checkbox"
            className="sr-only peer"
            checked={daemonHealth.isAutoStartEnabled}
            onChange={e => daemonHealth.setAutoStart(e.target.checked)}
          />
          <div className="w-11 h-6 bg-gray-200 peer-focus:outline-none peer-focus:ring-4 peer-focus:ring-blue-300 dark:peer-focus:ring-blue-800 rounded-full peer dark:bg-gray-700 peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-5 after:w-5 after:transition-all dark:border-gray-600 peer-checked:bg-blue-600"></div>
        </label>
      </div>

      {/* Control Actions */}
      <div className="flex flex-wrap gap-2">
        <button
          onClick={() => handleOperation(daemonHealth.startDaemon, 'start')}
          disabled={daemonHealth.status === 'running' || operationLoading !== null}
          className="inline-flex items-center gap-2 px-3 py-2 text-sm font-medium text-white bg-green-600 hover:bg-green-700 disabled:bg-gray-600 disabled:cursor-not-allowed rounded-lg transition-colors">
          {operationLoading === 'start' ? (
            <ArrowPathIcon className="w-4 h-4 animate-spin" />
          ) : (
            <PlayIcon className="w-4 h-4" />
          )}
          Start
        </button>

        <button
          onClick={() => handleOperation(daemonHealth.stopDaemon, 'stop')}
          disabled={daemonHealth.status === 'disconnected' || operationLoading !== null}
          className="inline-flex items-center gap-2 px-3 py-2 text-sm font-medium text-white bg-red-600 hover:bg-red-700 disabled:bg-gray-600 disabled:cursor-not-allowed rounded-lg transition-colors">
          {operationLoading === 'stop' ? (
            <ArrowPathIcon className="w-4 h-4 animate-spin" />
          ) : (
            <StopIcon className="w-4 h-4" />
          )}
          Stop
        </button>

        <button
          onClick={() => handleOperation(daemonHealth.restartDaemon, 'restart')}
          disabled={operationLoading !== null}
          className="inline-flex items-center gap-2 px-3 py-2 text-sm font-medium text-white bg-blue-600 hover:bg-blue-700 disabled:bg-gray-600 disabled:cursor-not-allowed rounded-lg transition-colors">
          {operationLoading === 'restart' ? (
            <ArrowPathIcon className="w-4 h-4 animate-spin" />
          ) : (
            <ArrowPathIcon className="w-4 h-4" />
          )}
          Restart
        </button>

        <button
          onClick={() => handleOperation(daemonHealth.checkDaemonStatus, 'check')}
          disabled={operationLoading !== null}
          className="inline-flex items-center gap-2 px-3 py-2 text-sm font-medium text-gray-300 bg-gray-700 hover:bg-gray-600 disabled:bg-gray-800 disabled:cursor-not-allowed rounded-lg transition-colors">
          {operationLoading === 'check' ? (
            <ArrowPathIcon className="w-4 h-4 animate-spin" />
          ) : (
            <ArrowPathIcon className="w-4 h-4" />
          )}
          Check Status
        </button>
      </div>

      {/* Connection Info */}
      {daemonHealth.connectionAttempts > 0 && (
        <div className="p-3 rounded-lg bg-yellow-900/20 border border-yellow-500/30">
          <div className="text-sm text-yellow-400">
            Connection attempts: {daemonHealth.connectionAttempts}
          </div>
        </div>
      )}

      {/* Debug Info (development only) */}
      {IS_DEV && daemonHealth.healthSnapshot && (
        <details className="text-xs">
          <summary className="cursor-pointer text-gray-400 hover:text-white">Debug Info</summary>
          <pre className="mt-2 p-3 bg-stone-800 rounded text-gray-300 overflow-x-auto">
            {JSON.stringify(daemonHealth.healthSnapshot, null, 2)}
          </pre>
        </details>
      )}
    </div>
  );
};

export default DaemonHealthPanel;
