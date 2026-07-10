import { useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { openhumanAgentChat } from '../../../utils/tauriCommands';
import Button from '../../ui/Button';
import {
  SettingsEmptyState,
  SettingsRow,
  SettingsSection,
  SettingsStatusLine,
  SettingsTextArea,
  SettingsTextField,
} from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

type ChatMessage = { role: 'user' | 'agent'; text: string };

const STORAGE_KEY = 'openhuman.settings.agentChat.history';

const AgentChatPanel = () => {
  const { t } = useT();
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState('');
  const [modelOverride, setModelOverride] = useState('');
  const [temperature, setTemperature] = useState('0.7');
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string>('');

  useEffect(() => {
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (!raw) return;
      const parsed = JSON.parse(raw) as {
        messages?: ChatMessage[];
        modelOverride?: string;
        temperature?: string;
      };
      if (parsed.messages && Array.isArray(parsed.messages)) {
        setMessages(parsed.messages);
      }
      if (parsed.modelOverride !== undefined) {
        setModelOverride(parsed.modelOverride);
      }
      if (parsed.temperature !== undefined) {
        setTemperature(parsed.temperature);
      }
    } catch {
      // Ignore corrupt storage
    }
  }, []);

  useEffect(() => {
    const payload = { messages, modelOverride, temperature };
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(payload));
    } catch {
      // Ignore storage errors (e.g., private mode)
    }
  }, [messages, modelOverride, temperature]);

  const sendMessage = async () => {
    const text = input.trim();
    if (!text || sending) return;
    setError('');
    setSending(true);
    setInput('');
    setMessages(prev => [...prev, { role: 'user', text }]);
    try {
      const response = await openhumanAgentChat(
        text,
        modelOverride.trim() ? modelOverride : undefined,
        Number.isFinite(Number(temperature)) ? Number(temperature) : undefined
      );
      setMessages(prev => [...prev, { role: 'agent', text: response.result }]);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
    } finally {
      setSending(false);
    }
  };

  return (
    <SettingsPanel description={t('settings.developerMenu.agentChat.desc')}>
      <SettingsSection title={t('chat.overrides')} description={t('chat.agentChatDesc')}>
        <div className="grid md:grid-cols-2">
          <SettingsRow
            stacked
            htmlFor="agent-chat-model"
            label={t('chat.model')}
            control={
              <SettingsTextField
                id="agent-chat-model"
                placeholder={t('chat.modelPlaceholder')}
                value={modelOverride}
                onChange={event => setModelOverride(event.target.value)}
                aria-label={t('chat.model')}
              />
            }
          />
          <SettingsRow
            stacked
            htmlFor="agent-chat-temperature"
            label={t('chat.temperature')}
            control={
              <SettingsTextField
                id="agent-chat-temperature"
                placeholder="0.7"
                value={temperature}
                onChange={event => setTemperature(event.target.value)}
                aria-label={t('chat.temperature')}
              />
            }
          />
        </div>
      </SettingsSection>

      <SettingsSection title={t('chat.conversation')}>
        <div className="px-4 py-3 space-y-3">
          <SettingsStatusLine saving={false} error={error || null} savingLabel="" />
          <div className="rounded-xl border border-line bg-surface-muted p-4 space-y-3 min-h-[6rem]">
            {messages.length === 0 ? (
              <SettingsEmptyState label={t('chat.startAgentConversation')} />
            ) : (
              messages.map((message, index) => (
                <div key={`${message.role}-${index}`} className="space-y-1">
                  <div className="text-[11px] uppercase tracking-wide text-content-muted">
                    {message.role === 'user' ? t('chat.you') : t('chat.agent')}
                  </div>
                  <div
                    className={`text-sm whitespace-pre-wrap ${
                      message.role === 'user'
                        ? 'text-content'
                        : 'text-emerald-700 dark:text-emerald-300'
                    }`}>
                    {message.text}
                  </div>
                </div>
              ))
            )}
          </div>
          <div className="space-y-2">
            <SettingsTextArea
              placeholder={t('chat.askAgent')}
              value={input}
              onChange={event => setInput(event.target.value)}
              rows={4}
              aria-label={t('chat.askAgent')}
            />
            <Button
              type="button"
              variant="primary"
              size="sm"
              onClick={() => void sendMessage()}
              disabled={sending}>
              {sending ? t('common.loading') : t('chat.sendMessage')}
            </Button>
          </div>
        </div>
      </SettingsSection>
    </SettingsPanel>
  );
};

export default AgentChatPanel;
