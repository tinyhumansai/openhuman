import {
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react';

import { useAppDispatch, useAppSelector } from '../store/hooks';
import {
  addOptimisticMessage,
  clearSelectedThread,
  createThread,
  fetchThreadMessages,
  fetchThreads,
  purgeThreads,
  sendMessage,
  setSelectedThread,
} from '../store/threadSlice';

const MIN_PANEL_WIDTH = 200;
const MAX_PANEL_WIDTH = 480;
const DEFAULT_PANEL_WIDTH = 320;

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
  const {
    threads,
    isLoading,
    selectedThreadId,
    messages,
    isLoadingMessages,
    createStatus,
    purgeStatus,
    sendStatus,
    sendError,
  } = useAppSelector(state => state.thread);

  const [showPurgeConfirm, setShowPurgeConfirm] = useState(false);
  const [inputValue, setInputValue] = useState('');
  const [panelWidth, setPanelWidth] = useState(DEFAULT_PANEL_WIDTH);
  const isDragging = useRef(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);

  const handleResizePointerDown = useCallback(
    (e: ReactPointerEvent<HTMLDivElement>) => {
      e.preventDefault();
      isDragging.current = true;
      const startX = e.clientX;
      const startWidth = panelWidth;

      const onPointerMove = (ev: globalThis.PointerEvent) => {
        const delta = ev.clientX - startX;
        const newWidth = Math.min(MAX_PANEL_WIDTH, Math.max(MIN_PANEL_WIDTH, startWidth + delta));
        setPanelWidth(newWidth);
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
    [panelWidth]
  );

  // Fetch threads on mount
  useEffect(() => {
    dispatch(fetchThreads());
  }, [dispatch]);

  // Fetch messages when a thread is selected
  useEffect(() => {
    if (selectedThreadId) {
      dispatch(fetchThreadMessages(selectedThreadId));
    }
  }, [dispatch, selectedThreadId]);

  // Auto-scroll to bottom when messages load
  useEffect(() => {
    if (messages.length > 0) {
      messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [messages]);

  const handleSelectThread = (threadId: string) => {
    if (threadId === selectedThreadId) return;
    dispatch(setSelectedThread(threadId));
  };

  const handleNewThread = () => {
    dispatch(createThread(undefined));
  };

  const handlePurge = async () => {
    const result = await dispatch(purgeThreads());
    if (purgeThreads.fulfilled.match(result)) {
      setShowPurgeConfirm(false);
      dispatch(clearSelectedThread());
    }
  };

  const handleSendMessage = () => {
    const trimmed = inputValue.trim();
    if (!trimmed || !selectedThreadId || sendStatus === 'loading') return;
    dispatch(addOptimisticMessage({ content: trimmed }));
    setInputValue('');
    dispatch(sendMessage({ threadId: selectedThreadId, message: trimmed }));
  };

  const handleInputKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSendMessage();
    }
  };

  const selectedThread = threads.find(t => t.id === selectedThreadId);

  return (
    <div className="h-full relative z-10 flex overflow-hidden">
      {/* Left Panel: Thread List */}
      <div className="flex-shrink-0 flex flex-col" style={{ width: panelWidth }}>
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-white/10">
          <h2 className="text-sm font-semibold">Conversations</h2>
          <button
            onClick={handleNewThread}
            disabled={createStatus === 'loading'}
            className="p-1.5 rounded-lg hover:bg-white/10 transition-colors text-stone-400 hover:text-stone-200 disabled:opacity-50 disabled:cursor-not-allowed"
            title="New Thread">
            {createStatus === 'loading' ? (
              <svg className="w-4.5 h-4.5 animate-spin" fill="none" viewBox="0 0 24 24">
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
              <svg className="w-4.5 h-4.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M12 4v16m8-8H4"
                />
              </svg>
            )}
          </button>
        </div>

        {/* Thread list */}
        <div className="flex-1 overflow-y-auto">
          {isLoading ? (
            <div className="space-y-1 p-2">
              {Array.from({ length: 6 }).map((_, i) => (
                <div key={i} className="h-16 bg-white/5 rounded-xl animate-pulse" />
              ))}
            </div>
          ) : threads.length > 0 ? (
            <div className="py-1">
              {threads.map(thread => (
                <button
                  key={thread.id}
                  onClick={() => handleSelectThread(thread.id)}
                  className={`w-full text-left py-3 px-4 transition-colors cursor-pointer ${
                    thread.id === selectedThreadId ? 'bg-white/10' : 'hover:bg-white/[0.07]'
                  }`}>
                  <div className="flex items-center gap-2 mb-1">
                    <span
                      className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${
                        thread.isActive ? 'bg-sage-500' : 'bg-stone-600'
                      }`}
                    />
                    <span className="text-sm font-medium truncate">
                      {thread.title || 'Untitled Thread'}
                    </span>
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
              ))}
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
                disabled={createStatus === 'loading'}
                className="text-xs text-primary-400 hover:text-primary-300 transition-colors disabled:opacity-50">
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

      {/* Resize Handle */}
      <div
        onPointerDown={handleResizePointerDown}
        className="w-1 flex-shrink-0 cursor-col-resize bg-white/10 hover:bg-primary-500/40 active:bg-primary-500/60 transition-colors"
      />

      {/* Right Panel: Messages */}
      <div className="flex-1 flex flex-col min-w-0">
        {selectedThread ? (
          <>
            {/* Thread header */}
            <div className="flex items-center gap-3 px-5 py-3 border-b border-white/10">
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
              ) : messages.length > 0 ? (
                <div className="space-y-3">
                  {messages.map(msg => (
                    <div
                      key={msg.id}
                      className={`flex ${msg.sender === 'user' ? 'justify-end' : 'justify-start'}`}>
                      <div
                        className={`max-w-[75%] rounded-2xl px-4 py-2.5 ${
                          msg.sender === 'user'
                            ? 'bg-primary-600/20 rounded-br-md'
                            : 'bg-white/5 rounded-bl-md'
                        }`}>
                        <p className="text-sm whitespace-pre-wrap break-words">{msg.content}</p>
                        <p
                          className={`text-[10px] mt-1 ${
                            msg.sender === 'user' ? 'text-primary-400/50' : 'text-stone-600'
                          }`}>
                          {formatRelativeTime(msg.createdAt)}
                        </p>
                      </div>
                    </div>
                  ))}
                  <div ref={messagesEndRef} />
                </div>
              ) : (
                <div className="flex-1 flex items-center justify-center h-full">
                  <p className="text-sm text-stone-600">No messages in this thread</p>
                </div>
              )}
            </div>

            {/* Message Input */}
            <div className="flex-shrink-0 border-t border-white/10 px-4 py-3">
              {sendError && <p className="text-xs text-coral-500 mb-2">{sendError}</p>}
              <div className="flex items-end gap-2">
                <textarea
                  value={inputValue}
                  onChange={e => setInputValue(e.target.value)}
                  onKeyDown={handleInputKeyDown}
                  placeholder="Type a message..."
                  rows={1}
                  className="flex-1 resize-none bg-white/5 border border-white/10 rounded-xl px-4 py-2.5 text-sm placeholder:text-stone-500 focus:outline-none focus:ring-1 focus:ring-primary-500/50 focus:border-primary-500/50 transition-all max-h-32"
                />
                <button
                  onClick={handleSendMessage}
                  disabled={!inputValue.trim() || sendStatus === 'loading'}
                  className="p-2.5 rounded-xl bg-primary-600 hover:bg-primary-500 disabled:opacity-40 disabled:cursor-not-allowed transition-colors flex-shrink-0">
                  {sendStatus === 'loading' ? (
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
    </div>
  );
};

export default Conversations;
