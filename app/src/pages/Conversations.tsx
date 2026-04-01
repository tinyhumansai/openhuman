import { convertFileSrc } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';
import Markdown from 'react-markdown';
import { useNavigate } from 'react-router-dom';

import { useLocalModelStatus } from '../hooks/useLocalModelStatus';
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
import { selectSocketStatus } from '../store/socketSelectors';
import {
  addInferenceResponse,
  addMessageLocal,
  addReaction,
  createThreadLocal,
  fetchSuggestedQuestions,
  setActiveThread,
  setLastViewed,
  setSelectedThread,
} from '../store/threadSlice';
import type { ThreadMessage } from '../types/thread';
import { getSegmentDelay, segmentMessage } from '../utils/messageSegmentation';
import {
  isTauri,
  type LocalAiChatMessage,
  openhumanAutocompleteAccept,
  openhumanAutocompleteCurrent,
  openhumanLocalAiChat,
  openhumanLocalAiShouldReact,
  openhumanVoiceStatus,
  openhumanVoiceTranscribeBytes,
  openhumanVoiceTts,
} from '../utils/tauriCommands';

const DEFAULT_THREAD_ID = 'default-thread';
const DEFAULT_THREAD_TITLE = 'Conversation';
type ToolTimelineEntryStatus = 'running' | 'success' | 'error';
type InputMode = 'text' | 'voice';
type ReplyMode = 'text' | 'voice';
const AUTOCOMPLETE_POLL_DEBOUNCE_MS = 180;

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
  const normalizedInput = input;
  const normalizedSuggestion = suggestion;

  if (!normalizedInput || !normalizedSuggestion) {
    return '';
  }

  if (normalizedSuggestion === normalizedInput) {
    return '';
  }

  if (normalizedSuggestion.startsWith(normalizedInput)) {
    return normalizedSuggestion.slice(normalizedInput.length);
  }

  return '';
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

  const [inputValue, setInputValue] = useState('');
  const [copiedMessageId, setCopiedMessageId] = useState<string | null>(null);
  const [inputMode, setInputMode] = useState<InputMode>('text');
  const [replyMode, setReplyMode] = useState<ReplyMode>('text');
  const [isRecording, setIsRecording] = useState(false);
  const [isTranscribing, setIsTranscribing] = useState(false);
  const [voiceStatus, setVoiceStatus] = useState<string | null>(null);
  const [isPlayingReply, setIsPlayingReply] = useState(false);
  const [inlineSuggestionValue, setInlineSuggestionValue] = useState('');

  const [availableModels, setAvailableModels] = useState<ModelInfo[]>([]);
  const [selectedModel, setSelectedModel] = useState('agentic-v1');
  const [isLoadingModels, setIsLoadingModels] = useState(false);
  const [isSending, setIsSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const socketStatus = useAppSelector(selectSocketStatus);
  const [toolTimelineByThread, setToolTimelineByThread] = useState<
    Record<string, ToolTimelineEntry[]>
  >({});
  const rustChat = useRustChat();
  const isLocalModelActive = useLocalModelStatus();
  const isLocalModelActiveRef = useRef(isLocalModelActive);
  const [isDelivering, setIsDelivering] = useState(false);
  const deliveryActiveRef = useRef(false);
  const [reactionPickerMsgId, setReactionPickerMsgId] = useState<string | null>(null);
  const defaultChannelType = useAppSelector(
    state => state.channelConnections?.defaultMessagingChannel ?? 'web'
  );
  const pendingReactionRef = useRef<
    Map<string, { msgId: string; content: string; threadId: string }>
  >(new Map());

  const selectedThreadIdRef = useRef(selectedThreadId);
  useEffect(() => {
    selectedThreadIdRef.current = selectedThreadId;
  }, [selectedThreadId]);

  useEffect(() => {
    isLocalModelActiveRef.current = isLocalModelActive;
  }, [isLocalModelActive]);

  const [teamUsage, setTeamUsage] = useState<TeamUsage | null>(null);
  const [isLoadingBudget, setIsLoadingBudget] = useState(false);

  const messagesEndRef = useRef<HTMLDivElement>(null);
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const mediaStreamRef = useRef<MediaStream | null>(null);
  const audioChunksRef = useRef<Blob[]>([]);
  const replyAudioRef = useRef<HTMLAudioElement | null>(null);
  const lastSpokenMessageIdRef = useRef<string | null>(null);
  const autocompleteDebounceRef = useRef<number | null>(null);
  const autocompleteRequestSeqRef = useRef(0);

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

    // Always set selected thread to ensure messages view is synced from persisted storage
    dispatch(setSelectedThread(DEFAULT_THREAD_ID));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dispatch]);

  useEffect(() => {
    setIsLoadingModels(true);
    inferenceApi
      .listModels()
      .then(data => {
        if (data.data.length > 0) {
          setAvailableModels(data.data);
          const preferred = data.data.find(m => m.id === 'agentic-v1');
          setSelectedModel(preferred ? preferred.id : data.data[0].id);
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
    if (
      !isTauri() ||
      !rustChat ||
      inputMode !== 'text' ||
      isSending ||
      inputValue.trim().length === 0
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
      deliveryActiveRef.current = false;
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

  // Proactively check voice binary availability when switching to voice mode
  useEffect(() => {
    if (inputMode !== 'voice' || !rustChat) return;
    let cancelled = false;
    void (async () => {
      try {
        const resp = await openhumanVoiceStatus();
        if (cancelled) return;
        const status = resp.result;
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
              nextEntries[i] = { ...entry, status: event.success ? 'success' : 'error' };
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

        // Fire-and-forget: auto-react to the user's message
        const pending = pendingReactionRef.current.get(event.thread_id);
        if (pending) {
          maybeAutoReact(pending.msgId, pending.content, pending.threadId);
          pendingReactionRef.current.delete(event.thread_id);
        }

        // Multi-bubble delivery gate: only when local model is active
        if (!isLocalModelActiveRef.current) {
          dispatch(
            addInferenceResponse({ content: event.full_response, threadId: event.thread_id })
          );
          setIsSending(false);
          dispatch(setActiveThread(null));
          return;
        }

        const segments = segmentMessage(event.full_response);

        if (segments.length <= 1) {
          dispatch(
            addInferenceResponse({ content: event.full_response, threadId: event.thread_id })
          );
          setIsSending(false);
          dispatch(setActiveThread(null));
          return;
        }

        // Async delivery: show each segment as a separate bubble with a typing pause
        setIsDelivering(true);
        deliveryActiveRef.current = true;

        void (async () => {
          for (let i = 0; i < segments.length; i++) {
            if (!deliveryActiveRef.current) break;

            if (i > 0) {
              await new Promise<void>(resolve =>
                setTimeout(resolve, getSegmentDelay(segments[i - 1]))
              );
            }

            if (!deliveryActiveRef.current) break;

            dispatch(addInferenceResponse({ content: segments[i], threadId: event.thread_id }));
          }

          deliveryActiveRef.current = false;
          setIsDelivering(false);
          setIsSending(false);
          // activeThreadId was already cleared by the first addInferenceResponse dispatch
        })();
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
   * Segment a complete response string and dispatch each segment as a
   * separate message bubble with a typing pause between them.
   * Local-model-only path — no cloud API calls.
   */
  const deliverLocalResponse = async (fullResponse: string, threadId: string) => {
    const segments = segmentMessage(fullResponse);

    if (segments.length <= 1) {
      dispatch(addInferenceResponse({ content: fullResponse, threadId }));
      return;
    }

    setIsDelivering(true);
    deliveryActiveRef.current = true;

    for (let i = 0; i < segments.length; i++) {
      if (!deliveryActiveRef.current) break;

      if (i > 0) {
        await new Promise<void>(resolve => setTimeout(resolve, getSegmentDelay(segments[i - 1])));
      }

      if (!deliveryActiveRef.current) break;

      dispatch(addInferenceResponse({ content: segments[i], threadId }));
    }

    deliveryActiveRef.current = false;
    setIsDelivering(false);
  };

  /**
   * Fire-and-forget: ask the local model if we should auto-react to the
   * user's message with an emoji. Adds a personal touch based on channel type.
   */
  const maybeAutoReact = (userMessageId: string, messageContent: string, threadId: string) => {
    if (!isTauri() || !isLocalModelActiveRef.current) return;

    void openhumanLocalAiShouldReact(messageContent, defaultChannelType)
      .then(response => {
        const decision = response.result;
        if (decision?.should_react && decision.emoji) {
          console.debug('[conversations:auto-react] reacting with', decision.emoji);
          dispatch(addReaction({ threadId, messageId: userMessageId, emoji: decision.emoji }));
        }
      })
      .catch(err => {
        console.debug('[conversations:auto-react] failed:', err);
      });
  };

  const handleSendMessage = async (text?: string) => {
    const normalized = text ?? inputValue;
    const trimmed = normalized.trim();

    if (!trimmed || !selectedThreadId || isSending) return;
    if (!isLocalModelActiveRef.current && socketStatus !== 'connected') {
      setSendError('Realtime socket is not connected.');
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
    pendingReactionRef.current.set(sendingThreadId, {
      msgId: userMessage.id,
      content: trimmed,
      threadId: sendingThreadId,
    });

    setInputValue('');
    setSendError(null);
    setIsSending(true);
    setToolTimelineByThread(prev => ({ ...prev, [sendingThreadId]: [] }));
    dispatch(setActiveThread(sendingThreadId));

    // ── Local Ollama path ────────────────────────────────────────────────────
    // When a local model is ready, bypass the cloud socket entirely.
    // Zero cloud tokens consumed on this path.
    if (isLocalModelActiveRef.current) {
      try {
        // Build message history: convert stored messages + the new user turn
        const storedMessages =
          (
            store.getState() as {
              thread: {
                messagesByThreadId: Record<string, import('../types/thread').ThreadMessage[]>;
              };
            }
          ).thread.messagesByThreadId[sendingThreadId] ?? [];

        const history: LocalAiChatMessage[] = storedMessages
          .filter(m => m.sender === 'user' || m.sender === 'agent')
          .map(m => ({
            role: m.sender === 'user' ? ('user' as const) : ('assistant' as const),
            content: m.content,
          }));

        console.debug('[conversations:local] sending to local model', {
          historyLength: history.length,
          threadId: sendingThreadId,
        });

        const response = await openhumanLocalAiChat(history);
        const reply = response.result?.trim() ?? '';

        if (!reply) {
          throw new Error('Local model returned an empty response.');
        }

        await deliverLocalResponse(reply, sendingThreadId);
        pendingReactionRef.current.delete(sendingThreadId);
        maybeAutoReact(userMessage.id, trimmed, sendingThreadId);
      } catch (err) {
        pendingReactionRef.current.delete(sendingThreadId);
        const msg = err instanceof Error ? err.message : String(err);
        setSendError(msg);
        dispatch(
          addInferenceResponse({
            content: 'Local model error — please try again.',
            threadId: sendingThreadId,
          })
        );
      } finally {
        setIsSending(false);
        dispatch(setActiveThread(null));
      }
      return;
    }

    // ── Cloud socket path (unchanged) ────────────────────────────────────────
    try {
      await chatSend({ threadId: sendingThreadId, message: trimmed, model: selectedModel });

      // setIsSending(false) and setActiveThread(null) happen in the onDone/onError event handlers
    } catch (err) {
      // Chat loop errors are emitted via socket events; this catch handles emit-level failures.
      const msg = err instanceof Error ? err.message : String(err);
      setSendError(msg);
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
      const transcript = result.result.text.trim();

      if (!transcript) {
        setVoiceStatus('No speech detected. Try again.');
        return;
      }

      setVoiceStatus(`Heard: ${transcript}`);
      await handleSendMessage(transcript);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setSendError(`Voice transcription failed: ${message}`);
      setVoiceStatus(null);
    } finally {
      setIsTranscribing(false);
    }
  };

  const handleVoiceRecordToggle = async () => {
    if (!rustChat || isSending || isTranscribing) return;
    if (!canUseMicrophoneApi) {
      setSendError(
        'Microphone capture is unavailable in this runtime. Use Text mode, or run the desktop app bundle with microphone permissions enabled.'
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
        setIsRecording(false);
        mediaStreamRef.current?.getTracks().forEach(track => track.stop());
        mediaStreamRef.current = null;
        setSendError('Microphone recording failed.');
      };
      recorder.onstop = () => {
        void transcribeAndSendAudio(recorder.mimeType);
      };

      mediaRecorderRef.current = recorder;
      setVoiceStatus('Listening… click Stop to send.');
      setSendError(null);
      setIsRecording(true);
      recorder.start();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setSendError(`Microphone access failed: ${message}`);
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

        const audioSrc = convertFileSrc(ttsResult.result.output_path);
        const audio = new window.Audio(audioSrc);
        replyAudioRef.current?.pause();
        replyAudioRef.current = audio;

        await audio.play();
      } catch {
        if (!cancelled) {
          setSendError('Failed to play voice reply.');
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

    if (e.key === 'Tab' && inlineSuffix.length > 0) {
      e.preventDefault();
      setInputValue(prev => prev + inlineSuffix);
      setInlineSuggestionValue('');
      if (isTauri()) {
        void openhumanAutocompleteAccept({ suggestion: inputValue + inlineSuffix }).catch(() => {
          // Keep local UX smooth even if accept RPC fails.
        });
      }
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

  const selectedThread = threads.find(t => t.id === selectedThreadId);
  const selectedThreadToolTimeline = selectedThreadId
    ? (toolTimelineByThread[selectedThreadId] ?? [])
    : [];
  const inlineCompletionSuffix = getInlineCompletionSuffix(inputValue, inlineSuggestionValue);

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
                                dispatch(
                                  addReaction({
                                    threadId: selectedThreadId,
                                    messageId: msg.id,
                                    emoji,
                                  })
                                )
                              }
                              className="flex items-center gap-0.5 px-1.5 py-0.5 rounded-full bg-primary-600/20 border border-primary-500/30 text-xs transition-colors hover:bg-primary-600/30"
                              title={`Remove ${emoji}`}>
                              {emoji}
                            </button>
                          ))}
                          {msg.sender === 'agent' &&
                            (reactionPickerMsgId === msg.id ? (
                              <div className="flex items-center gap-0.5 px-1 py-0.5 rounded-full bg-white/10">
                                {['👍', '❤️', '😂', '🔥', '👀', '🎯'].map(emoji => (
                                  <button
                                    key={emoji}
                                    onClick={() => {
                                      if (selectedThreadId) {
                                        dispatch(
                                          addReaction({
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
                                className="opacity-0 group-hover/msg:opacity-100 flex items-center px-1.5 py-0.5 rounded-full bg-white/5 hover:bg-white/15 text-stone-500 hover:text-stone-300 text-xs transition-all"
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
              {((activeThreadId === selectedThreadId && isSending) || isDelivering) && (
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
            <div className="flex items-center gap-1 rounded-lg border border-white/10 bg-white/5 p-1">
              <span className="text-[10px] text-stone-500 px-1">Input</span>
              <button
                type="button"
                onClick={() => setInputMode('text')}
                disabled={isRecording || isTranscribing}
                className={`px-2 py-1 rounded-md text-[11px] transition-colors ${
                  inputMode === 'text'
                    ? 'bg-primary-600 text-white'
                    : 'text-stone-300 hover:bg-white/10'
                }`}>
                Text
              </button>
              <button
                type="button"
                onClick={() => setInputMode('voice')}
                disabled={isRecording || isTranscribing || !rustChat || !canUseMicrophoneApi}
                className={`px-2 py-1 rounded-md text-[11px] transition-colors ${
                  inputMode === 'voice'
                    ? 'bg-primary-600 text-white'
                    : 'text-stone-300 hover:bg-white/10'
                }`}>
                Voice
              </button>
            </div>
            <div className="flex items-center gap-1 rounded-lg border border-white/10 bg-white/5 p-1">
              <span className="text-[10px] text-stone-500 px-1">Reply</span>
              <button
                type="button"
                onClick={() => setReplyMode('text')}
                className={`px-2 py-1 rounded-md text-[11px] transition-colors ${
                  replyMode === 'text'
                    ? 'bg-primary-600 text-white'
                    : 'text-stone-300 hover:bg-white/10'
                }`}>
                Text
              </button>
              <button
                type="button"
                onClick={() => setReplyMode('voice')}
                disabled={!rustChat}
                className={`px-2 py-1 rounded-md text-[11px] transition-colors ${
                  replyMode === 'voice'
                    ? 'bg-primary-600 text-white'
                    : 'text-stone-300 hover:bg-white/10'
                }`}>
                Voice
              </button>
            </div>
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
                    <svg
                      width={size}
                      height={size}
                      viewBox={`0 0 ${size} ${size}`}
                      className="-rotate-90">
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
                      <span className="text-[10px] text-stone-500">
                        ${teamUsage.remainingUsd.toFixed(2)}
                      </span>
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

          {inputMode === 'text' ? (
            <div className="flex items-end gap-2">
              <div className="relative flex-1 rounded-xl border border-white/10 bg-white/5 focus-within:ring-1 focus-within:ring-primary-500/50 focus-within:border-primary-500/50 transition-all">
                <div
                  aria-hidden
                  className="pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap break-words px-4 py-2.5 text-sm leading-normal">
                  <span className="invisible">{inputValue}</span>
                  <span className="text-stone-500/50">{inlineCompletionSuffix}</span>
                </div>
                <textarea
                  value={inputValue}
                  onChange={e => setInputValue(e.target.value)}
                  onKeyDown={handleInputKeyDown}
                  placeholder="Type a message..."
                  rows={1}
                  disabled={isSending || !rustChat}
                  className="relative z-10 w-full resize-none border-0 bg-transparent px-4 py-2.5 text-sm placeholder:text-stone-500 focus:outline-none focus:ring-0 max-h-32 disabled:opacity-50 disabled:cursor-not-allowed"
                />
              </div>
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
          ) : (
            <div className="flex items-center gap-2">
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
    </div>
  );
};

export default Conversations;
