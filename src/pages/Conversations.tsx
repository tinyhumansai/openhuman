import {
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import Markdown from 'react-markdown';
import { useNavigate, useParams } from 'react-router-dom';

import { injectAll } from '../lib/ai/injector';
import type { Message } from '../lib/ai/providers/interface';
import { skillManager } from '../lib/skills/manager';
import {
  type ChatMessage,
  inferenceApi,
  type ModelInfo,
  type Tool,
} from '../services/api/inferenceApi';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import type { NotionPageSummary, NotionSummary, NotionUserProfile } from '../store/notionSlice';
import {
  addInferenceResponse,
  addMessageLocal,
  clearDeleteStatus,
  clearPurgeStatus,
  clearSelectedThread,
  createThreadLocal,
  deleteThreadLocal,
  fetchSuggestedQuestions,
  purgeThreads,
  setActiveThread,
  setLastViewed,
  setPanelWidth,
  setSelectedThread,
  updateMessagesForThread,
} from '../store/threadSlice';
import type { ThreadMessage } from '../types/thread';

const MIN_PANEL_WIDTH = 200;
const MAX_PANEL_WIDTH = 480;

// ---------------------------------------------------------------------------
// Notion context builder
// ---------------------------------------------------------------------------

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
  const { threadId: urlThreadId } = useParams<{ threadId?: string }>();
  const {
    threads,
    selectedThreadId,
    messages,
    isLoadingMessages,
    messagesError,
    deleteStatus,
    purgeStatus,
    panelWidth,
    lastViewedAt,
    suggestedQuestions,
    isLoadingSuggestions,
    activeThreadId,
  } = useAppSelector(state => state.thread);

  const skillsState = useAppSelector(state => state.skills);
  const notionProfile = useAppSelector(state => state.notion.profile);
  const notionPages = useAppSelector(state => state.notion.pages);
  const notionSummaries = useAppSelector(state => state.notion.summaries);
  const notionWorkspaceName = useAppSelector(
    state =>
      ((state.skills.skillStates?.notion as Record<string, unknown> | undefined)?.workspaceName as
        | string
        | null) ?? null
  );

  const [showPurgeConfirm, setShowPurgeConfirm] = useState(false);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);
  const [inputValue, setInputValue] = useState('');
  const [searchQuery, setSearchQuery] = useState('');
  const [copiedMessageId, setCopiedMessageId] = useState<string | null>(null);

  // Inference model state
  const [availableModels, setAvailableModels] = useState<ModelInfo[]>([]);
  const [selectedModel, setSelectedModel] = useState('neocortex-mk1');
  const [isLoadingModels, setIsLoadingModels] = useState(false);
  const [isSending, setIsSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const isDragging = useRef(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const lastPanelWidthRef = useRef(panelWidth);

  // Filtered threads based on search query (#13)
  const filteredThreads = useMemo(() => {
    if (!searchQuery.trim()) return threads;
    const q = searchQuery.toLowerCase();
    return threads.filter(t => (t.title || 'Untitled Thread').toLowerCase().includes(q));
  }, [threads, searchQuery]);

  // Unread: thread has messages since last view (#15)
  const isThreadUnread = useCallback(
    (thread: { id: string; lastMessageAt?: string | null; createdAt: string }) => {
      const viewed = lastViewedAt[thread.id];
      const lastMsg = new Date(thread.lastMessageAt || thread.createdAt).getTime();
      return viewed == null || lastMsg > viewed;
    },
    [lastViewedAt]
  );

  // Mobile: detect small screens for responsive layout (#12)
  const [isMobile, setIsMobile] = useState(false);
  useEffect(() => {
    const mq = window.matchMedia('(max-width: 767px)');
    setIsMobile(mq.matches);
    const handler = (e: MediaQueryListEvent) => setIsMobile(e.matches);
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, []);

  const handleResizePointerDown = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      e.preventDefault();
      isDragging.current = true;
      const startX = e.clientX;
      const startWidth = panelWidth;

      const onPointerMove = (ev: globalThis.PointerEvent) => {
        const delta = ev.clientX - startX;
        const newWidth = Math.min(MAX_PANEL_WIDTH, Math.max(MIN_PANEL_WIDTH, startWidth + delta));
        lastPanelWidthRef.current = newWidth;
        dispatch(setPanelWidth(newWidth));
      };

      const onPointerUp = () => {
        isDragging.current = false;
        document.removeEventListener('pointermove', onPointerMove);
        document.removeEventListener('pointerup', onPointerUp);
        document.body.style.cursor = '';
        document.body.style.userSelect = '';
      };

      document.addEventListener('pointermove', onPointerMove);
      document.addEventListener('pointerup', onPointerUp);
      document.body.style.cursor = 'col-resize';
      document.body.style.userSelect = 'none';
    },
    [panelWidth, dispatch]
  );

  // Fetch available inference models on mount
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

  // Remove thread fetching - threads are now loaded from Redux persist

  // Sync URL → Redux: when URL has a threadId param, select that thread
  useEffect(() => {
    if (urlThreadId && urlThreadId !== selectedThreadId) {
      dispatch(setSelectedThread(urlThreadId));
    } else if (!urlThreadId && selectedThreadId) {
      dispatch(clearSelectedThread());
    }
  }, [urlThreadId]); // eslint-disable-line react-hooks/exhaustive-deps

  // Mark thread as viewed when selected (#15) — stored in Redux (persisted via redux-persist)
  useEffect(() => {
    if (selectedThreadId) dispatch(setLastViewed(selectedThreadId));
  }, [selectedThreadId, dispatch]);

  // Remove message fetching - messages load from local storage automatically

  // Fetch suggested questions when thread is empty (beginning of new thread)
  useEffect(() => {
    if (selectedThreadId && messages.length === 0) {
      dispatch(fetchSuggestedQuestions(selectedThreadId));
    }
  }, [selectedThreadId, messages.length, dispatch]);

  // Auto-scroll to bottom when messages load
  useEffect(() => {
    if (messages.length > 0) {
      messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [messages]);

  // Remove create status handling - using local thread creation

  useEffect(() => {
    if (deleteStatus === 'success' || deleteStatus === 'error') {
      dispatch(clearDeleteStatus());
    }
  }, [deleteStatus, dispatch]);

  useEffect(() => {
    if (purgeStatus === 'success' || purgeStatus === 'error') {
      dispatch(clearPurgeStatus());
    }
  }, [purgeStatus, dispatch]);

  // Clear send error when user starts typing again
  useEffect(() => {
    if (sendError && inputValue.length > 0) {
      setSendError(null);
    }
  }, [inputValue, sendError]);

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
    const trimmed = text ?? inputValue.trim();
    if (!trimmed || !selectedThreadId || isSending) return;

    // Check if another thread is already sending
    if (activeThreadId && activeThreadId !== selectedThreadId) {
      return; // Block sending from non-active threads
    }

    // Store the original thread ID to ensure response goes to correct thread
    const sendingThreadId = selectedThreadId;

    // Create stable user message and persist immediately
    const userMessage: ThreadMessage = {
      id: `msg_${Date.now()}_${Math.random()}`,
      content: trimmed,
      type: 'text',
      extraMetadata: {},
      sender: 'user',
      createdAt: new Date().toISOString(),
    };

    // Immediately persist user message to both current view and persistent storage
    dispatch(addMessageLocal({ threadId: sendingThreadId, message: userMessage }));

    // Update current view if this is the selected thread
    if (sendingThreadId === selectedThreadId) {
      // Message is already added to persistent storage, reload current view
      dispatch(setSelectedThread(sendingThreadId));
    }

    // Snapshot history for AI request (excluding the just-added user message since we'll add it manually)
    const historySnapshot = messages.filter(m => !m.id.startsWith('optimistic-') && m.id !== userMessage.id);

    setInputValue('');
    setSendError(null);
    setIsSending(true);

    // Set this thread as active
    dispatch(setActiveThread(sendingThreadId));

    try {
      // Process user message with SOUL + TOOLS injection
      let processedUserContent = trimmed;
      try {
        const userMessage: Message = { role: 'user', content: [{ type: 'text', text: trimmed }] };

        const injectedMessage = await injectAll(userMessage, {
          mode: 'context-block',
          includeMetadata: false,
        });

        // Extract the processed text
        processedUserContent = injectedMessage.content
          .filter(block => block.type === 'text')
          .map(block => (block as { text: string }).text)
          .join('\n');

        console.log('✅ SOUL + TOOLS injection successful in Conversations page');
      } catch (injectionError) {
        console.warn('⚠️ SOUL + TOOLS injection failed in Conversations page:', injectionError);
        // Continue with original message
      }

      // Prepend Notion workspace context if connected
      const notionContext = buildNotionContext(
        notionProfile,
        notionPages,
        notionSummaries,
        notionWorkspaceName
      );
      if (notionContext) {
        processedUserContent = `${notionContext}\n\n${processedUserContent}`;
      }

      const chatMessages: ChatMessage[] = [
        ...historySnapshot.map(m => ({
          role: (m.sender === 'user' ? 'user' : 'assistant') as ChatMessage['role'],
          content: m.content,
        })),
        { role: 'user' as const, content: processedUserContent },
      ];

      // Build tool definitions for ALL ready skills — namespaced as {skillId}__{toolName}
      const allSkillTools: Tool[] = Object.entries(skillsState.skills)
        .filter(([, skill]) => skill.status === 'ready' && skill.tools?.length)
        .flatMap(([skillId, skill]) =>
          (skill.tools ?? []).map(t => ({
            type: 'function' as const,
            function: {
              name: `${skillId}__${t.name}`,
              description: t.description,
              parameters: t.inputSchema as Tool['function']['parameters'],
            },
          }))
        );

      console.log(
        `[Conversations] active skill tools: ${allSkillTools.length}`,
        allSkillTools.map(t => t.function.name)
      );

      // Agentic tool calling loop — handles multi-turn tool execution
      const loopMessages = [...chatMessages];
      let finalContent = '';
      const MAX_TOOL_ROUNDS = 5;

      for (let round = 0; round < MAX_TOOL_ROUNDS; round++) {
        const request: Parameters<typeof inferenceApi.createChatCompletion>[0] = {
          model: selectedModel,
          messages: loopMessages,
          ...(allSkillTools.length > 0
            ? { tools: allSkillTools, tool_choice: 'auto' as const }
            : {}),
        };
        console.log('[Conversations] inference request:', {
          round: round + 1,
          model: request.model,
          messageCount: request.messages.length,
          tools: request.tools?.length ?? 0,
          payload: request,
        });
        const response = await inferenceApi.createChatCompletion(request);
        console.log('[Conversations] inference response:', {
          round: round + 1,
          choices: response.choices?.length ?? 0,
          usage: response.usage,
          payload: response,
        });

        const choice = response.choices[0];
        if (!choice) break;

        const { finish_reason, message } = choice;

        if (finish_reason === 'tool_calls' && message.tool_calls?.length) {
          // Append assistant message with tool_calls
          loopMessages.push({
            role: 'assistant',
            content: message.content ?? '',
            tool_calls: message.tool_calls,
          });

          const latestIndex = message.tool_calls.length - 1;
          // API requires a tool message for every tool_call_id; we execute only the latest and send placeholders for the rest
          for (let i = 0; i < message.tool_calls.length; i++) {
            const tc = message.tool_calls[i];
            if (i !== latestIndex) {
              loopMessages.push({ role: 'tool', tool_call_id: tc.id, content: '' });
              continue;
            }

            const dunderIdx = tc.function.name.indexOf('__');
            const skillId = dunderIdx !== -1 ? tc.function.name.substring(0, dunderIdx) : '';
            const toolName =
              dunderIdx !== -1 ? tc.function.name.substring(dunderIdx + 2) : tc.function.name;

            console.log(
              `[Conversations] tool_call dispatched — skill="${skillId}" tool="${toolName}" call_id="${tc.id}"`
            );

            let toolResultContent = '';
            try {
              let toolArgs: Record<string, unknown> = {};
              try {
                toolArgs = JSON.parse(tc.function.arguments) as Record<string, unknown>;
              } catch {
                toolArgs = {};
              }
              console.log(
                `[Conversations] calling skillManager.callTool("${skillId}", "${toolName}")`,
                toolArgs
              );
              const result = await skillManager.callTool(skillId, toolName, toolArgs);
              console.log(`[Conversations] tool "${toolName}" calling result:`, result);
              toolResultContent = result.content.map(c => c.text).join('\n');
              let toolReturnedError = result.isError;
              if (!toolReturnedError && toolResultContent) {
                try {
                  const parsed = JSON.parse(toolResultContent) as Record<string, unknown>;
                  if (parsed && typeof parsed.error === 'string') {
                    toolReturnedError = true;
                    toolResultContent = `Error: ${parsed.error}`;
                  }
                } catch {
                  // not JSON or no error key — keep content as-is
                }
              }
              if (toolReturnedError) {
                console.warn(
                  `[Conversations] tool "${toolName}" returned an error:`,
                  toolResultContent
                );
                if (!toolResultContent.startsWith('Error: ')) {
                  toolResultContent = `Error: ${toolResultContent}`;
                }
              } else {
                console.log(
                  `[Conversations] tool "${toolName}" succeeded:`,
                  toolResultContent.slice(0, 200)
                );
              }
            } catch (toolErr) {
              console.error(`[Conversations] tool "${toolName}" threw:`, toolErr);
              toolResultContent = `Tool execution failed: ${toolErr instanceof Error ? toolErr.message : String(toolErr)}`;
            }

            loopMessages.push({ role: 'tool', tool_call_id: tc.id, content: toolResultContent });
          }
          // Continue loop to get final response after tool results
          continue;
        }

        // Normal (non-tool) response — done
        finalContent = message.content ?? '';
        break;
      }

      // Pass the original sending thread ID to ensure response goes to correct thread
      dispatch(addInferenceResponse({ content: finalContent, threadId: sendingThreadId }));
    } catch (err) {
      // Remove the user message from persistent storage on error
      // We'll use a thunk-like approach to access current state
      dispatch((dispatch, getState) => {
        const state = getState() as { thread: { messagesByThreadId: Record<string, ThreadMessage[]> } };
        const persistedMessages = state.thread.messagesByThreadId[sendingThreadId] || [];
        const currentMessages = persistedMessages.filter(m => m.id !== userMessage.id);
        dispatch(updateMessagesForThread({ threadId: sendingThreadId, messages: currentMessages }));

        // Also remove from current view if this is the selected thread
        if (sendingThreadId === selectedThreadId) {
          dispatch(setSelectedThread(sendingThreadId));
        }
      });

      const msg =
        err && typeof err === 'object' && 'error' in err
          ? String((err as { error: unknown }).error)
          : 'Failed to get response';
      setSendError(msg);
      // Clear active thread on error
      dispatch(setActiveThread(null));
    } finally {
      setIsSending(false);
    }
  };

  const handleInputKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSendMessage();
    }
  };

  // Copy message to clipboard (#10)
  const handleCopyMessage = async (messageId: string, content: string) => {
    try {
      await navigator.clipboard.writeText(content);
      setCopiedMessageId(messageId);
      setTimeout(() => setCopiedMessageId(null), 1500);
    } catch {
      // Clipboard API not available — silently fail
    }
  };

  // Mobile: back to thread list
  const handleMobileBack = () => {
    navigate('/conversations', { replace: true });
  };

  const selectedThread = threads.find(t => t.id === selectedThreadId);

  // Mobile layout: show only one panel at a time (#12)
  const showThreadList = !isMobile || !selectedThreadId;
  const showMessages = !isMobile || !!selectedThreadId;

  return (
    <div className="h-full relative z-10 flex overflow-hidden">
      {/* Left Panel: Thread List */}
      {showThreadList && (
        <div
          className="flex-shrink-0 flex flex-col"
          style={isMobile ? { width: '100%' } : { width: panelWidth }}>
          {/* Header */}
          <div className="flex items-center justify-between px-4 py-3 border-b border-white/10">
            <h2 className="text-sm font-semibold">Conversations</h2>
            <button
              onClick={handleNewThread}
              className="p-1.5 rounded-lg hover:bg-white/10 transition-colors text-stone-400 hover:text-stone-200"
              title="New Thread">
              <svg className="w-4.5 h-4.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M12 4v16m8-8H4"
                />
              </svg>
            </button>
          </div>

          {/* Search bar (#13) */}
          {threads.length > 0 && (
            <div className="px-3 py-2 border-b border-white/10">
              <div className="relative">
                <svg
                  className="absolute left-2.5 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-stone-500"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={2}
                    d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z"
                  />
                </svg>
                <input
                  type="text"
                  value={searchQuery}
                  onChange={e => setSearchQuery(e.target.value)}
                  placeholder="Search threads..."
                  className="w-full bg-white/5 border border-white/10 rounded-lg pl-8 pr-3 py-1.5 text-xs placeholder:text-stone-500 focus:outline-none focus:ring-1 focus:ring-primary-500/50 focus:border-primary-500/50 transition-all"
                />
                {searchQuery && (
                  <button
                    onClick={() => setSearchQuery('')}
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-stone-500 hover:text-stone-300">
                    <svg className="w-3 h-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M6 18L18 6M6 6l12 12"
                      />
                    </svg>
                  </button>
                )}
              </div>
            </div>
          )}

          {/* Thread list */}
          <div className="flex-1 overflow-y-auto">
            {filteredThreads.length > 0 ? (
              <div className="py-1">
                {filteredThreads.map(thread => (
                  <div
                    key={thread.id}
                    className={`group relative transition-colors ${
                      thread.id === selectedThreadId ? 'bg-white/10' : 'hover:bg-white/[0.07]'
                    }`}>
                    {confirmDeleteId === thread.id ? (
                      <div className="flex items-center justify-between py-3 px-4">
                        <span className="text-xs text-stone-400 truncate">Delete this thread?</span>
                        <div className="flex gap-2 flex-shrink-0 ml-2">
                          <button
                            onClick={() => setConfirmDeleteId(null)}
                            className="text-xs text-stone-500 hover:text-stone-300 transition-colors">
                            Cancel
                          </button>
                          <button
                            onClick={() => handleDeleteThread(thread.id)}
                            disabled={deleteStatus === 'loading'}
                            className="text-xs text-coral-500 hover:text-coral-400 transition-colors disabled:opacity-50">
                            {deleteStatus === 'loading' ? 'Deleting...' : 'Delete'}
                          </button>
                        </div>
                      </div>
                    ) : (
                      <>
                        <button
                          onClick={() => handleSelectThread(thread.id)}
                          className="w-full text-left py-3 px-4 cursor-pointer">
                          <div className="flex items-center gap-2 mb-1">
                            <span
                              className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${
                                thread.isActive ? 'bg-sage-500' : 'bg-stone-600'
                              }`}
                            />
                            <span
                              className={`text-sm font-medium truncate ${
                                isThreadUnread(thread) ? 'font-semibold text-stone-200' : ''
                              }`}>
                              {thread.title || 'Untitled Thread'}
                            </span>
                            {isThreadUnread(thread) && (
                              <span
                                className="w-1.5 h-1.5 rounded-full bg-primary-500 flex-shrink-0"
                                title="Unread"
                              />
                            )}
                          </div>
                          <div className="flex items-center justify-between pl-3.5">
                            <span className="text-xs text-stone-500">
                              {thread.messageCount} message{thread.messageCount !== 1 ? 's' : ''}
                            </span>
                            <span className="text-xs text-stone-600">
                              {formatRelativeTime(thread.lastMessageAt || thread.createdAt)}
                            </span>
                          </div>
                        </button>
                        {/* Delete button — visible on hover */}
                        <button
                          onClick={e => {
                            e.stopPropagation();
                            setConfirmDeleteId(thread.id);
                          }}
                          className="absolute right-2 top-1/2 -translate-y-1/2 p-1.5 rounded-lg opacity-0 group-hover:opacity-100 hover:bg-white/10 text-stone-600 hover:text-coral-500 transition-all"
                          title="Delete thread">
                          <svg
                            className="w-3.5 h-3.5"
                            fill="none"
                            stroke="currentColor"
                            viewBox="0 0 24 24">
                            <path
                              strokeLinecap="round"
                              strokeLinejoin="round"
                              strokeWidth={2}
                              d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
                            />
                          </svg>
                        </button>
                      </>
                    )}
                  </div>
                ))}
              </div>
            ) : threads.length > 0 && searchQuery ? (
              <div className="flex-1 flex flex-col items-center justify-center py-16 px-4">
                <p className="text-sm text-stone-500">No matching threads</p>
                <button
                  onClick={() => setSearchQuery('')}
                  className="text-xs text-primary-400 hover:text-primary-300 transition-colors mt-2">
                  Clear search
                </button>
              </div>
            ) : (
              <div className="flex-1 flex flex-col items-center justify-center py-16 px-4">
                <svg
                  className="w-10 h-10 text-stone-600 mb-3"
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={1.5}
                    d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
                  />
                </svg>
                <p className="text-sm text-stone-500 mb-3">No conversations yet</p>
                <button
                  onClick={handleNewThread}
                  className="text-xs text-primary-400 hover:text-primary-300 transition-colors">
                  Start a new conversation
                </button>
              </div>
            )}
          </div>

          {/* Footer: Delete All */}
          {threads.length > 0 && (
            <div className="px-4 py-3 border-t border-white/10">
              {showPurgeConfirm ? (
                <div className="flex items-center justify-between">
                  <span className="text-xs text-stone-400">Delete all threads?</span>
                  <div className="flex gap-2">
                    <button
                      onClick={() => setShowPurgeConfirm(false)}
                      className="text-xs text-stone-500 hover:text-stone-300 transition-colors">
                      Cancel
                    </button>
                    <button
                      onClick={handlePurge}
                      disabled={purgeStatus === 'loading'}
                      className="text-xs text-coral-500 hover:text-coral-400 transition-colors disabled:opacity-50">
                      {purgeStatus === 'loading' ? 'Deleting...' : 'Confirm'}
                    </button>
                  </div>
                </div>
              ) : (
                <button
                  onClick={() => setShowPurgeConfirm(true)}
                  className="text-xs text-coral-500/70 hover:text-coral-500 transition-colors">
                  Delete All Threads
                </button>
              )}
            </div>
          )}
        </div>
      )}

      {/* Resize Handle — desktop only */}
      {!isMobile && (
        <div
          onPointerDown={handleResizePointerDown}
          className="w-1 flex-shrink-0 cursor-col-resize bg-white/10 hover:bg-primary-500/40 active:bg-primary-500/60 transition-colors"
        />
      )}

      {/* Right Panel: Messages */}
      {showMessages && (
        <div className="flex-1 flex flex-col min-w-0">
          {selectedThread ? (
            <>
              {/* Thread header */}
              <div className="flex items-center gap-3 px-5 py-3 border-b border-white/10">
                {/* Mobile back button (#12) */}
                {isMobile && (
                  <button
                    onClick={handleMobileBack}
                    className="p-1 rounded-lg hover:bg-white/10 text-stone-400 hover:text-stone-200 transition-colors flex-shrink-0 -ml-1">
                    <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        strokeWidth={2}
                        d="M15 19l-7-7 7-7"
                      />
                    </svg>
                  </button>
                )}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <h3 className="text-sm font-semibold truncate">
                      {selectedThread.title || 'Untitled Thread'}
                    </h3>
                    {selectedThread.isActive && (
                      <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-sage-500/20 text-sage-500 flex-shrink-0">
                        Active
                      </span>
                    )}
                  </div>
                  <p className="text-xs text-stone-500 mt-0.5">
                    Created {formatRelativeTime(selectedThread.createdAt)}
                  </p>
                </div>
              </div>

              {/* Messages */}
              <div className="flex-1 overflow-y-auto px-5 py-4">
                {isLoadingMessages ? (
                  <div className="space-y-4">
                    {Array.from({ length: 4 }).map((_, i) => (
                      <div
                        key={i}
                        className={`flex ${i % 2 === 0 ? 'justify-start' : 'justify-end'}`}>
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
                              <p className="text-sm whitespace-pre-wrap break-words">
                                {msg.content}
                              </p>
                            )}
                            <p
                              className={`text-[10px] mt-1 ${
                                msg.sender === 'user' ? 'text-primary-400/50' : 'text-stone-600'
                              }`}>
                              {formatRelativeTime(msg.createdAt)}
                            </p>
                          </div>
                          {/* Copy button (#10) */}
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
                              <svg
                                className="w-3.5 h-3.5"
                                fill="none"
                                stroke="currentColor"
                                viewBox="0 0 24 24">
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
                    {/* Typing indicator (#14) - Only show for the active thread */}
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
                    <div ref={messagesEndRef} />
                  </div>
                ) : (
                  <div className="flex-1 flex items-center justify-center h-full">
                    <p className="text-sm text-stone-600">No messages in this thread</p>
                  </div>
                )}
              </div>

              {/* Suggested questions — only at start of new thread (no messages yet); horizontal scroll */}
              {messages.length === 0 && suggestedQuestions.length > 0 && !isLoadingSuggestions && (
                <div className="flex-shrink-0 px-4 py-3">
                  <div className="flex gap-2 overflow-x-auto scrollbar-hide">
                    {suggestedQuestions.map((s, i) => (
                      <button
                        key={i}
                        type="button"
                        onClick={() => handleSendMessage(s.text)}
                        disabled={isSending || !!(activeThreadId && activeThreadId !== selectedThreadId)}
                        className="flex-shrink-0 px-3 py-1.5 rounded-lg text-[12px] whitespace-nowrap bg-white/5 text-stone-400 hover:bg-white/10 transition-colors disabled:opacity-50 disabled:cursor-not-allowed">
                        {s.text}
                      </button>
                    ))}
                  </div>
                </div>
              )}

              {/* Message Input */}
              <div className="flex-shrink-0 border-t border-white/10 px-4 py-3">
                {/* Show warning if another thread is active */}
                {activeThreadId && activeThreadId !== selectedThreadId && (
                  <div className="mb-3 p-2 rounded-lg bg-amber-500/10 border border-amber-500/20">
                    <p className="text-xs text-amber-400">
                      Another conversation is active. Please wait for it to complete before sending messages here.
                    </p>
                  </div>
                )}
                {/* Model selector */}
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
                    placeholder={activeThreadId && activeThreadId !== selectedThreadId ? "Another conversation is active..." : "Type a message..."}
                    rows={1}
                    disabled={!!(activeThreadId && activeThreadId !== selectedThreadId)}
                    className="flex-1 resize-none bg-white/5 border border-white/10 rounded-xl px-4 py-2.5 text-sm placeholder:text-stone-500 focus:outline-none focus:ring-1 focus:ring-primary-500/50 focus:border-primary-500/50 transition-all max-h-32 disabled:opacity-50 disabled:cursor-not-allowed"
                  />
                  <button
                    onClick={() => handleSendMessage()}
                    disabled={!inputValue.trim() || isSending || !!(activeThreadId && activeThreadId !== selectedThreadId)}
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
                      <svg
                        className="w-4 h-4"
                        fill="none"
                        stroke="currentColor"
                        viewBox="0 0 24 24">
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
            </>
          ) : (
            /* Empty state — no thread selected */
            <div className="flex-1 flex flex-col items-center justify-center">
              <svg
                className="w-12 h-12 text-stone-700 mb-3"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.5}
                  d="M8 12h.01M12 12h.01M16 12h.01M21 12c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 20l1.395-3.72C3.512 15.042 3 13.574 3 12c0-4.418 4.03-8 9-8s9 3.582 9 8z"
                />
              </svg>
              <p className="text-sm text-stone-600">Select a conversation</p>
            </div>
          )}
        </div>
      )}
    </div>
  );
};

export default Conversations;
