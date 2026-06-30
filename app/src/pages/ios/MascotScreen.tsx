/**
 * MascotScreen — iOS-only full-screen mascot chat interface.
 *
 * Layout:
 *   - Small header: paired desktop label + Disconnect button
 *   - YellowMascot canvas (fills the upper ~60% of screen)
 *   - Scrolling transcript of messages above the input row
 *   - Text input row pinned to bottom
 *   - PTT round button (hold to talk, release to send)
 *
 * Chat:
 *   - Sends via openhuman.channel_web_chat RPC (same as desktop chat).
 *   - Subscribes to chat events (text_delta, chat_done, chat_error) for
 *     mascot face transitions and transcript display.
 *   - Uses useHumanMascot() to drive face/viseme state.
 *
 * PTT (Layer 6):
 *   - onPointerDown -> startListening(); pttActive = true.
 *   - onPointerUp   -> stopListening() -> send transcript as chat message.
 *   - onTranscriptPartial -> shows live caption above button.
 *   - onError -> surfaces a toast.
 *   - Agent reply is spoken via speak() once chat_done fires.
 *   - Any new PTT press cancels active TTS first.
 */
import debug from 'debug';
import { type FC, type FormEvent, useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import {
  cancelSpeech,
  onError as onPttError,
  onTranscriptPartial,
  speak,
  startListening,
  stopListening,
} from 'tauri-plugin-ptt-api';

import Button from '../../components/ui/Button';
import { RiveMascot } from '../../features/human/Mascot';
import { useHumanMascot } from '../../features/human/useHumanMascot';
import { useT } from '../../lib/i18n/I18nContext';
import {
  type ChatDoneEvent,
  type ChatErrorEvent,
  chatSend,
  type ChatTextDeltaEvent,
  subscribeChatEvents,
} from '../../services/chatService';
import { deleteProfile, listProfiles } from '../../services/transport/profileStore';

const log = debug('ios:mascot-screen');
const logErr = debug('ios:mascot-screen:error');

// -- constants ---------------------------------------------------------------

/** Default thread ID for the iOS mascot chat. Static for now. */
const IOS_THREAD_ID = 'ios-mascot-thread';

/** Model to use for iOS chat. Falls through to core default if empty. */
const IOS_CHAT_MODEL = '';

// -- types -------------------------------------------------------------------

interface Message {
  id: string;
  role: 'user' | 'assistant';
  text: string;
  /** True while a streaming response is still accumulating. */
  streaming?: boolean;
}

// -- sub-components ----------------------------------------------------------

interface TranscriptProps {
  messages: Message[];
}

const MascotChatTranscript: FC<TranscriptProps> = ({ messages }) => {
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' });
  }, [messages]);

  if (messages.length === 0) return null;

  return (
    <div className="flex flex-col gap-2 px-4 pb-2 overflow-y-auto max-h-[30vh]">
      {messages.map(msg => (
        <div
          key={msg.id}
          className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}>
          <div
            className={`max-w-[80%] rounded-2xl px-3 py-2 text-sm leading-snug
              ${
                msg.role === 'user'
                  ? 'bg-[#4A83DD] text-white rounded-br-sm'
                  : 'bg-surface/10 text-white/90 rounded-bl-sm'
              }
              ${msg.streaming ? 'animate-pulse' : ''}`}>
            {msg.text}
            {msg.streaming && <span className="ml-1 opacity-60">...</span>}
          </div>
        </div>
      ))}
      <div ref={bottomRef} />
    </div>
  );
};

// -- PTT button ---------------------------------------------------------------

interface PTTButtonProps {
  active: boolean;
  partialText: string;
  ariaLabel: string;
  onDown: () => void;
  onUp: () => void;
}

const PTTButton: FC<PTTButtonProps> = ({ active, partialText, ariaLabel, onDown, onUp }) => {
  return (
    <div className="relative flex flex-col items-center justify-center gap-1">
      {partialText && (
        <div
          className="absolute bottom-full mb-2 px-3 py-1.5 rounded-lg bg-black/80 text-white text-xs
                     max-w-[200px] text-center pointer-events-none z-10">
          {partialText}
        </div>
      )}
      <button
        type="button"
        aria-label={ariaLabel}
        onPointerDown={e => {
          // setPointerCapture is not available in jsdom test environments.
          if (typeof e.currentTarget.setPointerCapture === 'function') {
            e.currentTarget.setPointerCapture(e.pointerId);
          }
          onDown();
        }}
        onPointerUp={onUp}
        onPointerCancel={onUp}
        className={`w-14 h-14 rounded-full border flex items-center justify-center
                   transition-all select-none touch-none
                   ${
                     active
                       ? 'bg-[#4A83DD] border-[#4A83DD] scale-110'
                       : 'bg-surface/10 border-white/20 opacity-80'
                   }`}>
        <svg width="24" height="24" viewBox="0 0 24 24" fill="none" aria-hidden="true">
          <path d="M12 1a4 4 0 0 1 4 4v6a4 4 0 0 1-8 0V5a4 4 0 0 1 4-4z" fill="white" />
          <path
            d="M19 10a1 1 0 0 1 2 0 9 9 0 0 1-18 0 1 1 0 0 1 2 0 7 7 0 0 0 14 0z"
            fill="white"
            fillOpacity="0.8"
          />
          <line
            x1="12"
            y1="19"
            x2="12"
            y2="23"
            stroke="white"
            strokeWidth="2"
            strokeLinecap="round"
          />
        </svg>
      </button>
    </div>
  );
};

// -- toast -------------------------------------------------------------------

interface ToastProps {
  message: string;
  onDismiss: () => void;
}

const Toast: FC<ToastProps> = ({ message, onDismiss }) => (
  <div
    role="alert"
    className="absolute bottom-24 left-1/2 -translate-x-1/2 z-50
               px-4 py-2 rounded-xl bg-red-500/90 text-white text-sm
               max-w-[80%] text-center shadow-lg"
    onClick={onDismiss}>
    {message}
  </div>
);

// -- main component ----------------------------------------------------------

export const MascotScreen: FC = () => {
  const navigate = useNavigate();
  const { t } = useT();

  const [messages, setMessages] = useState<Message[]>([]);
  const [inputText, setInputText] = useState('');
  const [isSending, setIsSending] = useState(false);

  // PTT state
  const [pttActive, setPttActive] = useState(false);
  const [partialText, setPartialText] = useState('');
  const [toast, setToast] = useState<string | null>(null);

  const streamingIdRef = useRef<string | null>(null);
  // Ref tracks whether PTT session is live — readable from async callbacks
  // without a stale closure over the pttActive state variable.
  const pttActiveRef = useRef(false);

  const { face, visemeCode } = useHumanMascot({ listening: pttActive });

  // Derive label from stored profile.
  const pairedLabel = (() => {
    const profiles = listProfiles();
    return profiles[0]?.label ?? t('iosMascot.defaultPairedLabel');
  })();

  log('[ios] mascot screen mounted pairedLabel=%s', pairedLabel);

  // -- chat event subscription -----------------------------------------------

  useEffect(() => {
    const unsub = subscribeChatEvents({
      onTextDelta: (e: ChatTextDeltaEvent) => {
        const sid = streamingIdRef.current;
        if (!sid) return;
        setMessages(prev =>
          prev.map(m => (m.id === sid ? { ...m, text: m.text + e.delta, streaming: true } : m))
        );
      },
      onDone: (e: ChatDoneEvent) => {
        const sid = streamingIdRef.current;
        log('[ios] chat done thread_id=%s', e.thread_id);
        streamingIdRef.current = null;
        setMessages(prev =>
          prev.map(m => (m.id === sid ? { ...m, text: e.full_response, streaming: false } : m))
        );
        setIsSending(false);

        // Speak the assistant reply via TTS. Do not speak if the user is
        // already recording again (PTT pressed before the reply arrived).
        if (e.full_response && !pttActiveRef.current) {
          log('[ios] TTS: speaking assistant reply len=%d', e.full_response.length);
          speak(e.full_response).catch((err: unknown) => {
            logErr('[ios] TTS speak error: %o', err);
          });
        }
      },
      onError: (e: ChatErrorEvent) => {
        logErr(
          '[ios] chat error thread_id=%s type=%s message=%s',
          e.thread_id,
          e.error_type,
          e.message
        );
        streamingIdRef.current = null;
        setIsSending(false);
        setMessages(prev => [
          ...prev,
          {
            id: `err-${Date.now()}`,
            role: 'assistant' as const,
            text: t('iosMascot.error.generic'),
            streaming: false,
          },
        ]);
      },
    });
    return unsub;
  }, [t]);

  // -- PTT event subscription ------------------------------------------------

  useEffect(() => {
    let unlistenPartial: (() => void) | undefined;
    let unlistenError: (() => void) | undefined;

    onTranscriptPartial(text => {
      log('[ios] PTT partial text_len=%d', text.length);
      setPartialText(text);
    })
      .then((fn: () => void) => {
        unlistenPartial = fn;
      })
      .catch((err: unknown) => logErr('[ios] PTT partial listener setup failed: %o', err));

    onPttError(err => {
      logErr('[ios] PTT error code=%s message=%s', err.code, err.message);
      // An interruption may have stopped the recorder without onPointerUp
      // being called — reset PTT state so the button is not stuck active.
      if (pttActiveRef.current) {
        pttActiveRef.current = false;
        setPttActive(false);
        setPartialText('');
      }
      setToast(err.message);
      setTimeout(() => setToast(null), 4000);
    })
      .then((fn: () => void) => {
        unlistenError = fn;
      })
      .catch((err: unknown) => logErr('[ios] PTT error listener setup failed: %o', err));

    return () => {
      unlistenPartial?.();
      unlistenError?.();
    };
  }, []);

  // -- shared send (declared before PTT handlers so it is in scope) -----------

  const sendMessage = useCallback(
    async (text: string) => {
      log('[ios] sendMessage len=%d thread_id=%s', text.length, IOS_THREAD_ID);

      const userMsg: Message = { id: `user-${Date.now()}`, role: 'user', text };
      const assistantId = `asst-${Date.now()}`;
      streamingIdRef.current = assistantId;

      setMessages(prev => [
        ...prev,
        userMsg,
        { id: assistantId, role: 'assistant', text: '', streaming: true },
      ]);
      setIsSending(true);

      try {
        await chatSend({ threadId: IOS_THREAD_ID, message: text, model: IOS_CHAT_MODEL });
        log('[ios] chatSend enqueued thread_id=%s', IOS_THREAD_ID);
      } catch (err) {
        logErr('[ios] chatSend failed: %o', err);
        streamingIdRef.current = null;
        setIsSending(false);
        setMessages(prev =>
          prev.map(m =>
            m.id === assistantId
              ? { ...m, text: t('iosMascot.error.sendFailed'), streaming: false }
              : m
          )
        );
      }
    },
    [t]
  );

  // -- PTT handlers ----------------------------------------------------------

  const handlePttDown = useCallback(() => {
    if (isSending) return;
    log('[ios] PTT down — starting listening');

    // Cancel any in-progress TTS before starting a new recording.
    cancelSpeech().catch((err: unknown) =>
      logErr('[ios] cancelSpeech on PTT down failed: %o', err)
    );

    pttActiveRef.current = true;
    setPttActive(true);
    setPartialText('');

    startListening().catch((err: unknown) => {
      logErr('[ios] startListening failed: %o', err);
      pttActiveRef.current = false;
      setPttActive(false);
      const msg = err instanceof Error ? err.message : String(err);
      setToast(msg);
      setTimeout(() => setToast(null), 4000);
    });
  }, [isSending]);

  const handlePttUp = useCallback(() => {
    if (!pttActiveRef.current) return;
    log('[ios] PTT up — stopping listening');

    pttActiveRef.current = false;
    setPttActive(false);

    stopListening()
      .then((result: { text: string; isFinal: boolean }) => {
        const text = result.text.trim();
        setPartialText('');
        log('[ios] PTT transcript text_len=%d', text.length);
        if (!text) return;
        void sendMessage(text);
      })
      .catch((err: unknown) => {
        logErr('[ios] stopListening failed: %o', err);
        setPartialText('');
      });
  }, [sendMessage]);

  const handleSend = useCallback(
    async (e: FormEvent) => {
      e.preventDefault();
      const text = inputText.trim();
      if (!text || isSending) return;
      setInputText('');
      // Cancel any active TTS when the user types a new message.
      cancelSpeech().catch(() => undefined);
      await sendMessage(text);
    },
    [inputText, isSending, sendMessage]
  );

  function handleDisconnect() {
    log('[ios] disconnecting — clearing profile and navigating to /pair');
    const profiles = listProfiles();
    profiles.forEach(p => deleteProfile(p.id));
    navigate('/pair', { replace: true });
  }

  return (
    <div className="flex flex-col h-screen bg-[#0f1117] text-white overflow-hidden relative">
      {/* Header */}
      <div className="flex items-center justify-between px-4 pt-safe-top py-3 border-b border-white/10 shrink-0">
        <div className="flex flex-col">
          <span className="text-xs text-white/40 uppercase tracking-wide">
            {t('iosMascot.connectedTo')}
          </span>
          <span className="text-sm font-medium text-white/90 truncate max-w-[200px]">
            {pairedLabel}
          </span>
        </div>
        <Button variant="secondary" tone="danger" size="sm" onClick={handleDisconnect}>
          {t('iosMascot.disconnect')}
        </Button>
      </div>

      {/* Mascot canvas */}
      <div className="flex-1 flex items-center justify-center overflow-hidden min-h-0 py-4">
        <div className="w-full max-w-xs aspect-square">
          <RiveMascot face={face} visemeCode={visemeCode} />
        </div>
      </div>

      {/* Transcript */}
      <MascotChatTranscript messages={messages} />

      {/* Toast */}
      {toast && <Toast message={toast} onDismiss={() => setToast(null)} />}

      {/* Input row */}
      <div className="shrink-0 border-t border-white/10 px-4 pb-safe-bottom py-3">
        <form onSubmit={e => void handleSend(e)} className="flex items-center gap-3">
          {/* PTT button — Layer 6 live implementation */}
          <PTTButton
            active={pttActive}
            partialText={partialText}
            ariaLabel={t('iosMascot.pushToTalk')}
            onDown={handlePttDown}
            onUp={handlePttUp}
          />

          {/* Text input */}
          <input
            type="text"
            value={inputText}
            onChange={e => setInputText(e.target.value)}
            disabled={isSending}
            placeholder={isSending ? t('iosMascot.thinking') : t('iosMascot.typeMessage')}
            className="flex-1 bg-surface/10 text-white placeholder-white/30 rounded-xl
                       px-4 py-3 text-sm outline-none border border-white/10
                       focus:border-[#4A83DD]/60 transition-colors
                       disabled:opacity-50"
          />

          {/* Send button */}
          <button
            type="submit"
            disabled={!inputText.trim() || isSending}
            aria-label={t('iosMascot.sendMessage')}
            className="w-10 h-10 rounded-xl bg-[#4A83DD] flex items-center justify-center
                       disabled:opacity-30 active:opacity-70 transition-opacity shrink-0">
            <svg width="18" height="18" viewBox="0 0 18 18" fill="none" aria-hidden="true">
              <path
                d="M2 9h14M10 3l6 6-6 6"
                stroke="white"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </button>
        </form>
      </div>
    </div>
  );
};
