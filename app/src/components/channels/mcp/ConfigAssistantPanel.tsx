/**
 * Inline LLM-driven configuration assistant chat.
 * Maintains a local message history and calls config_assist with each send.
 * If the reply includes suggested_env, shows an "Apply suggested values" button
 * that passes them up to the caller (e.g. to pre-fill the install dialog).
 */
import debug from 'debug';
import { useCallback, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { mcpClientsApi } from '../../../services/api/mcpClientsApi';

const log = debug('mcp-clients:config-assist');

interface Message {
  role: 'user' | 'assistant';
  content: string;
  suggested_env?: Record<string, string>;
}

interface ConfigAssistantPanelProps {
  qualifiedName: string;
  onApplySuggestedEnv?: (env: Record<string, string>) => void;
}

const ConfigAssistantPanel = ({
  qualifiedName,
  onApplySuggestedEnv,
}: ConfigAssistantPanelProps) => {
  const { t } = useT();
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState('');
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const bottomRef = useRef<HTMLDivElement | null>(null);

  const scrollToBottom = useCallback(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, []);

  const handleSend = useCallback(async () => {
    const text = input.trim();
    if (!text || sending) return;

    const userMessage: Message = { role: 'user', content: text };
    const updatedHistory = [...messages, userMessage];
    setMessages(updatedHistory);
    setInput('');
    setSending(true);
    setError(null);
    log('sending message: %s', text);

    try {
      const result = await mcpClientsApi.configAssist({
        qualified_name: qualifiedName,
        user_message: text,
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
      setInput(text);
    } finally {
      setSending(false);
    }
  }, [input, messages, qualifiedName, sending, scrollToBottom, t]);

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
      <h4 className="text-xs font-semibold text-stone-700 dark:text-neutral-300">
        {t('mcp.configAssistant.title')}
      </h4>

      {/* Message list */}
      <div className="flex-1 overflow-y-auto space-y-2 min-h-0 max-h-64 rounded-lg border border-stone-100 dark:border-neutral-800 p-2">
        {messages.length === 0 && (
          <p className="text-xs text-stone-400 dark:text-neutral-500 py-2 text-center">
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
                  ? 'bg-primary-500 text-white'
                  : 'bg-stone-100 dark:bg-neutral-800 text-stone-800 dark:text-neutral-100'
              }`}>
              <p className="whitespace-pre-wrap">{msg.content}</p>
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
                      className="mt-1 rounded px-2 py-1 text-[11px] font-medium bg-white/20 hover:bg-white/30 transition-colors">
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
            <div className="rounded-lg px-3 py-2 text-sm bg-stone-100 dark:bg-neutral-800 text-stone-400 dark:text-neutral-500">
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
          className="flex-1 rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-1.5 text-sm text-stone-800 dark:text-neutral-100 placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-2 focus:ring-primary-500/40 disabled:opacity-50 resize-none"
        />
        <button
          type="button"
          disabled={sending || !input.trim()}
          onClick={() => void handleSend()}
          className="self-end rounded-lg bg-primary-500 px-3 py-2 text-sm font-medium text-white hover:bg-primary-600 disabled:opacity-50 transition-colors shrink-0">
          {t('mcp.configAssistant.send')}
        </button>
      </div>
    </div>
  );
};

export default ConfigAssistantPanel;
