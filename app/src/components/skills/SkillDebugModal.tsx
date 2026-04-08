/**
 * Debug modal for inspecting a skill's runtime state and calling its tools.
 * Shows: snapshot metadata, published state, tool definitions, and a tool executor.
 */

import { useState, useEffect, useRef, useCallback } from 'react';
import { createPortal } from 'react-dom';

import { useSkillSnapshot } from '../../lib/skills/hooks';
import { callCoreRpc } from '../../services/coreRpcClient';
import type { SkillSnapshotRpc } from '../../lib/skills/skillsApi';

interface SkillDebugModalProps {
  skillId: string;
  skillName: string;
  onClose: () => void;
}

interface ToolCallResult {
  toolName: string;
  result: unknown;
  isError: boolean;
  durationMs: number;
}

export default function SkillDebugModal({ skillId, skillName, onClose }: SkillDebugModalProps) {
  const modalRef = useRef<HTMLDivElement>(null);
  const snap = useSkillSnapshot(skillId);
  const [activeTab, setActiveTab] = useState<'state' | 'tools'>('state');
  const [expandedTool, setExpandedTool] = useState<string | null>(null);
  const [toolArgs, setToolArgs] = useState<Record<string, string>>({});
  const [toolLoading, setToolLoading] = useState<string | null>(null);
  const [toolResult, setToolResult] = useState<ToolCallResult | null>(null);

  // Escape key
  useEffect(() => {
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleEscape);
    return () => document.removeEventListener('keydown', handleEscape);
  }, [onClose]);

  // Focus trap
  useEffect(() => {
    modalRef.current?.focus();
  }, []);

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onClose();
  };

  const handleCallTool = useCallback(
    async (toolName: string) => {
      setToolLoading(toolName);
      setToolResult(null);
      const start = Date.now();
      try {
        // Parse args JSON
        let args: Record<string, unknown> = {};
        const rawArgs = toolArgs[toolName];
        if (rawArgs?.trim()) {
          args = JSON.parse(rawArgs);
        }

        const result = await callCoreRpc<{
          content: Array<{ type: string; text?: string; data?: unknown }>;
          is_error: boolean;
        }>({
          method: 'openhuman.skills_call_tool',
          params: { skill_id: skillId, tool_name: toolName, arguments: args },
        });

        setToolResult({
          toolName,
          result,
          isError: result.is_error ?? false,
          durationMs: Date.now() - start,
        });
      } catch (err) {
        setToolResult({
          toolName,
          result: err instanceof Error ? err.message : String(err),
          isError: true,
          durationMs: Date.now() - start,
        });
      } finally {
        setToolLoading(null);
      }
    },
    [skillId, toolArgs]
  );

  const formatJson = (value: unknown) => {
    try {
      return JSON.stringify(value, null, 2);
    } catch {
      return String(value);
    }
  };

  const tabs = [
    { id: 'state' as const, label: 'State' },
    { id: 'tools' as const, label: `Tools (${snap?.tools.length ?? 0})` },
  ];

  const content = (
    <div
      className="fixed inset-0 z-[9999] bg-black/30 backdrop-blur-sm flex items-center justify-center p-4"
      onClick={handleBackdropClick}
      role="dialog"
      aria-modal="true">
      <div
        ref={modalRef}
        className="bg-white border border-stone-200 rounded-3xl shadow-large w-full max-w-[600px] max-h-[80vh] overflow-hidden animate-fade-up focus:outline-none"
        style={{ animationDuration: '200ms' }}
        tabIndex={-1}
        onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className="p-4 border-b border-stone-200 flex items-center justify-between">
          <div>
            <h2 className="text-base font-semibold text-stone-900">
              Debug: {skillName}
            </h2>
            <div className="flex items-center gap-2 mt-1">
              <StatusBadge label={snap?.status ?? 'unknown'} />
              <StatusBadge label={snap?.connection_status ?? 'unknown'} variant="blue" />
              {snap?.setup_complete && <StatusBadge label="setup complete" variant="green" />}
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="p-1 text-stone-400 hover:text-stone-900 transition-colors rounded-lg hover:bg-stone-100">
            <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Tabs */}
        <div className="flex border-b border-stone-200">
          {tabs.map(tab => (
            <button
              key={tab.id}
              type="button"
              data-testid={tab.id === 'tools' ? 'skill-debug-tab-tools' : 'skill-debug-tab-state'}
              onClick={() => setActiveTab(tab.id)}
              className={`flex-1 py-2 text-xs font-medium transition-colors ${
                activeTab === tab.id
                  ? 'text-primary-500 border-b-2 border-primary-500'
                  : 'text-stone-600 hover:text-stone-700'
              }`}>
              {tab.label}
            </button>
          ))}
        </div>

        {/* Content */}
        <div className="overflow-y-auto" style={{ maxHeight: 'calc(80vh - 140px)' }}>
          {activeTab === 'state' && <StateTab snap={snap} formatJson={formatJson} />}
          {activeTab === 'tools' && (
            <ToolsTab
              snap={snap}
              expandedTool={expandedTool}
              setExpandedTool={setExpandedTool}
              toolArgs={toolArgs}
              setToolArgs={setToolArgs}
              toolLoading={toolLoading}
              toolResult={toolResult}
              onCallTool={handleCallTool}
              formatJson={formatJson}
            />
          )}
        </div>
      </div>
    </div>
  );

  return createPortal(content, document.body);
}

// --- Sub-components ---

function StatusBadge({ label, variant = 'default' }: { label: string; variant?: 'default' | 'green' | 'blue' }) {
  const colors = {
    default: 'bg-stone-100 text-stone-600',
    green: 'bg-sage-100 text-sage-600',
    blue: 'bg-primary-100 text-primary-500',
  };
  return (
    <span className={`px-2 py-0.5 text-[10px] font-mono rounded ${colors[variant]}`}>
      {label}
    </span>
  );
}

function StateTab({
  snap,
  formatJson,
}: {
  snap: SkillSnapshotRpc | null;
  formatJson: (v: unknown) => string;
}) {
  if (!snap) {
    return <div className="p-4 text-sm text-stone-500">Skill not running. No state available.</div>;
  }

  const stateEntries = Object.entries(snap.state || {});

  return (
    <div className="p-4 space-y-3">
      {/* Metadata */}
      <Section title="Metadata">
        <KV label="skill_id" value={snap.skill_id} />
        <KV label="status" value={snap.status} />
        <KV label="connection_status" value={snap.connection_status} />
        <KV label="setup_complete" value={String(snap.setup_complete)} />
        {snap.error && <KV label="error" value={snap.error} isError />}
      </Section>

      {/* Published state */}
      <Section title={`Published State (${stateEntries.length} keys)`}>
        {stateEntries.length === 0 ? (
          <p className="text-xs text-stone-500 italic">No published state</p>
        ) : (
          <pre className="text-xs text-stone-700 font-mono bg-stone-50 rounded-lg p-3 overflow-x-auto whitespace-pre-wrap break-all max-h-64 overflow-y-auto">
            {formatJson(snap.state)}
          </pre>
        )}
      </Section>

      {/* Raw snapshot */}
      <Section title="Raw Snapshot">
        <pre className="text-xs text-stone-600 font-mono bg-stone-50 rounded-lg p-3 overflow-x-auto whitespace-pre-wrap break-all max-h-48 overflow-y-auto">
          {formatJson(snap)}
        </pre>
      </Section>
    </div>
  );
}

function ToolsTab({
  snap,
  expandedTool,
  setExpandedTool,
  toolArgs,
  setToolArgs,
  toolLoading,
  toolResult,
  onCallTool,
  formatJson,
}: {
  snap: SkillSnapshotRpc | null;
  expandedTool: string | null;
  setExpandedTool: (name: string | null) => void;
  toolArgs: Record<string, string>;
  setToolArgs: (args: Record<string, string>) => void;
  toolLoading: string | null;
  toolResult: ToolCallResult | null;
  onCallTool: (name: string) => void;
  formatJson: (v: unknown) => string;
}) {
  if (!snap || snap.tools.length === 0) {
    return <div className="p-4 text-sm text-stone-500">No tools available.</div>;
  }

  return (
    <div className="p-3 space-y-1">
      {snap.tools.map(tool => {
        const isExpanded = expandedTool === tool.name;
        const isLoading = toolLoading === tool.name;
        const hasResult = toolResult?.toolName === tool.name;

        return (
          <div key={tool.name} className="border border-stone-200 rounded-xl overflow-hidden">
            {/* Tool header */}
            <button
              type="button"
              data-testid={`skill-debug-tool-header-${tool.name}`}
              onClick={() => setExpandedTool(isExpanded ? null : tool.name)}
              className="w-full flex items-center justify-between p-3 text-left hover:bg-stone-50 transition-colors">
              <div className="min-w-0">
                <span className="text-sm font-mono text-primary-500">{tool.name}</span>
                <p className="text-xs text-stone-400 mt-0.5 line-clamp-1">{tool.description}</p>
              </div>
              <svg
                className={`w-4 h-4 text-stone-500 flex-shrink-0 ml-2 transition-transform ${isExpanded ? 'rotate-180' : ''}`}
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
              </svg>
            </button>

            {/* Expanded content */}
            {isExpanded && (
              <div className="px-3 pb-3 space-y-2 border-t border-stone-200">
                {/* Input schema */}
                {tool.inputSchema != null && (
                  <div className="mt-2">
                    <label className="text-[10px] font-medium text-stone-500 uppercase tracking-wider">
                      Input Schema
                    </label>
                    <pre className="text-xs text-stone-600 font-mono bg-stone-50 rounded-lg p-2 mt-1 overflow-x-auto max-h-32 overflow-y-auto">
                      {formatJson(tool.inputSchema)}
                    </pre>
                  </div>
                )}

                {/* Args input */}
                <div>
                  <label className="text-[10px] font-medium text-stone-500 uppercase tracking-wider">
                    Arguments (JSON)
                  </label>
                  <textarea
                    data-testid={`skill-debug-tool-args-${tool.name}`}
                    value={toolArgs[tool.name] ?? '{}'}
                    onChange={e => setToolArgs({ ...toolArgs, [tool.name]: e.target.value })}
                    placeholder="{}"
                    rows={3}
                    className="w-full mt-1 px-3 py-2 text-xs font-mono bg-stone-50 border border-stone-200 rounded-lg text-stone-700 placeholder-stone-400 focus:outline-none focus:border-primary-500/50 resize-y"
                  />
                </div>

                {/* Call button */}
                <button
                  type="button"
                  data-testid={`skill-debug-execute-${tool.name}`}
                  onClick={() => onCallTool(tool.name)}
                  disabled={isLoading}
                  className="w-full py-2 text-xs font-medium bg-primary-50 text-primary-500 border border-primary-500/30 rounded-lg hover:bg-primary-100 disabled:opacity-50 transition-colors flex items-center justify-center gap-2">
                  {isLoading ? (
                    <>
                      <svg className="animate-spin w-3 h-3" fill="none" viewBox="0 0 24 24">
                        <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
                        <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
                      </svg>
                      Running...
                    </>
                  ) : (
                    'Execute Tool'
                  )}
                </button>

                {/* Result */}
                {hasResult && toolResult && (
                  <div className={`rounded-lg p-3 ${toolResult.isError ? 'bg-red-500/10 border border-red-500/30' : 'bg-sage-500/10 border border-sage-500/30'}`}>
                    <div className="flex items-center justify-between mb-1">
                      <span className={`text-[10px] font-medium uppercase tracking-wider ${toolResult.isError ? 'text-red-400' : 'text-sage-400'}`}>
                        {toolResult.isError ? 'Error' : 'Result'}
                      </span>
                      <span className="text-[10px] text-stone-500">{toolResult.durationMs}ms</span>
                    </div>
                    <pre className="text-xs font-mono text-stone-700 whitespace-pre-wrap break-all max-h-48 overflow-y-auto">
                      {formatJson(toolResult.result)}
                    </pre>
                  </div>
                )}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <h3 className="text-[10px] font-medium text-stone-500 uppercase tracking-wider mb-1.5">{title}</h3>
      {children}
    </div>
  );
}

function KV({ label, value, isError }: { label: string; value: string; isError?: boolean }) {
  return (
    <div className="flex items-center gap-2 py-0.5">
      <span className="text-xs text-stone-500 font-mono min-w-[120px]">{label}</span>
      <span className={`text-xs font-mono ${isError ? 'text-red-500' : 'text-stone-700'}`}>{value}</span>
    </div>
  );
}
