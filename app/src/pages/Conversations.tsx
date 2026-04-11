import { convertFileSrc } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';
import Markdown from 'react-markdown';
import { useNavigate } from 'react-router-dom';

import { type ChatSendError, chatSendError } from '../chat/chatSendError';
import UpsellBanner from '../components/upsell/UpsellBanner';
import { dismissBanner, shouldShowBanner } from '../components/upsell/upsellDismissState';
import UsageLimitModal from '../components/upsell/UsageLimitModal';
import { useUsageState } from '../hooks/useUsageState';
import {
  chatCancel,
  type ChatSegmentEvent,
  chatSend,
  type ChatToolCallEvent,
  type ChatToolResultEvent,
  segmentText,
  subscribeChatEvents,
  useRustChat,
} from '../services/chatService';
import { store } from '../store';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { selectSocketStatus } from '../store/socketSelectors';
import {
  addInferenceResponse,
  addMessageLocal,
  createThreadLocal,
  fetchSuggestedQuestions,
  loadThreadMessages,
  loadThreads,
  persistReaction,
  setActiveThread,
  setLastViewed,
  setSelectedThread,
} from '../store/threadSlice';
import type { ThreadMessage } from '../types/thread';
import {
  isTauri,
  notifyOverlaySttState,
  openhumanAutocompleteAccept,
  openhumanAutocompleteCurrent,
  openhumanLocalAiAnalyzeSentiment,
  openhumanLocalAiShouldSendGif,
  openhumanLocalAiTenorSearch,
  openhumanVoiceStatus,
  openhumanVoiceTranscribeBytes,
  openhumanVoiceTts,
} from '../utils/tauriCommands';

const DEFAULT_THREAD_ID = 'default-thread';
const DEFAULT_THREAD_TITLE = 'Conversation';
const AGENTIC_MODEL_ID = 'agentic-v1';
type ToolTimelineEntryStatus = 'running' | 'success' | 'error';
type InputMode = 'text' | 'voice';
type ReplyMode = 'text' | 'voice';
const AUTOCOMPLETE_POLL_DEBOUNCE_MS = 320;
const AUTOCOMPLETE_MIN_CONTEXT_CHARS = 3;

interface ToolTimelineEntry {
  id: string;
  name: string;
  round: number;
  status: ToolTimelineEntryStatus;
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

function getInlineCompletionSuffix(input: string, suggestion: string): string {
  if (!input || !suggestion) return '';
  const normalize = (value: string) =>
    value
      .replace(/\u2192/g, ' ')
      .replace(/\s+/g, ' ')
      .trim();

  const normalizedInput = normalize(input);
  const normalizedSuggestion = normalize(suggestion);
  if (!normalizedSuggestion) return '';

  // Full-text response: strip already-typed prefix.
  if (normalizedSuggestion.startsWith(normalizedInput)) {
    return normalizedSuggestion.slice(normalizedInput.length).trimStart();
  }

  // Remove overlap to prevent duplicate phrase insertion:
  // "...want to" + "want to create..." => "create..."
  const maxOverlap = Math.min(normalizedInput.length, normalizedSuggestion.length, 120);
  for (let overlap = maxOverlap; overlap >= 1; overlap -= 1) {
    if (
      normalizedInput.slice(normalizedInput.length - overlap) ===
      normalizedSuggestion.slice(0, overlap)
    ) {
      return normalizedSuggestion.slice(overlap).trimStart();
    }
  }

  // Suffix-only fallback (the backend is intended to return suffix text).
  if (normalizedInput.endsWith(normalizedSuggestion)) {
    return '';
  }
  return normalizedSuggestion;
}

function buildAcceptedInlineCompletion(input: string, suffix: string): string {
  const normalizedInput = input.replace(/\u2192/g, ' ').replace(/\t+/g, ' ');
  const cleanSuffix = suffix
    .replace(/\u2192/g, ' ')
    .replace(/\t+/g, ' ')
    .replace(/\s+/g, ' ')
    .trim();

  if (!cleanSuffix) return normalizedInput;

  const needsSpace =
    normalizedInput.length > 0 && !/\s$/.test(normalizedInput) && !/^[,.;:!?)]/.test(cleanSuffix);

  return `${normalizedInput}${needsSpace ? ' ' : ''}${cleanSuffix}`;
}

function formatResetTime(isoStr: string): string {
  const ms = new Date(isoStr).getTime() - Date.now();
  if (ms <= 0) return 'now';
  const mins = Math.ceil(ms / 60_000);
  if (mins < 60) return `in ${mins}m`;
  const hours = Math.floor(mins / 60);
  const remMins = mins % 60;
  if (hours < 24) return remMins > 0 ? `in ${hours}h ${remMins}m` : `in ${hours}h`;
  const days = Math.floor(hours / 24);
  const remHours = hours % 24;
  return remHours > 0 ? `in ${days}d ${remHours}h` : `in ${days}d`;
}

function LimitPill({ label, usedPct }: { label: string; usedPct: number }) {
  const barColor =
    usedPct >= 1 ? 'bg-coral-500' : usedPct >= 0.8 ? 'bg-amber-500' : 'bg-primary-500';
  return (
    <div className="flex items-center gap-1">
      <span className="text-[9px] text-stone-400 font-medium uppercase">{label}</span>
      <div className="w-8 h-1.5 rounded-full bg-stone-200 overflow-hidden">
        <div
          className={`h-full rounded-full transition-all duration-300 ${barColor}`}
          style={{ width: `${Math.min(100, usedPct * 100)}%` }}
        />
      </div>
      <span className="text-[9px] text-stone-500 tabular-nums">{Math.round(usedPct * 100)}%</span>
    </div>
  );
}

const Conversations = () => {
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const {
    selectedThreadId,
    messages,
    isLoadingMessages,
    messagesError,
    suggestedQuestions,
    isLoadingSuggestions,
    activeThreadId,
  } = useAppSelector(state => state.thread);

  const [inputValue, setInputValue] = useState('');
  const [copiedMessageId, setCopiedMessageId] = useState<string | null>(null);
  const [inputMode, setInputMode] = useState<InputMode>('text');
  const [replyMode, setReplyMode] = useState<ReplyMode>('text');
  const [isRecording, setIsRecording] = useState(false);
  const [isTranscribing, setIsTranscribing] = useState(false);
  const [voiceStatus, setVoiceStatus] = useState<string | null>(null);
  const [isPlayingReply, setIsPlayingReply] = useState(false);
  const [inlineSuggestionValue, setInlineSuggestionValue] = useState('');

  const [isSending, setIsSending] = useState(false);
  const [sendError, setSendError] = useState<ChatSendError | null>(null);
  const socketStatus = useAppSelector(selectSocketStatus);
  const [toolTimelineByThread, setToolTimelineByThread] = useState<
    Record<string, ToolTimelineEntry[]>
  >({});
  const rustChat = useRustChat();
  const defaultChannelType = useAppSelector(
    state => state.channelConnections?.defaultMessagingChannel ?? 'web'
  );
  const [reactionPickerMsgId, setReactionPickerMsgId] = useState<string | null>(null);
  const pendingReactionRef = useRef<
    Map<string, { msgId: string; content: string; threadId: string }>
  >(new Map());

  /** Message counter for GIF cadence — check every ~7 messages. */
  const gifCadenceCountRef = useRef(0);
  const GIF_CADENCE_MESSAGES = 7;
  /** Timestamp (ms) of last sentiment analysis — run roughly every hour. */
  const lastSentimentAtRef = useRef(0);
  const SENTIMENT_INTERVAL_MS = 60 * 60 * 1000; // 1 hour

  const selectedThreadIdRef = useRef(selectedThreadId);
  useEffect(() => {
    selectedThreadIdRef.current = selectedThreadId;
  }, [selectedThreadId]);

  const {
    teamUsage,
    isLoading: isLoadingBudget,
    isAtLimit,
    isBudgetExhausted,
    isNearLimit,
    isFreeTier,
    usagePct10h,
    usagePct7d,
    currentTier,
  } = useUsageState();
  const [showLimitModal, setShowLimitModal] = useState(false);

  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textInputRef = useRef<HTMLTextAreaElement>(null);
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const mediaStreamRef = useRef<MediaStream | null>(null);
  const audioChunksRef = useRef<Blob[]>([]);
  const replyAudioRef = useRef<HTMLAudioElement | null>(null);
  const lastSpokenMessageIdRef = useRef<string | null>(null);
  const autocompleteDebounceRef = useRef<number | null>(null);
  const autocompleteRequestSeqRef = useRef(0);
  const sendingTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const seenChatEventsRef = useRef<Map<string, number>>(new Map());

  const markChatEventSeen = (key: string): boolean => {
    const now = Date.now();
    const cache = seenChatEventsRef.current;
    const ttlMs = 10 * 60_000;
    const maxEntries = 500;

    if (cache.has(key)) return false;

    cache.set(key, now);

    // Prune old entries first.
    for (const [existingKey, timestamp] of cache) {
      if (now - timestamp > ttlMs) {
        cache.delete(existingKey);
      }
    }

    // Keep bounded memory in long sessions.
    while (cache.size > maxEntries) {
      const oldest = cache.keys().next().value;
      if (!oldest) break;
      cache.delete(oldest);
    }

    return true;
  };

  const getAudioExtension = (mimeType: string): string => {
    const lower = mimeType.toLowerCase();
    if (lower.includes('webm')) return 'webm';
    if (lower.includes('ogg')) return 'ogg';
    if (lower.includes('wav')) return 'wav';
    if (lower.includes('mp4') || lower.includes('mpeg') || lower.includes('aac')) return 'm4a';
    return 'webm';
  };
  const canUseMicrophoneApi =
    typeof navigator !== 'undefined' &&
    typeof navigator.mediaDevices !== 'undefined' &&
    typeof navigator.mediaDevices.getUserMedia === 'function';

  useEffect(() => {
    void dispatch(loadThreads());
    void dispatch(
      createThreadLocal({
        id: DEFAULT_THREAD_ID,
        title: DEFAULT_THREAD_TITLE,
        createdAt: new Date().toISOString(),
      })
    ).then(() => {
      dispatch(setSelectedThread(DEFAULT_THREAD_ID));
      void dispatch(loadThreadMessages(DEFAULT_THREAD_ID));
    });
  }, [dispatch]);

  useEffect(() => {
    if (selectedThreadId) dispatch(setLastViewed(selectedThreadId));
  }, [selectedThreadId, dispatch]);

  useEffect(() => {
    if (selectedThreadId) {
      void dispatch(loadThreadMessages(selectedThreadId));
    }
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
    const onDictationInsert = (event: Event) => {
      const customEvent = event as CustomEvent<{ text?: string }>;
      const text = customEvent.detail?.text?.trim();
      if (!text) return;

      customEvent.preventDefault();
      setInputMode('text');
      setInputValue(prev => {
        const base = prev.trim();
        if (!base) return text;
        return `${base}${base.endsWith(' ') ? '' : ' '}${text}`;
      });

      window.requestAnimationFrame(() => {
        textInputRef.current?.focus();
      });
    };

    window.addEventListener('dictation://insert-text', onDictationInsert as EventListener);
    return () =>
      window.removeEventListener('dictation://insert-text', onDictationInsert as EventListener);
  }, []);

  useEffect(() => {
    if (sendError && inputValue.length > 0) {
      setSendError(null);
    }
  }, [inputValue, sendError]);

  useEffect(() => {
    if (
      !isTauri() ||
      !rustChat ||
      inputMode !== 'text' ||
      isSending ||
      inputValue.trim().length < AUTOCOMPLETE_MIN_CONTEXT_CHARS
    ) {
      setInlineSuggestionValue('');
      return;
    }

    if (autocompleteDebounceRef.current !== null) {
      window.clearTimeout(autocompleteDebounceRef.current);
    }

    autocompleteDebounceRef.current = window.setTimeout(() => {
      const requestSeq = autocompleteRequestSeqRef.current + 1;
      autocompleteRequestSeqRef.current = requestSeq;

      void openhumanAutocompleteCurrent({ context: inputValue })
        .then(response => {
          if (autocompleteRequestSeqRef.current !== requestSeq) return;
          setInlineSuggestionValue(response.result.suggestion?.value ?? '');
        })
        .catch(() => {
          if (autocompleteRequestSeqRef.current !== requestSeq) return;
          setInlineSuggestionValue('');
        });
    }, AUTOCOMPLETE_POLL_DEBOUNCE_MS);

    return () => {
      if (autocompleteDebounceRef.current !== null) {
        window.clearTimeout(autocompleteDebounceRef.current);
        autocompleteDebounceRef.current = null;
      }
    };
  }, [inputValue, inputMode, isSending, rustChat]);

  useEffect(() => {
    return () => {
      mediaRecorderRef.current?.stop();
      mediaStreamRef.current?.getTracks().forEach(track => track.stop());
      replyAudioRef.current?.pause();
      replyAudioRef.current = null;
    };
  }, []);

  useEffect(() => {
    if (inputMode === 'text' && isRecording) {
      mediaRecorderRef.current?.stop();
    }
  }, [inputMode, isRecording]);

  useEffect(() => {
    if (inputMode === 'voice') {
      setReplyMode('voice');
    } else if (replyMode === 'voice') {
      setReplyMode('text');
    }
  }, [inputMode, replyMode]);

  // Proactively check voice binary availability when switching to voice mode
  useEffect(() => {
    if (inputMode !== 'voice' || !rustChat) return;
    let cancelled = false;
    void (async () => {
      try {
        const status = await openhumanVoiceStatus();
        if (cancelled) return;
        if (!status.stt_available) {
          setVoiceStatus(
            'Speech-to-text unavailable: whisper-cli binary or STT model not found. Check Settings > Local Models.'
          );
        } else {
          setVoiceStatus('Ready — tap "Start Talking" to record.');
        }
      } catch {
        if (!cancelled) {
          setVoiceStatus('Could not check voice availability.');
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [inputMode, rustChat]);

  useEffect(() => {
    if (!rustChat || socketStatus !== 'connected') return;

    const cleanup = subscribeChatEvents({
      onToolCall: (event: ChatToolCallEvent) => {
        const eventKey = `tool_call:${event.thread_id}:${event.request_id ?? 'none'}:${event.round}:${event.tool_name}`;
        if (!markChatEventSeen(eventKey)) return;

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
        const eventKey = `tool_result:${event.thread_id}:${event.request_id ?? 'none'}:${event.round}:${event.tool_name}:${event.success}`;
        if (!markChatEventSeen(eventKey)) return;

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
              nextEntries[i] = { ...entry, status: event.success ? 'success' : 'error' };
              changed = true;
              break;
            }
          }

          if (!changed) return prev;
          return { ...prev, [event.thread_id]: nextEntries };
        });
      },
      onSegment: (event: ChatSegmentEvent) => {
        const eventKey = `segment:${event.thread_id}:${event.request_id}:${event.segment_index}`;
        if (!markChatEventSeen(eventKey)) return;

        // Rust delivers segments with delays already applied — just dispatch.
        if (event.reaction_emoji) {
          const pending = pendingReactionRef.current.get(event.thread_id);
          if (pending) {
            void dispatch(
              persistReaction({
                threadId: pending.threadId,
                messageId: pending.msgId,
                emoji: event.reaction_emoji,
              })
            );
            pendingReactionRef.current.delete(event.thread_id);
          }
        }
        dispatch(addInferenceResponse({ content: segmentText(event), threadId: event.thread_id }));
      },
      onDone: event => {
        const eventKey = `done:${event.thread_id}:${event.request_id ?? 'none'}`;
        if (!markChatEventSeen(eventKey)) return;

        // Update tool timeline
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
        if (sendingTimeoutRef.current) {
          clearTimeout(sendingTimeoutRef.current);
          sendingTimeoutRef.current = null;
        }

        // Apply reaction emoji from Rust (when not segmented — no onSegment fired).
        if (event.reaction_emoji) {
          const pending = pendingReactionRef.current.get(event.thread_id);
          if (pending) {
            void dispatch(
              persistReaction({
                threadId: pending.threadId,
                messageId: pending.msgId,
                emoji: event.reaction_emoji,
              })
            );
          }
        }

        // Fire-and-forget: GIF decision + sentiment analysis (cadence-based)
        const pendingMsg = pendingReactionRef.current.get(event.thread_id);
        if (pendingMsg) {
          maybeCheckGif(pendingMsg.content, pendingMsg.threadId);
          maybeSentimentAnalysis(pendingMsg.content);
        }
        pendingReactionRef.current.delete(event.thread_id);

        // Only add the response bubble if Rust didn't already deliver it
        // via chat_segment events (segment_total > 0 means segments were sent).
        if (!event.segment_total) {
          dispatch(
            addInferenceResponse({ content: event.full_response, threadId: event.thread_id })
          );
        }

        setIsSending(false);
        dispatch(setActiveThread(null));
      },
      onError: event => {
        const eventKey = `error:${event.thread_id}:${event.request_id ?? 'none'}:${event.error_type}:${event.message}`;
        if (!markChatEventSeen(eventKey)) return;

        if (event.thread_id !== selectedThreadIdRef.current) return;
        if (sendingTimeoutRef.current) {
          clearTimeout(sendingTimeoutRef.current);
          sendingTimeoutRef.current = null;
        }
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

        // Clear pending reaction so stale callbacks are ignored
        pendingReactionRef.current.delete(event.thread_id);

        if (event.error_type !== 'cancelled') {
          // Deduplicate: skip if the last message is already an error
          const currentState = store.getState() as {
            thread: { messagesByThreadId: Record<string, ThreadMessage[]> };
          };
          const threadMessages = currentState.thread.messagesByThreadId[event.thread_id] || [];
          const lastMsg = threadMessages[threadMessages.length - 1];
          if (
            lastMsg?.sender === 'agent' &&
            lastMsg?.content === 'Something went wrong — please try again.'
          ) {
            return;
          }

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
    });

    return cleanup;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rustChat, socketStatus]);

  /**
   * Fire-and-forget: periodically check if a GIF response is appropriate
   * (every ~GIF_CADENCE_MESSAGES messages). If the model says yes, search
   * Tenor and dispatch the top result as a gif-type message.
   */
  const maybeCheckGif = (messageContent: string, threadId: string) => {
    if (!isTauri()) return;

    gifCadenceCountRef.current += 1;
    if (gifCadenceCountRef.current < GIF_CADENCE_MESSAGES) return;
    gifCadenceCountRef.current = 0;

    console.debug('[conversations:gif] cadence reached, evaluating gif decision');

    void openhumanLocalAiShouldSendGif(messageContent, defaultChannelType)
      .then(async response => {
        const decision = response.result;
        if (!decision?.should_send_gif || !decision.search_query) return;

        console.debug('[conversations:gif] searching tenor for:', decision.search_query);
        const tenorResponse = await openhumanLocalAiTenorSearch(decision.search_query, 5);
        const results = tenorResponse.result?.results;
        if (!results || results.length === 0) return;

        // Pick a random GIF from top results
        const picked = results[Math.floor(Math.random() * Math.min(results.length, 3))];
        const gifUrl =
          picked.media?.mediumgif?.url || picked.media?.gif?.url || picked.media?.tinygif?.url;
        if (!gifUrl) return;

        console.debug('[conversations:gif] sending gif:', picked.title || picked.id);
        void dispatch(addInferenceResponse({ content: gifUrl, threadId, type: 'gif' }));
      })
      .catch(err => {
        console.debug('[conversations:gif] failed:', err);
      });
  };

  /**
   * Fire-and-forget: periodically analyze user sentiment (~every hour).
   * Stores the result in debug logs for now.
   */
  const maybeSentimentAnalysis = (messageContent: string) => {
    if (!isTauri()) return;

    const now = Date.now();
    if (now - lastSentimentAtRef.current < SENTIMENT_INTERVAL_MS) return;
    lastSentimentAtRef.current = now;

    console.debug('[conversations:sentiment] interval reached, analyzing sentiment');

    void openhumanLocalAiAnalyzeSentiment(messageContent)
      .then(response => {
        const sentiment = response.result;
        if (!sentiment) return;
        console.debug(
          '[conversations:sentiment] result:',
          sentiment.emotion,
          sentiment.valence,
          `(${sentiment.confidence})`
        );
      })
      .catch(err => {
        console.debug('[conversations:sentiment] failed:', err);
      });
  };

  const handleSendMessage = async (text?: string) => {
    const normalized = text ?? inputValue;
    const trimmed = normalized.trim();

    if (!trimmed || !selectedThreadId || isSending) return;
    if (isAtLimit) {
      setShowLimitModal(true);
      setSendError(
        chatSendError('usage_limit_reached', 'Usage limit reached. Upgrade or wait for reset.')
      );
      return;
    }
    if (socketStatus !== 'connected') {
      setSendError(
        chatSendError(
          'socket_disconnected',
          'Realtime socket is not connected — responses cannot be delivered without a client ID.'
        )
      );
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

    void dispatch(addMessageLocal({ threadId: sendingThreadId, message: userMessage }));
    pendingReactionRef.current.set(sendingThreadId, {
      msgId: userMessage.id,
      content: trimmed,
      threadId: sendingThreadId,
    });

    setInputValue('');
    setSendError(null);
    setIsSending(true);
    // Safety: auto-clear isSending if no response arrives within 120s
    if (sendingTimeoutRef.current) clearTimeout(sendingTimeoutRef.current);
    sendingTimeoutRef.current = setTimeout(() => {
      console.warn('[chat] safety timeout: clearing isSending after 120s with no response');
      setIsSending(false);
      setSendError(
        chatSendError(
          'safety_timeout',
          'No response from the assistant after 2 minutes. Try again or check your connection.'
        )
      );
      dispatch(setActiveThread(null));
      sendingTimeoutRef.current = null;
    }, 120_000);
    setToolTimelineByThread(prev => ({ ...prev, [sendingThreadId]: [] }));
    dispatch(setActiveThread(sendingThreadId));

    // ── Cloud socket path ─────────────────────────────────────────────────────
    // Always route primary chat through the cloud backend via socket.
    // Local model (Ollama) is used only for supplementary features
    // (auto-react, autocomplete, etc.) — never as a primary chat path.
    try {
      await chatSend({ threadId: sendingThreadId, message: trimmed, model: AGENTIC_MODEL_ID });

      // setIsSending(false) and setActiveThread(null) happen in the onDone/onError event handlers
    } catch (err) {
      // Chat loop errors are emitted via socket events; this catch handles emit-level failures.
      if (sendingTimeoutRef.current) {
        clearTimeout(sendingTimeoutRef.current);
        sendingTimeoutRef.current = null;
      }
      const msg = err instanceof Error ? err.message : String(err);
      setSendError(chatSendError('cloud_send_failed', msg));
      setIsSending(false);
      dispatch(setActiveThread(null));
    }
  };

  const transcribeAndSendAudio = async (mimeType: string) => {
    setIsRecording(false);
    mediaRecorderRef.current = null;
    mediaStreamRef.current?.getTracks().forEach(track => track.stop());
    mediaStreamRef.current = null;

    const chunks = audioChunksRef.current;
    audioChunksRef.current = [];
    if (chunks.length === 0) {
      notifyOverlaySttState('cancelled');
      setVoiceStatus('No audio captured. Try again.');
      return;
    }

    setIsTranscribing(true);
    setVoiceStatus('Transcribing with Whisper…');
    try {
      const blob = new Blob(chunks, { type: mimeType || 'audio/webm' });
      const audioBytes = Array.from(new Uint8Array(await blob.arrayBuffer()));
      const extension = getAudioExtension(mimeType || blob.type);

      // Build conversation context from recent messages for LLM cleanup.
      const recentMessages = messages.slice(-10);
      const context =
        recentMessages.length > 0
          ? recentMessages.map(m => `${m.sender}: ${m.content}`).join('\n')
          : undefined;

      const result = await openhumanVoiceTranscribeBytes(audioBytes, extension, context);
      const transcript = result.text.trim();

      if (!transcript) {
        notifyOverlaySttState('cancelled');
        setVoiceStatus('No speech detected. Try again.');
        return;
      }

      notifyOverlaySttState('transcription_done', transcript);
      setVoiceStatus(`Heard: ${transcript}`);
      await handleSendMessage(transcript);
    } catch (err) {
      notifyOverlaySttState('error');
      const message = err instanceof Error ? err.message : String(err);
      setSendError(chatSendError('voice_transcription', `Voice transcription failed: ${message}`));
      setVoiceStatus(null);
    } finally {
      setIsTranscribing(false);
    }
  };

  const handleVoiceRecordToggle = async () => {
    if (!rustChat || isSending || isTranscribing) return;
    if (!canUseMicrophoneApi) {
      setSendError(
        chatSendError(
          'microphone_unavailable',
          'Microphone capture is unavailable in this runtime. Use Text mode, or run the desktop app bundle with microphone permissions enabled.'
        )
      );
      return;
    }

    if (isRecording) {
      mediaRecorderRef.current?.stop();
      return;
    }

    try {
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
      mediaStreamRef.current = stream;

      const preferredTypes = [
        'audio/webm;codecs=opus',
        'audio/webm',
        'audio/ogg;codecs=opus',
        'audio/ogg',
        'audio/mp4',
      ];
      const supportedType = preferredTypes.find(type => MediaRecorder.isTypeSupported(type));
      const recorder = supportedType
        ? new MediaRecorder(stream, { mimeType: supportedType })
        : new MediaRecorder(stream);

      audioChunksRef.current = [];
      recorder.ondataavailable = event => {
        if (event.data.size > 0) {
          audioChunksRef.current.push(event.data);
        }
      };
      recorder.onerror = () => {
        notifyOverlaySttState('error');
        setIsRecording(false);
        mediaStreamRef.current?.getTracks().forEach(track => track.stop());
        mediaStreamRef.current = null;
        setSendError(chatSendError('microphone_recording', 'Microphone recording failed.'));
      };
      recorder.onstop = () => {
        void transcribeAndSendAudio(recorder.mimeType);
      };

      mediaRecorderRef.current = recorder;
      setVoiceStatus('Listening… click Stop to send.');
      setSendError(null);
      setIsRecording(true);
      recorder.start();
      notifyOverlaySttState('recording_started');
    } catch (err) {
      notifyOverlaySttState('error');
      const message = err instanceof Error ? err.message : String(err);
      setSendError(chatSendError('microphone_access', `Microphone access failed: ${message}`));
      setVoiceStatus(null);
    }
  };

  useEffect(() => {
    const latestAgentMessage = [...messages].reverse().find(m => m.sender === 'agent');
    if (!latestAgentMessage) return;

    if (replyMode === 'text') {
      lastSpokenMessageIdRef.current = latestAgentMessage.id;
      replyAudioRef.current?.pause();
      replyAudioRef.current = null;
      setIsPlayingReply(false);
      return;
    }

    if (!rustChat || latestAgentMessage.id === lastSpokenMessageIdRef.current) return;

    lastSpokenMessageIdRef.current = latestAgentMessage.id;
    let cancelled = false;
    setIsPlayingReply(true);

    void (async () => {
      try {
        const ttsResult = await openhumanVoiceTts(latestAgentMessage.content);
        if (cancelled) return;

        const audioSrc = convertFileSrc(ttsResult.output_path);
        const audio = new window.Audio(audioSrc);
        replyAudioRef.current?.pause();
        replyAudioRef.current = audio;

        await audio.play();
      } catch {
        if (!cancelled) {
          setSendError(chatSendError('voice_playback', 'Failed to play voice reply.'));
        }
      } finally {
        if (!cancelled) {
          setIsPlayingReply(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [messages, replyMode, rustChat]);

  const handleInputKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    const inlineSuffix = getInlineCompletionSuffix(inputValue, inlineSuggestionValue);
    const textarea = e.currentTarget;
    const caretAtEnd =
      textarea.selectionStart === inputValue.length && textarea.selectionEnd === inputValue.length;
    const tryAcceptInlineSuggestion = () => {
      const nextValue = buildAcceptedInlineCompletion(inputValue, inlineSuffix);
      if (!nextValue || nextValue === inputValue) return false;
      setInputValue(nextValue);
      setInlineSuggestionValue('');
      if (isTauri()) {
        void openhumanAutocompleteAccept({ suggestion: nextValue, skip_apply: true }).catch(() => {
          // Keep local UX smooth even if accept RPC fails.
        });
      }
      return true;
    };

    if (
      e.key === 'Tab' &&
      !e.shiftKey &&
      !e.altKey &&
      !e.ctrlKey &&
      !e.metaKey &&
      inlineSuffix.length > 0 &&
      caretAtEnd
    ) {
      e.preventDefault();
      tryAcceptInlineSuggestion();
      return;
    }

    if (e.key === 'ArrowRight' && inlineSuffix.length > 0 && caretAtEnd) {
      e.preventDefault();
      tryAcceptInlineSuggestion();
      return;
    }

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

  const selectedThreadToolTimeline = selectedThreadId
    ? (toolTimelineByThread[selectedThreadId] ?? [])
    : [];
  const inlineCompletionSuffix = getInlineCompletionSuffix(inputValue, inlineSuggestionValue);

  return (
    <div className="h-full relative z-10 flex justify-center overflow-hidden p-4 pt-6">
      <div className="flex-1 flex flex-col min-w-0 max-w-2xl bg-white rounded-2xl shadow-soft border border-stone-200 overflow-hidden">
        <div className="flex-1 overflow-y-auto px-5 py-4 bg-stone-50">
          {isLoadingMessages ? (
            <div className="space-y-4">
              {Array.from({ length: 4 }).map((_, i) => (
                <div key={i} className={`flex ${i % 2 === 0 ? 'justify-start' : 'justify-end'}`}>
                  <div
                    className={`h-12 rounded-2xl animate-pulse bg-stone-100 ${
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
                          ? 'bg-primary-500 text-white rounded-br-md'
                          : 'bg-stone-200/80 text-stone-900 rounded-bl-md'
                      }`}>
                      {msg.sender === 'agent' ? (
                        <div className="text-sm prose prose-sm max-w-none prose-p:my-1 prose-pre:my-2 prose-pre:bg-stone-300/50 prose-pre:rounded-lg prose-code:text-primary-700 prose-code:text-xs prose-a:text-primary-500 prose-headings:text-sm prose-headings:font-semibold prose-ul:my-1 prose-ol:my-1 prose-li:my-0">
                          <Markdown>{msg.content}</Markdown>
                        </div>
                      ) : (
                        <p className="text-sm whitespace-pre-wrap break-words">{msg.content}</p>
                      )}
                      <p
                        className={`text-[10px] mt-1 ${
                          msg.sender === 'user' ? 'text-white/60' : 'text-stone-400'
                        }`}>
                        {formatRelativeTime(msg.createdAt)}
                      </p>
                    </div>
                    <button
                      onClick={() => handleCopyMessage(msg.id, msg.content)}
                      className={`absolute -top-1 ${msg.sender === 'user' ? '-left-8' : '-right-8'} p-1 rounded-md opacity-0 group-hover/msg:opacity-100 hover:bg-stone-100 text-stone-400 hover:text-stone-600 transition-all`}
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
                    {(() => {
                      const myReactions =
                        (msg.extraMetadata?.myReactions as string[] | undefined) ?? [];
                      const hasReactions = myReactions.length > 0;
                      // Show reaction row if there are existing reactions (any sender)
                      // or if this is an agent message (manual picker available)
                      if (!hasReactions && msg.sender !== 'agent') return null;
                      return (
                        <div className="mt-1 flex items-center gap-1 flex-wrap min-h-[20px]">
                          {myReactions.map(emoji => (
                            <button
                              key={emoji}
                              onClick={() =>
                                selectedThreadId &&
                                void dispatch(
                                  persistReaction({
                                    threadId: selectedThreadId,
                                    messageId: msg.id,
                                    emoji,
                                  })
                                )
                              }
                              className="flex items-center gap-0.5 px-1.5 py-0.5 rounded-full bg-primary-100 border border-primary-200 text-xs transition-colors hover:bg-primary-200"
                              title={`Remove ${emoji}`}>
                              {emoji}
                            </button>
                          ))}
                          {msg.sender === 'agent' &&
                            (reactionPickerMsgId === msg.id ? (
                              <div className="flex items-center gap-0.5 px-1 py-0.5 rounded-full bg-stone-100">
                                {['👍', '❤️', '😂', '🔥', '👀', '🎯'].map(emoji => (
                                  <button
                                    key={emoji}
                                    onClick={() => {
                                      if (selectedThreadId) {
                                        void dispatch(
                                          persistReaction({
                                            threadId: selectedThreadId,
                                            messageId: msg.id,
                                            emoji,
                                          })
                                        );
                                      }
                                      setReactionPickerMsgId(null);
                                    }}
                                    className="px-0.5 rounded text-sm hover:scale-125 transition-transform"
                                    title={emoji}>
                                    {emoji}
                                  </button>
                                ))}
                                <button
                                  onClick={() => setReactionPickerMsgId(null)}
                                  className="ml-0.5 text-stone-600 hover:text-stone-400 text-xs px-0.5">
                                  ✕
                                </button>
                              </div>
                            ) : (
                              <button
                                onClick={() => setReactionPickerMsgId(msg.id)}
                                className="opacity-0 group-hover/msg:opacity-100 flex items-center px-1.5 py-0.5 rounded-full bg-stone-50 hover:bg-stone-200 text-stone-500 hover:text-stone-300 text-xs transition-all"
                                title="Add reaction">
                                +
                              </button>
                            ))}
                        </div>
                      );
                    })()}
                  </div>
                </div>
              ))}
              {activeThreadId === selectedThreadId && isSending && (
                <div className="flex justify-start">
                  <div className="bg-stone-200/80 rounded-2xl rounded-bl-md px-4 py-3">
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
                            ? 'bg-amber-100 text-amber-600'
                            : entry.status === 'success'
                              ? 'bg-sage-100 text-sage-600'
                              : 'bg-coral-100 text-coral-600'
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
                    className="text-xs text-stone-500 hover:text-stone-700 transition-colors">
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
                  className="flex-shrink-0 px-3 py-1.5 rounded-lg text-[12px] whitespace-nowrap bg-white text-stone-500 border border-stone-200 hover:bg-stone-50 transition-colors disabled:opacity-50 disabled:cursor-not-allowed">
                  {s.text}
                </button>
              ))}
            </div>
          </div>
        )}

        <div className="flex-shrink-0 border-t border-stone-200 px-4 py-3">
          {isNearLimit &&
            !isAtLimit &&
            isFreeTier &&
            shouldShowBanner('conversations-warning', 24 * 60 * 60 * 1000) && (
              <div className="mb-3">
                <UpsellBanner
                  variant="warning"
                  title="Approaching usage limit"
                  message={`You've used ${Math.round(Math.max(usagePct10h, usagePct7d) * 100)}% of your inference budget. Upgrade for higher limits.`}
                  ctaLabel="Upgrade"
                  onCtaClick={() => navigate('/settings/billing')}
                  dismissible
                  onDismiss={() => dismissBanner('conversations-warning')}
                />
              </div>
            )}
          {teamUsage &&
            ((teamUsage.cycleBudgetUsd > 0 && teamUsage.remainingUsd <= 0) ||
              (!teamUsage.bypassCycleLimit &&
                teamUsage.fiveHourCapUsd > 0 &&
                teamUsage.cycleLimit5hr >= teamUsage.fiveHourCapUsd)) && (
              <div className="mb-3 p-3 rounded-xl bg-coral-50 border border-coral-200 flex items-center justify-between gap-3">
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
                  <p className="text-xs text-coral-600 truncate">
                    {teamUsage.cycleBudgetUsd > 0 && teamUsage.remainingUsd <= 0
                      ? `You've hit your weekly limit.${teamUsage.cycleEndsAt ? ` Resets ${formatResetTime(teamUsage.cycleEndsAt)}.` : ''} Top up to continue.`
                      : `10-hour rate limit reached.${teamUsage.fiveHourResetsAt ? ` Resets ${formatResetTime(teamUsage.fiveHourResetsAt)}.` : ''}`}
                  </p>
                </div>
                {teamUsage.cycleBudgetUsd > 0 && teamUsage.remainingUsd <= 0 && (
                  <button
                    onClick={() => navigate('/settings/billing')}
                    className="flex-shrink-0 px-3 py-1.5 rounded-lg bg-coral-500 hover:bg-coral-400 text-white text-xs font-medium transition-colors">
                    Top Up
                  </button>
                )}
              </div>
            )}

          <div className="flex items-center justify-end gap-2 mb-2">
            {(isLoadingBudget || teamUsage) && (
              <div className="relative group">
                {teamUsage ? (
                  <div className="flex items-center gap-2">
                    {!teamUsage.bypassCycleLimit && (
                      <LimitPill
                        label="5h"
                        usedPct={
                          teamUsage.fiveHourCapUsd > 0
                            ? Math.min(1, teamUsage.cycleLimit5hr / teamUsage.fiveHourCapUsd)
                            : 0
                        }
                      />
                    )}
                    <LimitPill
                      label="7d"
                      usedPct={
                        teamUsage.cycleBudgetUsd > 0
                          ? Math.min(
                              1,
                              (teamUsage.cycleBudgetUsd - teamUsage.remainingUsd) /
                                teamUsage.cycleBudgetUsd
                            )
                          : 0
                      }
                    />
                  </div>
                ) : (
                  <span className="text-[10px] text-stone-400 animate-pulse">loading…</span>
                )}
                {teamUsage && (
                  <div className="absolute bottom-full right-0 mb-2 hidden group-hover:block z-50">
                    <div className="bg-stone-900 text-white text-[10px] rounded-lg px-3 py-2 shadow-lg whitespace-nowrap space-y-1.5">
                      {!teamUsage.bypassCycleLimit && (
                        <div className="flex items-center justify-between gap-4">
                          <span className="text-stone-400">5-hour limit</span>
                          <span>
                            ${(teamUsage.cycleLimit5hr ?? 0).toFixed(2)} / $
                            {(teamUsage.fiveHourCapUsd ?? 0).toFixed(2)}
                            {teamUsage.fiveHourResetsAt && (
                              <span className="text-stone-400 ml-1">
                                — resets {formatResetTime(teamUsage.fiveHourResetsAt)}
                              </span>
                            )}
                          </span>
                        </div>
                      )}
                      <div className="flex items-center justify-between gap-4">
                        <span className="text-stone-400">Weekly limit</span>
                        <span>
                          ${(teamUsage.remainingUsd ?? 0).toFixed(2)} / $
                          {(teamUsage.cycleBudgetUsd ?? 0).toFixed(2)} left
                          {teamUsage.cycleEndsAt && (
                            <span className="text-stone-400 ml-1">
                              — resets {formatResetTime(teamUsage.cycleEndsAt)}
                            </span>
                          )}
                        </span>
                      </div>
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>

          {sendError && (
            <div className="flex items-center justify-between mb-2">
              <p className="text-xs text-coral-500" data-chat-send-error-code={sendError.code}>
                {sendError.message}
              </p>
              <button
                onClick={() => setSendError(null)}
                className="text-xs text-stone-500 hover:text-stone-700 transition-colors ml-2 flex-shrink-0">
                Dismiss
              </button>
            </div>
          )}

          {inputMode === 'text' ? (
            <div className="flex items-end gap-3">
              <div className="relative flex flex-1 items-center justify-center rounded-xl border border-stone-200 bg-white transition-all focus-within:border-primary-500/50 focus-within:ring-1 focus-within:ring-primary-500/50">
                <div
                  aria-hidden
                  className="pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap break-words px-4 py-2.5 text-sm leading-normal font-sans">
                  <span className="invisible">{inputValue}</span>
                  <span className="text-stone-500/50">{inlineCompletionSuffix}</span>
                </div>
                <textarea
                  ref={textInputRef}
                  value={inputValue}
                  onChange={e => setInputValue(e.target.value)}
                  onKeyDown={handleInputKeyDown}
                  placeholder="Type a message..."
                  rows={1}
                  disabled={isSending || !rustChat}
                  className="relative z-10 w-full resize-none border-0 bg-transparent pl-4 pr-10 py-2.5 text-sm leading-normal whitespace-pre-wrap break-words font-sans text-stone-900 placeholder:text-stone-400 outline-none focus:outline-none focus-visible:outline-none focus:ring-0 focus-visible:ring-0 max-h-32 disabled:opacity-50 disabled:cursor-not-allowed"
                />
                {/* Mic icon inside input */}
                <button
                  type="button"
                  onClick={() => setInputMode('voice')}
                  disabled={isRecording || isTranscribing || !rustChat}
                  className="absolute right-3 top-1/2 -translate-y-1/2 z-20 text-stone-400 hover:text-stone-600 transition-colors disabled:opacity-40"
                  title="Switch to voice input">
                  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      strokeWidth={1.8}
                      d="M19 11a7 7 0 01-7 7m0 0a7 7 0 01-7-7m7 7v4m0 0H8m4 0h4m-4-8a3 3 0 01-3-3V5a3 3 0 116 0v6a3 3 0 01-3 3z"
                    />
                  </svg>
                </button>
              </div>
              <button
                onClick={() => {
                  void handleSendMessage();
                }}
                disabled={!inputValue.trim() || isSending || !rustChat}
                className="w-10 h-10 flex items-center justify-center rounded-full bg-primary-500 hover:bg-primary-600 text-white disabled:opacity-40 disabled:cursor-not-allowed transition-colors flex-shrink-0">
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
                      strokeWidth={2.5}
                      d="M9 5l7 7-7 7"
                    />
                  </svg>
                )}
              </button>
            </div>
          ) : (
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={() => setInputMode('text')}
                disabled={isRecording || isTranscribing}
                className="w-10 h-10 flex items-center justify-center rounded-full border border-stone-200 bg-white text-stone-500 hover:text-stone-700 hover:border-stone-300 transition-colors disabled:opacity-40"
                title="Switch to text input">
                <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    strokeWidth={1.8}
                    d="M4 6h16M4 12h10m-10 6h16"
                  />
                </svg>
              </button>
              <button
                type="button"
                onClick={() => {
                  void handleVoiceRecordToggle();
                }}
                disabled={!rustChat || isSending || isTranscribing || !canUseMicrophoneApi}
                className={`px-4 py-2.5 rounded-xl text-sm font-medium transition-colors ${
                  isRecording
                    ? 'bg-coral-500 hover:bg-coral-400 text-white'
                    : 'bg-primary-600 hover:bg-primary-500 text-white'
                } disabled:opacity-40 disabled:cursor-not-allowed`}>
                {isTranscribing ? 'Transcribing…' : isRecording ? 'Stop & Send' : 'Start Talking'}
              </button>
              <p className="text-xs text-stone-400 truncate">
                {voiceStatus ??
                  (isPlayingReply && replyMode === 'voice'
                    ? 'Playing voice reply…'
                    : canUseMicrophoneApi
                      ? 'Click "Start Talking" to speak to the agent.'
                      : 'Microphone input is not available in this runtime.')}
              </p>
            </div>
          )}
        </div>
      </div>
      <UsageLimitModal
        open={showLimitModal}
        onClose={() => setShowLimitModal(false)}
        isBudgetExhausted={isBudgetExhausted}
        resetTime={isBudgetExhausted ? teamUsage?.cycleEndsAt : teamUsage?.fiveHourResetsAt}
        currentTier={currentTier}
      />
    </div>
  );
};

export default Conversations;
