import { useEffect, useState } from 'react';

import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import { alphahumanAgentChat } from '../../../utils/tauriCommands';

type ChatMessage = { role: 'user' | 'agent'; text: string };

const STORAGE_KEY = 'alphahuman.settings.agentChat.history';

const AgentChatPanel = () => {
  const { navigateBack } = useSettingsNavigation();
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState('');
  const [providerOverride, setProviderOverride] = useState('');
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
        providerOverride?: string;
        modelOverride?: string;
        temperature?: string;
      };
      if (parsed.messages && Array.isArray(parsed.messages)) {
        setMessages(parsed.messages);
      }
      if (parsed.providerOverride !== undefined) {
        setProviderOverride(parsed.providerOverride);
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
    const payload = {
      messages,
      providerOverride,
      modelOverride,
      temperature,
    };
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(payload));
    } catch {
      // Ignore storage errors (e.g., private mode)
    }
  }, [messages, providerOverride, modelOverride, temperature]);

  const sendMessage = async () => {
    const text = input.trim();
    if (!text || sending) return;
    setError('');
    setSending(true);
    setInput('');
    setMessages((prev) => [...prev, { role: 'user', text }]);
    try {
      const response = await alphahumanAgentChat(
        text,
        providerOverride.trim() ? providerOverride : undefined,
        modelOverride.trim() ? modelOverride : undefined,
        Number.isFinite(Number(temperature)) ? Number(temperature) : undefined
      );
      setMessages((prev) => [...prev, { role: 'agent', text: response.result }]);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
    } finally {
      setSending(false);
    }
  };

  return (
    <div className="h-full flex flex-col">
      <SettingsHeader title="Agent Chat" showBackButton={true} onBack={navigateBack} />

      <div className="flex-1 overflow-y-auto px-6 pb-10 space-y-6">
        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Overrides</h3>
          <div className="grid gap-3 md:grid-cols-3">
            <label className="space-y-2 text-sm text-gray-300">
              Provider
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="openai"
                value={providerOverride}
                onChange={(event) => setProviderOverride(event.target.value)}
              />
            </label>
            <label className="space-y-2 text-sm text-gray-300">
              Model
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="gpt-4.1-mini"
                value={modelOverride}
                onChange={(event) => setModelOverride(event.target.value)}
              />
            </label>
            <label className="space-y-2 text-sm text-gray-300">
              Temperature
              <input
                className="input input-bordered w-full text-slate-900 bg-white"
                placeholder="0.7"
                value={temperature}
                onChange={(event) => setTemperature(event.target.value)}
              />
            </label>
          </div>
        </section>

        <section className="space-y-3">
          <h3 className="text-lg font-semibold text-white">Conversation</h3>
          {error && (
            <div className="rounded-lg border border-red-500/40 bg-red-500/10 px-4 py-3 text-sm text-red-200">
              {error}
            </div>
          )}
          <div className="rounded-xl border border-stone-800/60 bg-black/40 p-4 space-y-3">
            {messages.length === 0 && (
              <div className="text-sm text-stone-400">Start a conversation with the agent.</div>
            )}
            {messages.map((message, index) => (
              <div key={`${message.role}-${index}`} className="space-y-1">
                <div className="text-[11px] uppercase tracking-wide text-stone-400">
                  {message.role === 'user' ? 'You' : 'Agent'}
                </div>
                <div
                  className={`text-sm whitespace-pre-wrap ${
                    message.role === 'user' ? 'text-white' : 'text-emerald-200'
                  }`}
                >
                  {message.text}
                </div>
              </div>
            ))}
          </div>
          <div className="space-y-2">
            <textarea
              className="textarea textarea-bordered w-full min-h-[140px] text-slate-900 bg-white"
              placeholder="Ask the agent anything..."
              value={input}
              onChange={(event) => setInput(event.target.value)}
            />
            <button className="btn btn-primary" onClick={sendMessage} disabled={sending}>
              {sending ? 'Sending…' : 'Send Message'}
            </button>
          </div>
        </section>
      </div>
    </div>
  );
};

export default AgentChatPanel;
