/**
 * Inline LLM-driven configuration assistant chat.
 * Maintains a local message history and calls config_assist with each send.
 * If the reply includes suggested_env, shows an "Apply suggested values" button
 * that passes them up to the caller (e.g. to pre-fill the install dialog).
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { BubbleMarkdown } from '../../../pages/conversations/components/AgentMessageBubble';
import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import Button from '../../ui/Button';

const log = debug('mcp-clients:config-assist');

interface Message {
  role: 'user' | 'assistant';
  content: string;
  suggested_env?: Record<string, string>;
}

// Per-server chat cache (keyed by qualified_name). Lets the help chat survive
// closing+reopening the modal (you keep your place) while you stay on the MCP's
// detail page. `clearConfigChat` is called when the detail page unmounts (back
// to the list), so re-entering a server starts fresh.
const chatCache = new Map<string, Message[]>();

/** Drop the cached help chat for a server (called on detail-page unmount). */
export function clearConfigChat(qualifiedName: string): void {
  chatCache.delete(qualifiedName);
}

interface ConfigAssistantPanelProps {
  qualifiedName: string;
  onApplySuggestedEnv?: (env: Record<string, string>) => void;
  /** A fixed, server-specific prompt auto-sent once on mount (so the user gets
   * guidance with a single click instead of having to know what to ask). */
  autoPrompt?: string;
}

const ConfigAssistantPanel = ({
  qualifiedName,
  onApplySuggestedEnv,
  autoPrompt,
}: ConfigAssistantPanelProps) => {
  const { t } = useT();
  // Restore any in-progress chat for this server (survives modal reopen).
  const [messages, setMessages] = useState<Message[]>(() => chatCache.get(qualifiedName) ?? []);
  const [input, setInput] = useState('');
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const bottomRef = useRef<HTMLDivElement | null>(null);

  const scrollToBottom = useCallback(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, []);

  const send = useCallback(
    async (text: string) => {
      const trimmed = text.trim();
      if (!trimmed || sending) return;

      const userMessage: Message = { role: 'user', content: trimmed };
      const updatedHistory = [...messages, userMessage];
      setMessages(updatedHistory);
      setSending(true);
      setError(null);
      log('sending message: %s', trimmed);

      try {
        const result = await mcpClientsApi.configAssist({
          qualified_name: qualifiedName,
          user_message: trimmed,
          history: updatedHistory.map(m => ({ role: m.role, content: m.content })),
        });
        log(
          'received reply length=%d suggested_env=%s',
          result.reply.length,
          result.suggested_env ? 'yes' : 'no'
        );

        const assistantMessage: Message = {
          role: 'assistant',
          content: result.reply,
          suggested_env: result.suggested_env,
        };
        setMessages(prev => [...prev, assistantMessage]);
        setTimeout(scrollToBottom, 50);
      } catch (err) {
        const msg = err instanceof Error ? err.message : t('mcp.configAssistant.failedResponse');
        log('config_assist error: %s', msg);
        setError(msg);
        setMessages(messages);
      } finally {
        setSending(false);
      }
    },
    [messages, qualifiedName, sending, scrollToBottom, t]
  );

  const handleSend = useCallback(() => {
    const text = input.trim();
    if (!text) return;
    setInput('');
    void send(text);
  }, [input, send]);

  // Persist the chat per-server so reopening the modal restores it.
  useEffect(() => {
    chatCache.set(qualifiedName, messages);
  }, [qualifiedName, messages]);

  // Auto-run the fixed prompt once — but only for a fresh chat. If we restored
  // an existing conversation from the cache, don't re-ask.
  const autoSent = useRef(false);
  useEffect(() => {
    if (autoPrompt && !autoSent.current && messages.length === 0) {
      autoSent.current = true;
      void send(autoPrompt);
    }
  }, [autoPrompt, send, messages.length]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        void handleSend();
      }
    },
    [handleSend]
  );

  return (
    <div className="flex flex-col h-full space-y-2">
      {/* Message list */}
      <div className="flex-1 overflow-y-auto space-y-2 min-h-0 rounded-lg border border-line-subtle p-2">
        {messages.length === 0 && (
          <p className="text-xs text-content-faint py-2 text-center">
            {t('mcp.configAssistant.empty')}
          </p>
        )}
        {messages.map((msg, idx) => (
          <div
            key={idx}
            className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}>
            <div
              className={`max-w-[85%] rounded-lg px-3 py-2 text-sm ${
                msg.role === 'user'
                  ? 'bg-primary-500 text-content-inverted'
                  : 'bg-surface-subtle text-content'
              }`}>
              {msg.role === 'assistant' ? (
                <BubbleMarkdown content={msg.content} tone="agent" />
              ) : (
                <p className="whitespace-pre-wrap">{msg.content}</p>
              )}
              {msg.suggested_env && Object.keys(msg.suggested_env).length > 0 && (
                <div className="mt-2 pt-2 border-t border-white/20 space-y-1">
                  <p className="text-[11px] font-medium opacity-80">
                    {t('mcp.configAssistant.suggestedValues')}
                  </p>
                  <ul className="space-y-0.5">
                    {Object.keys(msg.suggested_env).map(key => (
                      <li key={key} className="text-[11px] font-mono opacity-90">
                        {key}:{' '}
                        <span className="opacity-60">{t('mcp.configAssistant.valueHidden')}</span>
                      </li>
                    ))}
                  </ul>
                  {onApplySuggestedEnv && (
                    <button
                      type="button"
                      onClick={() => onApplySuggestedEnv(msg.suggested_env!)}
                      className="mt-1 rounded px-2 py-1 text-[11px] font-medium bg-surface/20 hover:bg-white/30 transition-colors">
                      {t('mcp.configAssistant.applySuggested')}
                    </button>
                  )}
                  {!onApplySuggestedEnv && (
                    <p className="text-[11px] opacity-70">
                      {t('mcp.configAssistant.reinstallHint')}
                    </p>
                  )}
                </div>
              )}
            </div>
          </div>
        ))}
        {sending && (
          <div className="flex justify-start">
            <div className="rounded-lg px-3 py-2 text-sm bg-surface-subtle text-content-faint">
              {t('mcp.configAssistant.thinking')}
            </div>
          </div>
        )}
        <div ref={bottomRef} />
      </div>

      {/* Error */}
      {error && (
        <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-xs text-coral-700 dark:text-coral-300">
          {error}
        </div>
      )}

      {/* Input row */}
      <div className="flex gap-2">
        <textarea
          rows={2}
          value={input}
          onChange={e => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          disabled={sending}
          placeholder={t('mcp.configAssistant.inputPlaceholder')}
          className="flex-1 rounded-lg border border-line bg-surface px-3 py-1.5 text-sm text-content placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-2 focus:ring-primary-500/40 disabled:opacity-50 resize-none"
        />
        <Button
          variant="primary"
          size="md"
          disabled={sending || !input.trim()}
          onClick={() => void handleSend()}
          className="self-end shrink-0">
          {t('mcp.configAssistant.send')}
        </Button>
      </div>
    </div>
  );
};

export default ConfigAssistantPanel;
