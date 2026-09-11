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
import Button from '../../ui/Button';
import { ModalShell } from '../../ui/ModalShell';
import TextArea from '../../ui/TextArea';
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
type ParsedToolArgs = { ok: true; value: unknown } | { ok: false; error: string };

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

  // Esc-to-close and click-outside-to-close are now `ModalShell` / Radix
  // `Dialog` behavior (its own document-level Escape listener + backdrop
  // pointerdown-outside), so there is no hand-rolled listener here any more.

  // Auto-focus the args editor on mount so keyboard-first users land exactly
  // where they need to type. `ModalShell` / Radix `Dialog` runs its own
  // focus-trap auto-focus in an effect that commits after this component's
  // children (React fires passive effects bottom-up), so a plain
  // `useEffect` here would be overridden immediately after. Defer to the
  // next animation frame so this one wins.
  useEffect(() => {
    const raf = window.requestAnimationFrame(() => argsTextareaRef.current?.focus());
    return () => window.cancelAnimationFrame(raf);
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

  return (
    <ModalShell
      onClose={onClose}
      titleId="mcp-playground-title"
      title={
        <span className="font-mono wrap-break-word">
          {t('mcp.playground.title').replace('{name}', tool.name)}
        </span>
      }
      subtitle={tool.description}
      maxWidthClassName="max-w-2xl"
      contentClassName="max-h-full overflow-y-auto p-5">
      <>
        {/* Input schema (collapsible) */}
        <div className="mb-3">
          <Button
            variant="tertiary"
            size="xs"
            onClick={() => setShowSchema(prev => !prev)}
            aria-expanded={showSchema}
            className="h-auto gap-1.5 p-0 text-xs font-medium text-content-secondary hover:text-content">
            <span
              className={`transition-transform ${showSchema ? 'rotate-90' : ''}`}
              aria-hidden="true">
              ▶
            </span>
            {t('mcp.playground.inputSchema')}
          </Button>
          {showSchema && (
            <pre
              data-testid="mcp-playground-schema"
              className="mt-1.5 max-h-40 overflow-auto rounded-lg border border-line bg-surface-muted p-2 text-[11px] font-mono text-content-secondary whitespace-pre-wrap wrap-break-word">
              {schemaJson}
            </pre>
          )}
        </div>

        {/* Args editor */}
        <div className="mb-3">
          <div className="flex items-center justify-between mb-1.5">
            <label
              htmlFor="mcp-playground-args"
              className="text-xs font-medium text-content-secondary">
              {t('mcp.playground.argsLabel')}
            </label>
            <div className="flex items-center gap-2">
              <span className="text-[10px] text-content-faint">
                {t('mcp.playground.runShortcut')}
              </span>
              <Button
                variant="tertiary"
                size="xs"
                onClick={handleFormat}
                aria-label={t('mcp.playground.format')}
                className="h-auto p-0 text-[10px] font-medium text-primary-600 hover:underline dark:text-primary-300">
                {t('mcp.playground.format')}
              </Button>
            </div>
          </div>
          <TextArea
            id="mcp-playground-args"
            ref={argsTextareaRef}
            value={argsJson}
            onChange={e => handleArgsChange(e.target.value)}
            onKeyDown={handleTextareaKeyDown}
            spellCheck={false}
            rows={6}
            aria-label={t('mcp.playground.argsLabel')}
            aria-describedby="mcp-playground-args-help"
            className="w-full font-mono text-xs focus:border-primary-400 focus:ring-primary-400"
          />
          <p id="mcp-playground-args-help" className="mt-1 text-[10px] text-content-faint">
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
          <Button variant="primary" size="sm" onClick={() => void handleRun()} disabled={isRunning}>
            {isRunning ? t('mcp.playground.running') : t('mcp.playground.run')}
          </Button>
        </div>

        {/* Result */}
        {resultText !== null && (
          <div className="mb-4">
            <div className="flex items-center justify-between mb-1.5">
              <p className="text-xs font-medium text-content-secondary">
                {resultIsError ? t('mcp.playground.resultError') : t('mcp.playground.result')}
              </p>
              <Button
                variant="tertiary"
                size="xs"
                onClick={() => void handleCopyResult()}
                aria-label={t('mcp.playground.copyResult')}
                className="h-auto p-0 text-[10px] font-medium text-primary-600 hover:underline dark:text-primary-300">
                {copyStatus === 'copied'
                  ? t('mcp.playground.copied')
                  : t('mcp.playground.copyResult')}
              </Button>
            </div>
            <pre
              data-testid="mcp-playground-result"
              role={resultIsError ? 'alert' : 'status'}
              aria-live={resultIsError ? 'assertive' : 'polite'}
              className={`max-h-60 overflow-auto rounded-lg border p-2 text-[11px] font-mono whitespace-pre-wrap wrap-break-word ${
                resultIsError
                  ? 'border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 text-coral-700 dark:text-coral-300'
                  : 'border-sage-200 dark:border-sage-500/30 bg-sage-50 dark:bg-sage-500/10 text-content'
              }`}>
              {resultText}
            </pre>
          </div>
        )}

        {/* History */}
        <div>
          <Button
            variant="tertiary"
            size="xs"
            onClick={() => setShowHistory(prev => !prev)}
            aria-expanded={showHistory}
            className="h-auto gap-1.5 p-0 text-xs font-medium text-content-secondary hover:text-content">
            <span
              className={`transition-transform ${showHistory ? 'rotate-90' : ''}`}
              aria-hidden="true">
              ▶
            </span>
            {t('mcp.playground.history')} ({history.length})
          </Button>
          {showHistory && (
            <div className="mt-1.5">
              {history.length === 0 ? (
                <p className="text-[11px] text-content-faint">{t('mcp.playground.historyEmpty')}</p>
              ) : (
                <ul className="space-y-1">
                  {history.map((record, idx) => (
                    <li
                      key={`${record.timestamp}-${idx}`}
                      className="flex items-center justify-between gap-2 rounded border border-line px-2 py-1">
                      <div className="min-w-0 flex items-center gap-2">
                        <span
                          className={`w-1.5 h-1.5 rounded-full shrink-0 ${
                            record.isError ? 'bg-coral-500' : 'bg-sage-500'
                          }`}
                          aria-hidden="true"
                        />
                        <span className="text-[10px] font-mono text-content-muted">
                          {record.timestamp}
                        </span>
                        <span className="text-[10px] text-content-secondary truncate">
                          {record.argsJson}
                        </span>
                      </div>
                      <Button
                        variant="tertiary"
                        size="xs"
                        onClick={() => handleLoadFromHistory(record)}
                        aria-label={t('mcp.playground.historyLoad')}
                        className="h-auto shrink-0 p-0 text-[10px] font-medium text-primary-600 hover:underline dark:text-primary-300">
                        {t('mcp.playground.historyLoad')}
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </div>
      </>
    </ModalShell>
  );
};

export default McpToolPlayground;
