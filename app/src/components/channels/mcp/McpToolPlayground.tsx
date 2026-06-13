/**
 * Tool Execution Playground — modal for interactively invoking a single
 * MCP tool against a connected server.
 *
 * Lives next to `McpToolList`: clicking the "Try" button on a tool opens
 * this modal. The parent (`InstalledServerDetail`) holds the `serverId`
 * and the currently-targeted `tool`; this component renders the modal
 * UI and orchestrates the round-trip through `mcpClientsApi.toolCall`.
 *
 * Features:
 *   - JSON args editor with validate + format buttons; Cmd/Ctrl+Enter to
 *     run; Esc to close (does NOT trigger run).
 *   - Result/error display with copy-to-clipboard.
 *   - In-session invocation history (last 10) with one-click "load" to
 *     restore an earlier set of args.
 *   - Collapsible input-schema viewer so callers can see the JSON-schema
 *     contract before composing args.
 *
 * Intentional non-features:
 *   - No JSON-schema-driven form generation. Args are typed as raw JSON;
 *     keeps the surface predictable and avoids re-implementing JSON-schema
 *     coercion semantics (the upstream tool can validate stricter).
 *   - No persistence across modal closes. History is session-only; this
 *     is a debug/exploration surface, not a saved workspace.
 *   - No global keyboard shortcut for opening the modal (would clash with
 *     the app-wide CommandProvider).
 */
import {
  type KeyboardEvent as ReactKeyboardEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import type { McpTool } from './types';

interface McpToolPlaygroundProps {
  serverId: string;
  tool: McpTool;
  onClose: () => void;
}

interface InvocationRecord {
  /** Local-timezone HH:MM:SS string captured at submit. */
  timestamp: string;
  /** Raw args string the user submitted. */
  argsJson: string;
  /** JSON-stringified result if the tool returned successfully. */
  resultText: string;
  /** True if the tool itself reported is_error OR an exception was thrown. */
  isError: boolean;
}

const HISTORY_LIMIT = 10;
const EMPTY_ARGS = '{}';

/**
 * Try to pretty-print whatever value the user typed. Returns the
 * original string unchanged if it isn't valid JSON — never throws.
 */
const formatArgs = (raw: string): string => {
  const trimmed = raw.trim();
  if (!trimmed) return EMPTY_ARGS;
  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2);
  } catch {
    return raw;
  }
};

/**
 * Parse the args textarea into a value for the tool call. Empty input is
 * treated as `{}`. Returns a discriminated result rather than throwing so the
 * caller can keep JSON-parse failures (user input) cleanly separate from RPC
 * failures (the actual tool call) — they surface to the user differently.
 */
export type ParsedToolArgs = { ok: true; value: unknown } | { ok: false; error: string };

export const parseToolArgs = (argsJson: string, fallbackMessage: string): ParsedToolArgs => {
  if (argsJson.trim() === '') return { ok: true, value: {} };
  try {
    return { ok: true, value: JSON.parse(argsJson) };
  } catch (err) {
    return { ok: false, error: err instanceof Error ? err.message : fallbackMessage };
  }
};

const stringifyResult = (value: unknown): string => {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
};

const formatTimestamp = (date: Date): string =>
  date.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });

const McpToolPlayground = ({ serverId, tool, onClose }: McpToolPlaygroundProps) => {
  const { t } = useT();
  const [argsJson, setArgsJson] = useState<string>(EMPTY_ARGS);
  const [parseError, setParseError] = useState<string | null>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [resultText, setResultText] = useState<string | null>(null);
  const [resultIsError, setResultIsError] = useState(false);
  const [showSchema, setShowSchema] = useState(false);
  const [showHistory, setShowHistory] = useState(false);
  const [copyStatus, setCopyStatus] = useState<'idle' | 'copied'>('idle');
  const [history, setHistory] = useState<InvocationRecord[]>([]);
  const argsTextareaRef = useRef<HTMLTextAreaElement>(null);

  // Esc closes; click-outside the dialog card also closes. We attach the
  // keydown listener to document so the modal handles Esc regardless of
  // which child has focus.
  useEffect(() => {
    const handleDocumentKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener('keydown', handleDocumentKey);
    return () => document.removeEventListener('keydown', handleDocumentKey);
  }, [onClose]);

  // Auto-focus the args editor on mount so keyboard-first users land
  // exactly where they need to type.
  useEffect(() => {
    argsTextareaRef.current?.focus();
  }, []);

  const schemaJson = useMemo(() => stringifyResult(tool.input_schema), [tool.input_schema]);

  const handleArgsChange = useCallback((next: string) => {
    setArgsJson(next);
    // Live-clear stale parse errors; they re-appear on Run if still bad.
    setParseError(null);
  }, []);

  const handleFormat = useCallback(() => {
    setArgsJson(prev => formatArgs(prev));
    setParseError(null);
  }, []);

  const handleRun = useCallback(async () => {
    if (isRunning) return;
    // Parse args first; refuse to call the RPC with bad input.
    const parsed = parseToolArgs(argsJson, t('mcp.playground.invalidJson'));
    if (!parsed.ok) {
      setParseError(parsed.error);
      setResultText(null);
      return;
    }
    setParseError(null);
    setIsRunning(true);
    setResultText(null);
    setResultIsError(false);
    // Reset the copy-feedback chip so a stale "Copied" label doesn't
    // briefly persist over the next result — the Copy timer has its
    // own 1.5s reset, but starting a new run is itself a clear signal
    // the prior result is gone.
    setCopyStatus('idle');
    const submittedArgs = argsJson;
    const timestamp = formatTimestamp(new Date());
    try {
      const callResult = await mcpClientsApi.toolCall({
        server_id: serverId,
        tool_name: tool.name,
        arguments: parsed.value,
      });
      const text = stringifyResult(callResult.result);
      const isError = Boolean(callResult.is_error);
      setResultText(text);
      setResultIsError(isError);
      setHistory(prev =>
        [{ timestamp, argsJson: submittedArgs, resultText: text, isError }, ...prev].slice(
          0,
          HISTORY_LIMIT
        )
      );
    } catch (err) {
      const msg = err instanceof Error ? err.message : t('mcp.playground.unexpectedError');
      setResultText(msg);
      setResultIsError(true);
      setHistory(prev =>
        [{ timestamp, argsJson: submittedArgs, resultText: msg, isError: true }, ...prev].slice(
          0,
          HISTORY_LIMIT
        )
      );
    } finally {
      setIsRunning(false);
    }
  }, [argsJson, isRunning, serverId, t, tool.name]);

  // Cmd/Ctrl+Enter from the textarea triggers Run. We do NOT propagate
  // the keydown to the document Esc listener — only the Enter+modifier
  // combination is intercepted.
  const handleTextareaKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLTextAreaElement>) => {
      if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') {
        event.preventDefault();
        void handleRun();
      }
    },
    [handleRun]
  );

  const handleCopyResult = useCallback(async () => {
    if (resultText == null) return;
    if (typeof navigator === 'undefined' || !navigator.clipboard) return;
    try {
      await navigator.clipboard.writeText(resultText);
      setCopyStatus('copied');
      window.setTimeout(() => setCopyStatus('idle'), 1500);
    } catch {
      // Best-effort copy — silently ignore platforms / contexts where
      // clipboard access is denied. The result is still visible.
    }
  }, [resultText]);

  const handleLoadFromHistory = useCallback((record: InvocationRecord) => {
    setArgsJson(record.argsJson);
    setParseError(null);
    argsTextareaRef.current?.focus();
  }, []);

  // Click on the backdrop (not the dialog card) closes.
  const handleBackdropMouseDown = useCallback(
    (event: React.MouseEvent<HTMLDivElement>) => {
      if (event.target === event.currentTarget) {
        onClose();
      }
    },
    [onClose]
  );

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="mcp-playground-title"
      onMouseDown={handleBackdropMouseDown}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 px-4 py-6 overflow-y-auto">
      <div className="bg-white dark:bg-neutral-900 rounded-xl shadow-xl max-w-2xl w-full p-5 max-h-full overflow-y-auto">
        {/* Header */}
        <div className="flex items-start justify-between gap-3 mb-3">
          <div className="min-w-0">
            <h2
              id="mcp-playground-title"
              className="text-base font-semibold text-stone-900 dark:text-neutral-100 font-mono break-words">
              {t('mcp.playground.title').replace('{name}', tool.name)}
            </h2>
            {tool.description && (
              <p className="text-xs text-stone-500 dark:text-neutral-400 mt-1">
                {tool.description}
              </p>
            )}
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label={t('mcp.playground.close')}
            className="shrink-0 rounded-lg p-1.5 text-stone-400 hover:text-stone-700 dark:text-neutral-500 dark:hover:text-neutral-200 transition-colors">
            <svg
              className="w-4 h-4"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
              strokeWidth={2}
              aria-hidden="true">
              <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Input schema (collapsible) */}
        <div className="mb-3">
          <button
            type="button"
            onClick={() => setShowSchema(prev => !prev)}
            aria-expanded={showSchema}
            className="flex items-center gap-1.5 text-xs font-medium text-stone-600 dark:text-neutral-300 hover:text-stone-900 dark:hover:text-neutral-100">
            <span
              className={`transition-transform ${showSchema ? 'rotate-90' : ''}`}
              aria-hidden="true">
              ▶
            </span>
            {t('mcp.playground.inputSchema')}
          </button>
          {showSchema && (
            <pre
              data-testid="mcp-playground-schema"
              className="mt-1.5 max-h-40 overflow-auto rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-950 p-2 text-[11px] font-mono text-stone-700 dark:text-neutral-200 whitespace-pre-wrap break-words">
              {schemaJson}
            </pre>
          )}
        </div>

        {/* Args editor */}
        <div className="mb-3">
          <div className="flex items-center justify-between mb-1.5">
            <label
              htmlFor="mcp-playground-args"
              className="text-xs font-medium text-stone-600 dark:text-neutral-300">
              {t('mcp.playground.argsLabel')}
            </label>
            <div className="flex items-center gap-2">
              <span className="text-[10px] text-stone-400 dark:text-neutral-500">
                {t('mcp.playground.runShortcut')}
              </span>
              <button
                type="button"
                onClick={handleFormat}
                aria-label={t('mcp.playground.format')}
                className="text-[10px] font-medium text-primary-600 dark:text-primary-300 hover:underline">
                {t('mcp.playground.format')}
              </button>
            </div>
          </div>
          <textarea
            id="mcp-playground-args"
            ref={argsTextareaRef}
            value={argsJson}
            onChange={e => handleArgsChange(e.target.value)}
            onKeyDown={handleTextareaKeyDown}
            spellCheck={false}
            rows={6}
            aria-label={t('mcp.playground.argsLabel')}
            aria-describedby="mcp-playground-args-help"
            className="w-full rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-xs font-mono text-stone-800 dark:text-neutral-100 focus:outline-none focus:ring-2 focus:ring-primary-400 focus:border-primary-400 resize-y"
          />
          <p
            id="mcp-playground-args-help"
            className="mt-1 text-[10px] text-stone-400 dark:text-neutral-500">
            {t('mcp.playground.argsHelp')}
          </p>
          {parseError && (
            <p role="alert" className="mt-1 text-[11px] text-coral-700 dark:text-coral-300">
              {t('mcp.playground.invalidJson')}: {parseError}
            </p>
          )}
        </div>

        {/* Run button */}
        <div className="flex justify-end gap-2 mb-4">
          <button
            type="button"
            onClick={() => void handleRun()}
            disabled={isRunning}
            className="rounded-lg bg-primary-500 px-4 py-1.5 text-xs font-semibold text-white hover:bg-primary-600 disabled:opacity-50 disabled:cursor-not-allowed">
            {isRunning ? t('mcp.playground.running') : t('mcp.playground.run')}
          </button>
        </div>

        {/* Result */}
        {resultText !== null && (
          <div className="mb-4">
            <div className="flex items-center justify-between mb-1.5">
              <p className="text-xs font-medium text-stone-600 dark:text-neutral-300">
                {resultIsError ? t('mcp.playground.resultError') : t('mcp.playground.result')}
              </p>
              <button
                type="button"
                onClick={() => void handleCopyResult()}
                aria-label={t('mcp.playground.copyResult')}
                className="text-[10px] font-medium text-primary-600 dark:text-primary-300 hover:underline">
                {copyStatus === 'copied'
                  ? t('mcp.playground.copied')
                  : t('mcp.playground.copyResult')}
              </button>
            </div>
            <pre
              data-testid="mcp-playground-result"
              role={resultIsError ? 'alert' : 'status'}
              aria-live={resultIsError ? 'assertive' : 'polite'}
              className={`max-h-60 overflow-auto rounded-lg border p-2 text-[11px] font-mono whitespace-pre-wrap break-words ${
                resultIsError
                  ? 'border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 text-coral-700 dark:text-coral-300'
                  : 'border-sage-200 dark:border-sage-500/30 bg-sage-50 dark:bg-sage-500/10 text-stone-800 dark:text-neutral-100'
              }`}>
              {resultText}
            </pre>
          </div>
        )}

        {/* History */}
        <div>
          <button
            type="button"
            onClick={() => setShowHistory(prev => !prev)}
            aria-expanded={showHistory}
            className="flex items-center gap-1.5 text-xs font-medium text-stone-600 dark:text-neutral-300 hover:text-stone-900 dark:hover:text-neutral-100">
            <span
              className={`transition-transform ${showHistory ? 'rotate-90' : ''}`}
              aria-hidden="true">
              ▶
            </span>
            {t('mcp.playground.history')} ({history.length})
          </button>
          {showHistory && (
            <div className="mt-1.5">
              {history.length === 0 ? (
                <p className="text-[11px] text-stone-400 dark:text-neutral-500">
                  {t('mcp.playground.historyEmpty')}
                </p>
              ) : (
                <ul className="space-y-1">
                  {history.map((record, idx) => (
                    <li
                      key={`${record.timestamp}-${idx}`}
                      className="flex items-center justify-between gap-2 rounded border border-stone-200 dark:border-neutral-800 px-2 py-1">
                      <div className="min-w-0 flex items-center gap-2">
                        <span
                          className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                            record.isError ? 'bg-coral-500' : 'bg-sage-500'
                          }`}
                          aria-hidden="true"
                        />
                        <span className="text-[10px] font-mono text-stone-500 dark:text-neutral-400">
                          {record.timestamp}
                        </span>
                        <span className="text-[10px] text-stone-600 dark:text-neutral-300 truncate">
                          {record.argsJson}
                        </span>
                      </div>
                      <button
                        type="button"
                        onClick={() => handleLoadFromHistory(record)}
                        aria-label={t('mcp.playground.historyLoad')}
                        className="shrink-0 text-[10px] font-medium text-primary-600 dark:text-primary-300 hover:underline">
                        {t('mcp.playground.historyLoad')}
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
};

export default McpToolPlayground;
