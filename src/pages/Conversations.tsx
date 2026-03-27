import { useEffect, useRef, useState } from 'react';
import Markdown from 'react-markdown';
import { useNavigate } from 'react-router-dom';

import { creditsApi, type TeamUsage } from '../services/api/creditsApi';
import { inferenceApi, type ModelInfo } from '../services/api/inferenceApi';
import {
  chatCancel,
  chatSend,
  type ChatToolCallEvent,
  type ChatToolResultEvent,
  subscribeChatEvents,
  useRustChat,
} from '../services/chatService';
import { store } from '../store';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import type { NotionPageSummary, NotionSummary, NotionUserProfile } from '../store/notionSlice';
import {
  addInferenceResponse,
  addMessageLocal,
  createThreadLocal,
  fetchSuggestedQuestions,
  setActiveThread,
  setLastViewed,
  setSelectedThread,
} from '../store/threadSlice';
import type { ThreadMessage } from '../types/thread';
import { BACKEND_URL } from '../utils/config';

const DEFAULT_THREAD_ID = 'default-thread';
const DEFAULT_THREAD_TITLE = 'Conversation';
type ToolTimelineEntryStatus = 'running' | 'success' | 'error';

interface ToolTimelineEntry {
  id: string;
  name: string;
  round: number;
  status: ToolTimelineEntryStatus;
}

function buildNotionContext(
  profile: NotionUserProfile | null,
  pages: NotionPageSummary[],
  summaries: NotionSummary[],
  workspaceName: string | null
): string | null {
  if (!profile && pages.length === 0) return null;

  const lines: string[] = ['[NOTION_CONTEXT]'];

  if (workspaceName) lines.push(`Workspace: ${workspaceName}`);
  if (profile) {
    const who = [profile.name, profile.email].filter(Boolean).join(' · ');
    if (who) lines.push(`Connected as: ${who}`);
  }

  if (pages.length > 0) {
    lines.push(`\nRecent Pages (${pages.length} total):`);
    const top = pages.slice(0, 10);
    for (const p of top) {
      const urlPart = p.url ? ` — ${p.url}` : '';
      lines.push(`• ${p.title}${urlPart}`);
    }
  }

  if (summaries.length > 0) {
    lines.push('\nAI Page Summaries:');
    const top = summaries.slice(0, 5);
    for (const s of top) {
      const meta = [s.category, s.sentiment !== 'neutral' ? s.sentiment : null]
        .filter(Boolean)
        .join(', ');
      const topicStr = s.topics.length > 0 ? ` | Topics: ${s.topics.slice(0, 4).join(', ')}` : '';
      lines.push(`• ${s.summary}${meta ? ` [${meta}]` : ''}${topicStr}`);
    }
  }

  lines.push('[/NOTION_CONTEXT]');
  return lines.join('\n');
}

function formatRelativeTime(dateStr: string): string {
  const now = Date.now();
  const then = new Date(dateStr).getTime();
  const diffMs = now - then;
  if (diffMs < 60_000) return 'just now';
  const mins = Math.floor(diffMs / 60_000);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

const Conversations = () => {
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const {
    threads,
    selectedThreadId,
    messages,
    isLoadingMessages,
    messagesError,
    suggestedQuestions,
    isLoadingSuggestions,
    activeThreadId,
  } = useAppSelector(state => state.thread);

  const notionProfile = useAppSelector(state => state.notion.profile);
  const notionPages = useAppSelector(state => state.notion.pages);
  const notionSummaries = useAppSelector(state => state.notion.summaries);
  const notionWorkspaceName = useAppSelector(
    state =>
      ((state.skills.skillStates?.notion as Record<string, unknown> | undefined)?.workspaceName as
        | string
        | null) ?? null
  );

  const [inputValue, setInputValue] = useState('');
  const [copiedMessageId, setCopiedMessageId] = useState<string | null>(null);

  const [availableModels, setAvailableModels] = useState<ModelInfo[]>([]);
  const [selectedModel, setSelectedModel] = useState('neocortex-mk1');
  const [isLoadingModels, setIsLoadingModels] = useState(false);
  const [isSending, setIsSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [toolTimelineByThread, setToolTimelineByThread] = useState<
    Record<string, ToolTimelineEntry[]>
  >({});
  const rustChat = useRustChat();

  const selectedThreadIdRef = useRef(selectedThreadId);
  useEffect(() => {
    selectedThreadIdRef.current = selectedThreadId;
  }, [selectedThreadId]);

  const [teamUsage, setTeamUsage] = useState<TeamUsage | null>(null);
  const [isLoadingBudget, setIsLoadingBudget] = useState(false);

  const messagesEndRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const defaultThread = threads.find(t => t.id === DEFAULT_THREAD_ID);

    if (!defaultThread) {
      dispatch(
        createThreadLocal({
          id: DEFAULT_THREAD_ID,
          title: DEFAULT_THREAD_TITLE,
          createdAt: new Date().toISOString(),
        })
      );
    }

    if (selectedThreadId !== DEFAULT_THREAD_ID) {
      dispatch(setSelectedThread(DEFAULT_THREAD_ID));
    }
  }, [dispatch, selectedThreadId, threads]);

  useEffect(() => {
    setIsLoadingModels(true);
    inferenceApi
      .listModels()
      .then(data => {
        if (data.data.length > 0) {
          setAvailableModels(data.data);
          setSelectedModel(data.data[0].id);
        }
      })
      .catch(() => {
        // Keep default model on failure
      })
      .finally(() => setIsLoadingModels(false));
  }, []);

  useEffect(() => {
    setIsLoadingBudget(true);
    creditsApi
      .getTeamUsage()
      .then(data => setTeamUsage(data))
      .catch(() => {
        // Budget unavailable — silently ignore
      })
      .finally(() => setIsLoadingBudget(false));
  }, []);

  useEffect(() => {
    if (selectedThreadId) dispatch(setLastViewed(selectedThreadId));
  }, [selectedThreadId, dispatch]);

  useEffect(() => {
    if (selectedThreadId && messages.length === 0) {
      dispatch(fetchSuggestedQuestions(selectedThreadId));
    }
  }, [selectedThreadId, messages.length, dispatch]);

  useEffect(() => {
    if (messages.length > 0) {
      messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [messages]);

  useEffect(() => {
    if (sendError && inputValue.length > 0) {
      setSendError(null);
    }
  }, [inputValue, sendError]);

  useEffect(() => {
    if (!rustChat) return;

    let cleanup: (() => void) | null = null;
    let mounted = true;

    subscribeChatEvents({
      onToolCall: (event: ChatToolCallEvent) => {
        setToolTimelineByThread(prev => {
          const existing = prev[event.thread_id] ?? [];
          return {
            ...prev,
            [event.thread_id]: [
              ...existing,
              {
                id: `${event.thread_id}:${event.round}:${existing.length}:${event.tool_name}`,
                name: event.tool_name,
                round: event.round,
                status: 'running',
              },
            ],
          };
        });
      },
      onToolResult: (event: ChatToolResultEvent) => {
        setToolTimelineByThread(prev => {
          const existing = prev[event.thread_id] ?? [];
          if (existing.length === 0) return prev;

          const nextEntries = [...existing];
          let changed = false;
          for (let i = nextEntries.length - 1; i >= 0; i--) {
            const entry = nextEntries[i];
            if (
              entry.status === 'running' &&
              entry.name === event.tool_name &&
              entry.round === event.round
            ) {
              nextEntries[i] = {
                ...entry,
                status: event.success ? 'success' : 'error',
              };
              changed = true;
              break;
            }
          }

          if (!changed) return prev;
          return { ...prev, [event.thread_id]: nextEntries };
        });
      },
      onDone: event => {
        const currentState = store.getState() as {
          thread: { messagesByThreadId: Record<string, ThreadMessage[]> };
        };
        const threadMessages = currentState.thread.messagesByThreadId[event.thread_id] || [];
        const lastMsg = threadMessages[threadMessages.length - 1];
        if (lastMsg?.sender === 'agent' && lastMsg?.content === event.full_response) {
          return;
        }

        dispatch(addInferenceResponse({ content: event.full_response, threadId: event.thread_id }));
        setToolTimelineByThread(prev => {
          const existing = prev[event.thread_id] ?? [];
          if (existing.length === 0) return prev;
          return {
            ...prev,
            [event.thread_id]: existing.map(entry =>
              entry.status === 'running' ? { ...entry, status: 'success' as const } : entry
            ),
          };
        });
        setIsSending(false);
        dispatch(setActiveThread(null));
      },
      onError: event => {
        if (event.thread_id !== selectedThreadIdRef.current) return;
        setIsSending(false);
        setToolTimelineByThread(prev => {
          const existing = prev[event.thread_id] ?? [];
          if (existing.length === 0) return prev;
          return {
            ...prev,
            [event.thread_id]: existing.map(entry =>
              entry.status === 'running' ? { ...entry, status: 'error' as const } : entry
            ),
          };
        });

        if (event.error_type !== 'cancelled') {
          dispatch(
            addInferenceResponse({
              content: 'Something went wrong — please try again.',
              threadId: event.thread_id,
            })
          );
        } else {
          dispatch(setActiveThread(null));
        }
      },
    }).then(fn => {
      if (mounted) cleanup = fn;
    });

    return () => {
      mounted = false;
      cleanup?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rustChat]);

  const handleSelectThread = (threadId: string) => {
    if (threadId === selectedThreadId) return;
    navigate(`/conversations/${threadId}`, { replace: true });
  };

  const handleNewThread = () => {
    const threadId = crypto.randomUUID();
    dispatch(
      createThreadLocal({
        id: threadId,
        title: 'New Conversation',
        createdAt: new Date().toISOString(),
      })
    );
    navigate(`/conversations/${threadId}`, { replace: true });
  };

  const handleDeleteThread = (threadId: string) => {
    dispatch(deleteThreadLocal(threadId));
    setConfirmDeleteId(null);
    if (threadId === selectedThreadId) {
      navigate('/conversations', { replace: true });
    }
  };

  const handlePurge = async () => {
    const result = await dispatch(purgeThreads());
    if (purgeThreads.fulfilled.match(result)) {
      setShowPurgeConfirm(false);
      navigate('/conversations', { replace: true });
    }
  };

  const handleSendMessage = async (text?: string) => {
    const normalized = text ?? inputValue;
    const trimmed = normalized.trim();

    if (!trimmed || !selectedThreadId || isSending) return;
    if (!rustChat) {
      setSendError('Desktop runtime required for chat in this build.');
      return;
    }

    if (activeThreadId && activeThreadId !== selectedThreadId) {
      return;
    }

    const sendingThreadId = selectedThreadId;

    const userMessage: ThreadMessage = {
      id: `msg_${Date.now()}_${Math.random()}`,
      content: trimmed,
      type: 'text',
      extraMetadata: {},
      sender: 'user',
      createdAt: new Date().toISOString(),
    };

    dispatch(addMessageLocal({ threadId: sendingThreadId, message: userMessage }));
    dispatch(setSelectedThread(sendingThreadId));

    const historySnapshot = messages.filter(
      m => !m.id.startsWith('optimistic-') && m.id !== userMessage.id
    );

    setInputValue('');
    setSendError(null);
    setIsSending(true);
    setToolTimelineByThread(prev => ({ ...prev, [sendingThreadId]: [] }));
    dispatch(setActiveThread(sendingThreadId));

    if (!rustChat) {
      setSendError('Chat is only available in the Tauri runtime.');
      setIsSending(false);
      dispatch(setActiveThread(null));
      return;
    }

    // ── Rust path ────────────────────────────────────────────────────────
    try {
      const chatMessages = historySnapshot.map(m => ({
        role: m.sender === 'user' ? 'user' : 'assistant',
        content: m.content,
      }));

      const notionCtx = buildNotionContext(
        notionProfile,
        notionPages,
        notionSummaries,
        notionWorkspaceName
      );

      const authToken = (store.getState() as { auth: { token: string | null } }).auth.token;
      if (!authToken) {
        setSendError('Not authenticated');
        setIsSending(false);
        dispatch(setActiveThread(null));
        return;
      }

      await chatSend({
        threadId: sendingThreadId,
        message: trimmed,
        model: selectedModel,
        authToken,
        backendUrl: BACKEND_URL,
        messages: chatMessages,
        notionContext: notionCtx,
      });

      // setIsSending(false) and setActiveThread(null) happen in the onDone/onError event handlers
    } catch (err) {
      // invoke() itself failed (the chat loop reports errors via events)
      const msg = err instanceof Error ? err.message : String(err);
      setSendError(msg);
      setIsSending(false);
      dispatch(setActiveThread(null));
    }
  };

  const handleInputKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void handleSendMessage();
    }
  };

  const handleCopyMessage = async (messageId: string, content: string) => {
    try {
      await navigator.clipboard.writeText(content);
      setCopiedMessageId(messageId);
      setTimeout(() => setCopiedMessageId(null), 1500);
    } catch {
      // Clipboard API not available — silently fail
    }
  };

  const selectedThread = threads.find(t => t.id === selectedThreadId);

  return (
    <div className="h-full relative z-10 flex overflow-hidden">
      <div className="flex-1 flex flex-col min-w-0">
        <div className="flex items-center gap-3 px-5 py-3 border-b border-white/10">
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <h3 className="text-sm font-semibold truncate">
                {selectedThread?.title || DEFAULT_THREAD_TITLE}
              </h3>
              {selectedThread?.isActive && (
                <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-sage-500/20 text-sage-500 flex-shrink-0">
                  Active
                </span>
              )}
            </div>
            {selectedThread?.createdAt && (
              <p className="text-xs text-stone-500 mt-0.5">
                Created {formatRelativeTime(selectedThread.createdAt)}
              </p>
            )}
          </div>
        </div>

        <div className="flex-1 overflow-y-auto px-5 py-4">
          {isLoadingMessages ? (
            <div className="space-y-4">
              {Array.from({ length: 4 }).map((_, i) => (
                <div key={i} className={`flex ${i % 2 === 0 ? 'justify-start' : 'justify-end'}`}>
                  <div
                    className={`h-12 rounded-2xl animate-pulse bg-white/5 ${
                      i % 2 === 0 ? 'w-2/3' : 'w-1/2'
                    }`}
                  />
                </div>
              ))}
            </div>
          ) : messagesError ? (
            <div className="flex-1 flex flex-col items-center justify-center h-full">
              <svg
                className="w-8 h-8 text-coral-500/70 mb-3"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.5}
                  d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
                />
              </svg>
              <p className="text-sm text-stone-400 mb-1">Failed to load messages</p>
              <p className="text-xs text-stone-600 mb-3 text-center">{messagesError}</p>
              <button
                onClick={() => window.location.reload()}
                className="text-xs text-primary-400 hover:text-primary-300 transition-colors">
                Reload
              </button>
            </div>
          ) : messages.length > 0 ? (
            <div className="space-y-3">
              {messages.map(msg => (
                <div
                  key={msg.id}
                  className={`group/msg flex ${msg.sender === 'user' ? 'justify-end' : 'justify-start'}`}>
                  <div className="relative max-w-[75%]">
                    <div
                      className={`rounded-2xl px-4 py-2.5 ${
                        msg.sender === 'user'
                          ? 'bg-primary-600/20 rounded-br-md'
                          : 'bg-white/5 rounded-bl-md'
                      }`}>
                      {msg.sender === 'agent' ? (
                        <div className="text-sm prose prose-invert prose-sm max-w-none prose-p:my-1 prose-pre:my-2 prose-pre:bg-black/30 prose-pre:rounded-lg prose-code:text-primary-300 prose-code:text-xs prose-a:text-primary-400 prose-headings:text-sm prose-headings:font-semibold prose-ul:my-1 prose-ol:my-1 prose-li:my-0">
                          <Markdown>{msg.content}</Markdown>
                        </div>
                      ) : (
                        <p className="text-sm whitespace-pre-wrap break-words">{msg.content}</p>
                      )}
                      <p
                        className={`text-[10px] mt-1 ${
                          msg.sender === 'user' ? 'text-primary-400/50' : 'text-stone-600'
                        }`}>
                        {formatRelativeTime(msg.createdAt)}
                      </p>
                    </div>
                    <button
                      onClick={() => handleCopyMessage(msg.id, msg.content)}
                      className={`absolute -top-1 ${msg.sender === 'user' ? '-left-8' : '-right-8'} p-1 rounded-md opacity-0 group-hover/msg:opacity-100 hover:bg-white/10 text-stone-600 hover:text-stone-300 transition-all`}
                      title="Copy message">
                      {copiedMessageId === msg.id ? (
                        <svg
                          className="w-3.5 h-3.5 text-sage-500"
                          fill="none"
                          stroke="currentColor"
                          viewBox="0 0 24 24">
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M5 13l4 4L19 7"
                          />
                        </svg>
                      ) : (
                        <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                          <path
                            strokeLinecap="round"
                            strokeLinejoin="round"
                            strokeWidth={2}
                            d="M8 16H6a2 2 0 01-2-2V6a2 2 0 012-2h8a2 2 0 012 2v2m-6 12h8a2 2 0 002-2v-8a2 2 0 00-2-2h-8a2 2 0 00-2 2v8a2 2 0 002 2z"
                          />
                        </svg>
                      )}
                    </button>
                  </div>
                </div>
              ))}
              {activeThreadId === selectedThreadId && isSending && (
                <div className="flex justify-start">
                  <div className="bg-white/5 rounded-2xl rounded-bl-md px-4 py-3">
                    <div className="flex items-center gap-1">
                      <span className="w-1.5 h-1.5 rounded-full bg-stone-500 animate-bounce [animation-delay:0ms]" />
                      <span className="w-1.5 h-1.5 rounded-full bg-stone-500 animate-bounce [animation-delay:150ms]" />
                      <span className="w-1.5 h-1.5 rounded-full bg-stone-500 animate-bounce [animation-delay:300ms]" />
                    </div>
                  </div>
                </div>
              )}
              {selectedThreadToolTimeline.length > 0 && (
                <div className="space-y-1 px-1 py-1">
                  {selectedThreadToolTimeline.map(entry => (
                    <div key={entry.id} className="flex items-center gap-2 text-xs text-stone-400">
                      <span className="font-mono">{entry.name}</span>
                      <span
                        className={`rounded-full px-2 py-0.5 text-[10px] ${
                          entry.status === 'running'
                            ? 'bg-amber-500/20 text-amber-300'
                            : entry.status === 'success'
                              ? 'bg-sage-500/20 text-sage-300'
                              : 'bg-coral-500/20 text-coral-300'
                        }`}>
                        {entry.status}
                      </span>
                    </div>
                  ))}
                </div>
              )}
              {isSending && rustChat && (
                <div className="flex justify-start px-1">
                  <button
                    onClick={() => {
                      if (selectedThreadId) void chatCancel(selectedThreadId);
                    }}
                    className="text-xs text-stone-400 hover:text-stone-200 transition-colors">
                    Cancel
                  </button>
                </div>
              )}
              <div ref={messagesEndRef} />
            </div>
          ) : (
            <div className="flex-1 flex items-center justify-center h-full">
              <p className="text-sm text-stone-600">No messages yet</p>
            </div>
          )}
        </div>

        {messages.length === 0 && suggestedQuestions.length > 0 && !isLoadingSuggestions && (
          <div className="flex-shrink-0 px-4 py-3">
            <div className="flex gap-2 overflow-x-auto scrollbar-hide">
              {suggestedQuestions.map((s, i) => (
                <button
                  key={i}
                  type="button"
                  onClick={() => {
                    void handleSendMessage(s.text);
                  }}
                  disabled={isSending || !rustChat}
                  className="flex-shrink-0 px-3 py-1.5 rounded-lg text-[12px] whitespace-nowrap bg-white/5 text-stone-400 hover:bg-white/10 transition-colors disabled:opacity-50 disabled:cursor-not-allowed">
                  {s.text}
                </button>
              ))}
            </div>
          </div>
        )}

        <div className="flex-shrink-0 border-t border-white/10 px-4 py-3">
          {teamUsage && teamUsage.remainingUsd <= 0 && (
            <div className="mb-3 p-3 rounded-xl bg-coral-500/10 border border-coral-500/20 flex items-center justify-between gap-3">
              <div className="flex items-center gap-2 min-w-0">
                <svg
                  className="w-4 h-4 text-coral-400 flex-shrink-0"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
                  />
                </svg>
                <p className="text-xs text-coral-300 truncate">
                  Daily inference budget exhausted. Top up to continue.
                </p>
              </div>
              <button
                onClick={() => navigate('/settings/billing')}
                className="flex-shrink-0 px-3 py-1.5 rounded-lg bg-coral-500 hover:bg-coral-400 text-white text-xs font-medium transition-colors">
                Top Up
              </button>
            </div>
          )}

          <div className="flex items-center gap-2 mb-2">
            {isLoadingModels ? (
              <span className="text-xs text-stone-600">Loading models…</span>
            ) : (
              <>
                <span className="text-xs text-stone-500">Model</span>
                <select
                  value={selectedModel}
                  onChange={e => setSelectedModel(e.target.value)}
                  disabled={isSending}
                  className="bg-white/5 border border-white/10 rounded-lg px-2 py-1 text-xs text-stone-300 focus:outline-none focus:ring-1 focus:ring-primary-500/50 disabled:opacity-50 cursor-pointer">
                  {availableModels.length > 0 ? (
                    availableModels.map(m => (
                      <option key={m.id} value={m.id} className="bg-stone-900">
                        {m.id}
                      </option>
                    ))
                  ) : (
                    <option value={selectedModel} className="bg-stone-900">
                      {selectedModel}
                    </option>
                  )}
                </select>
              </>
            )}
            <div className="flex-1" />
            {(isLoadingBudget || teamUsage) &&
              (() => {
                const size = 22;
                const r = 9;
                const circ = 2 * Math.PI * r;
                const pct = teamUsage
                  ? Math.min(1, teamUsage.remainingUsd / teamUsage.cycleBudgetUsd)
                  : 0;
                const dash = pct * circ;
                return (
                  <div
                    className="flex items-center gap-1.5"
                    title={
                      teamUsage
                        ? `$${teamUsage.remainingUsd.toFixed(2)} of $${teamUsage.cycleBudgetUsd.toFixed(2)} remaining`
                        : 'Loading budget…'
                    }>
                    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="-rotate-90">
                      <circle
                        cx={size / 2}
                        cy={size / 2}
                        r={r}
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="2.5"
                        className="text-white/10"
                      />
                      {teamUsage ? (
                        <circle
                          cx={size / 2}
                          cy={size / 2}
                          r={r}
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2.5"
                          strokeDasharray={`${dash} ${circ}`}
                          strokeLinecap="round"
                          className={pct < 0.2 ? 'text-amber-500' : 'text-primary-500'}
                          style={{ transition: 'stroke-dasharray 0.3s ease' }}
                        />
                      ) : (
                        <circle
                          cx={size / 2}
                          cy={size / 2}
                          r={r}
                          fill="none"
                          stroke="currentColor"
                          strokeWidth="2.5"
                          strokeDasharray={`${circ * 0.25} ${circ}`}
                          strokeLinecap="round"
                          className="text-stone-600 animate-spin origin-center"
                          style={{ transformOrigin: `${size / 2}px ${size / 2}px` }}
                        />
                      )}
                    </svg>
                    {teamUsage && (
                      <span className="text-[10px] text-stone-500">${teamUsage.remainingUsd.toFixed(2)}</span>
                    )}
                  </div>
                );
              })()}
          </div>

          {sendError && (
            <div className="flex items-center justify-between mb-2">
              <p className="text-xs text-coral-500">{sendError}</p>
              <button
                onClick={() => setSendError(null)}
                className="text-xs text-stone-500 hover:text-stone-300 transition-colors ml-2 flex-shrink-0">
                Dismiss
              </button>
            </div>
          )}

          <div className="flex items-end gap-2">
            <textarea
              value={inputValue}
              onChange={e => setInputValue(e.target.value)}
              onKeyDown={handleInputKeyDown}
              placeholder="Type a message..."
              rows={1}
              disabled={isSending || !rustChat}
              className="flex-1 resize-none bg-white/5 border border-white/10 rounded-xl px-4 py-2.5 text-sm placeholder:text-stone-500 focus:outline-none focus:ring-1 focus:ring-primary-500/50 focus:border-primary-500/50 transition-all max-h-32 disabled:opacity-50 disabled:cursor-not-allowed"
            />
            <button
              onClick={() => {
                void handleSendMessage();
              }}
              disabled={!inputValue.trim() || isSending || !rustChat}
              className="p-2.5 rounded-xl bg-primary-600 hover:bg-primary-500 disabled:opacity-40 disabled:cursor-not-allowed transition-colors flex-shrink-0">
              {isSending ? (
                <svg className="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
                  <circle
                    className="opacity-25"
                    cx="12"
                    cy="12"
                    r="10"
                    stroke="currentColor"
                    strokeWidth="4"
                  />
                  <path
                    className="opacity-75"
                    fill="currentColor"
                    d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
                  />
                </svg>
              ) : (
                <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M5 12h14M12 5l7 7-7 7"
                  />
                </svg>
              )}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default Conversations;
