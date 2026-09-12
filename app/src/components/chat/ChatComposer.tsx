import { ComposerPrimitive } from '@assistant-ui/react';
import debugFactory from 'debug';
import { useRef, useState } from 'react';

import type { ChatSendError } from '../../chat/chatSendError';
import type { Attachment } from '../../lib/attachments';
import { useT } from '../../lib/i18n/I18nContext';
import { Button } from '../ui';
import AttachmentPreview from './AttachmentPreview';
import {
  AddAttachmentIcon,
  HumanModeIcon,
  MicIcon,
  SendIcon,
  SendingSpinnerIcon,
  StopIcon,
} from './composer/ComposerIcons';
import { ComposerRuntimeBoundary } from './composer/ComposerRuntimeBoundary';
import {
  COMPOSER_ACTION_WRAPPER,
  COMPOSER_ADD_ATTACHMENT,
  COMPOSER_ATTACHMENTS,
  COMPOSER_DRAG_OVERLAY,
  COMPOSER_DROPZONE,
  COMPOSER_GHOST_OVERLAY,
  COMPOSER_INPUT,
  COMPOSER_MIC,
  COMPOSER_ROOT,
  COMPOSER_SEND,
} from './composer/composerStyles';
import { useComposerTextBridge } from './composer/useComposerTextBridge';
import ModelQualityPill from './ModelQualityPill';

const debug = debugFactory('openhuman:chat-composer');

/**
 * Render-loop guard threshold: if the component re-renders this many times
 * within a single microtick, we log a warning to help diagnose "Maximum update
 * depth exceeded" issues (TAURI-REACT-2G). React's own limit is 50; we warn at
 * a lower threshold so developers catch it before the crash.
 */
const RENDER_LOOP_WARN_THRESHOLD = 30;

export interface ChatComposerProps {
  inputValue: string;
  setInputValue: (value: string | ((prev: string) => string)) => void;
  onSend: (text?: string) => Promise<void>;
  /**
   * Cancel the in-flight generation for the selected thread. When provided, the
   * Send button morphs into a Stop button while `isSending` is true so the user
   * can halt the response from inside the composer. When omitted, the Send
   * button falls back to a disabled spinner during generation.
   */
  onStopGeneration?: () => void;
  /**
   * Open the Human page (the full-bleed mascot stage). When supplied, the
   * primary action doubles as the entry point to it: with the composer empty
   * the button is a person glyph that routes there, and it becomes Send the
   * moment there is something to send — the same slot ChatGPT gives its voice
   * mode. Omit it and the empty composer keeps the plain disabled Send button.
   */
  onOpenHumanMode?: () => void;
  textInputRef: React.RefObject<HTMLTextAreaElement | null>;
  fileInputRef: React.RefObject<HTMLInputElement | null>;
  composerInteractionBlocked: boolean;
  isSending: boolean;
  /**
   * When true, the selected thread has an in-flight turn but the composer stays
   * usable: plain Enter / the Send button queue a follow-up (sent after the
   * current turn), and Cmd/Ctrl+Enter forks a parallel branch. Keeps the
   * textarea + send button editable even though `composerInteractionBlocked` is
   * set, and surfaces a follow-up hint instead of showing the in-flight spinner.
   */
  allowParallelSend?: boolean;
  attachments: Attachment[];
  onAttachFiles: (files: FileList | File[] | null) => Promise<void>;
  onRemoveAttachment: (id: string) => void;
  attachError: ChatSendError | null;
  onSwitchToMicCloud: () => void;
  handleInputKeyDown: (e: React.KeyboardEvent<HTMLTextAreaElement>) => void;
  inlineCompletionSuffix: string;
  isComposingTextRef: React.MutableRefObject<boolean>;
  maxAttachments: number;
  allowedMimeTypes: readonly string[];
  /**
   * Whether chat multimodal attachments are available. When `false`, the
   * attach button, hidden file input, and preview strip are not rendered.
   */
  attachmentsEnabled: boolean;
  /**
   * Whether the voice-mode (mic) button is shown. Defaults to `true` for the
   * main chat; surfaces that don't have a voice mode (e.g. the flows Workflow
   * Copilot) pass `false` to hide it while still reusing this composer.
   */
  micEnabled?: boolean;
  /**
   * Overrides the textarea placeholder. Defaults to the main chat's
   * type-a-message / follow-up hint; surfaces like the Workflow Copilot pass
   * their own prompt.
   */
  placeholder?: string;
  /**
   * Optional nodes stacked above the input box but *outside* its focus
   * highlight — e.g. the queued-follow-ups strip and the thread-goal editor.
   * They render within the overall composer component (so they move with it)
   * yet are not wrapped by the focus ring/border of the input box. Entries that
   * render `null` contribute nothing.
   */
  headerSlots?: React.ReactNode[];
  /**
   * Optional node anchored to the input box itself (not to `headerSlots`) so it
   * can stand on the box's top edge. The merged chat surface passes its mascot
   * dock here; surfaces without a mascot (Workflow Copilot, embedded sidebars)
   * omit it and render exactly as before.
   */
  mascotDock?: React.ReactNode;
  /** Per-composer model route selected from the assistant-ui-style control. */
  modelOverride?: string | null;
  /** `null` means managed — see `ModelQualityPill`'s `onValueChange`. */
  onModelOverrideChange?: (value: string | null, contextWindow?: number | null) => void;
}

/**
 * The chat composer, rebuilt on assistant-ui's **headless** `ComposerPrimitive`
 * and styled from its default composer's spec (ported in
 * `composer/composerStyles.ts` — see that file for the class-by-class mapping).
 *
 * Upstream's structure is reproduced:
 *
 * ```
 * <ComposerRoot>            // ComposerPrimitive.Root — a <form>
 *   <ComposerAttachments/>  // attachment strip
 *   <ComposerAddAttachment/>
 *   <ComposerInput rows={1}/>
 *   <ComposerAction/>       // send, or cancel while running
 * </ComposerRoot>
 * ```
 *
 * ## What comes from the primitive, and what deliberately does not
 *
 * From the primitive: the `<form>` wrapper (blank-space click focuses the
 * input, Enter routes through `requestSubmit`), the auto-resizing textarea with
 * its composition/IME tracking, Escape-to-cancel, and the composer text store
 * that `useComposerTextBridge` mirrors this app's `inputValue` into.
 *
 * **Not** from the primitive: sending. `ComposerPrimitive.Send` and
 * `ComposerPrimitive.Cancel` fire `aui.composer.send()` / `.cancel()`, which the
 * external-store runtime forwards to `chatSurfaceHandlers` as
 * `send(text: string)`. That signature cannot carry this composer's
 * attachments, and it knows nothing about the OpenHuman-specific modifier
 * semantics `allowParallelSend` implements (plain Enter queues a follow-up
 * mid-stream; Cmd/Ctrl+Enter forks a parallel branch) — routing a send through
 * it would silently drop attachments and collapse the two modifiers into one.
 * So the action buttons stay this app's, and `Root`'s `onSubmit` is intercepted
 * with `preventDefault()`, which (via Radix's `composeEventHandlers`) suppresses
 * the primitive's own submit handler. Enter therefore still reaches
 * `handleInputKeyDown`, which owns the modifier semantics.
 *
 * `ComposerPrimitive.AddAttachment` and `.Attachments` are likewise unused:
 * both require an attachment adapter on the runtime, and this app's attachments
 * are read, validated, compressed and budgeted by the host surface long before
 * the composer sees them. The upstream *look* of both is still applied.
 *
 * Similarly, upstream's `ComposerAction` morphs send/cancel off
 * `ThreadPrimitive.If running`. Here the same decision reads `isSending` — not
 * a different source of truth: the runtime adapter's `isRunning` is itself
 * derived from `chatRuntime.inferenceTurnLifecycleByThread`, the very state the
 * host surface passes down as `isSending`. Reading it directly keeps the
 * composer renderable by surfaces whose thread is not the runtime's.
 *
 * ## Runtime context
 *
 * `ComposerPrimitive.*` reads its runtime from React context, so every surface
 * rendering this component should sit under a runtime scoped to *its own*
 * thread. Two did not, and would have inherited the app-wide one bound to
 * `selectedThreadId` — the home chat's thread; `WorkflowCopilotPanel` now
 * mounts its own. The default
 * export wraps this body in `ComposerRuntimeBoundary`, which supplies an inert
 * runtime when there is none at all; read that file before assuming the
 * boundary makes the per-surface decision unnecessary — it cannot, because an
 * inherited runtime and a correct one are indistinguishable from in here.
 */
function ChatComposerBody({
  inputValue,
  setInputValue,
  onSend,
  onStopGeneration,
  onOpenHumanMode,
  textInputRef,
  fileInputRef,
  composerInteractionBlocked,
  isSending,
  allowParallelSend = false,
  attachments,
  onAttachFiles,
  onRemoveAttachment,
  attachError: _attachError,
  onSwitchToMicCloud,
  handleInputKeyDown,
  inlineCompletionSuffix,
  isComposingTextRef,
  maxAttachments,
  allowedMimeTypes,
  attachmentsEnabled,
  micEnabled = true,
  placeholder,
  headerSlots = [],
  mascotDock,
  modelOverride,
  onModelOverrideChange,
}: ChatComposerProps) {
  const { t } = useT();
  const [isDragging, setIsDragging] = useState(false);

  useComposerTextBridge(inputValue);

  // Render-loop detection guard: tracks re-renders within a single microtick
  // and warns when the count exceeds a safe threshold, helping diagnose
  // "Maximum update depth exceeded" issues (TAURI-REACT-2G).
  const renderCountRef = useRef(0);
  const renderTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  renderCountRef.current += 1;
  if (renderCountRef.current >= RENDER_LOOP_WARN_THRESHOLD) {
    console.warn(
      `[ChatComposer] Render-loop detected: ${renderCountRef.current} renders in a single tick. ` +
        'Check whether a parent onChange/setState cycle is causing cascading re-renders.'
    );
  }
  if (renderTimerRef.current === null) {
    renderTimerRef.current = setTimeout(() => {
      renderCountRef.current = 0;
      renderTimerRef.current = null;
    }, 0);
  }

  // While a turn streams (`allowParallelSend`) the composer stays usable for a
  // queued follow-up / parallel branch, so the in-flight `isSending` spinner
  // and lock no longer apply — only real typed content gates the send button.
  const hasContent =
    inputValue.trim().length > 0 || attachments.length > 0 || (isSending && !allowParallelSend);
  // The textarea (and send button) stay editable while a turn streams so the
  // user can queue a follow-up or fork a parallel branch; otherwise an in-flight
  // turn (`composerInteractionBlocked`/`isSending`) locks the composer.
  const composerLocked = !allowParallelSend && (composerInteractionBlocked || isSending);
  // Show the working spinner only for a normal in-flight send, not while the
  // composer is intentionally open for follow-up/parallel queueing.
  const showSendingSpinner = isSending && !allowParallelSend;
  // During an in-flight turn the primary button becomes a Stop button so the
  // user can halt generation from the composer — but only while no follow-up is
  // typed. Once they type (parallel/follow-up send), it reverts to Send so the
  // follow-up can be queued instead of cancelling the current turn.
  const hasTypedContent = inputValue.trim().length > 0 || attachments.length > 0;
  const showStopButton = isSending && !!onStopGeneration && !hasTypedContent;
  // Empty composer, nothing in flight: the primary slot offers the Human page
  // instead of a dead disabled arrow. Stop and the in-flight spinner both win
  // over it — a turn is still the thing the button is about while one is
  // running — and the first typed character hands the slot back to Send.
  const showHumanModeButton =
    !!onOpenHumanMode && !hasTypedContent && !showStopButton && !showSendingSpinner;

  // Attachment ingest is blocked while the feature is off, the composer is
  // locked, or the budget is full — drag-drop and paste honour the same gate as
  // the [+] button so they can't bypass it.
  const attachDisabled =
    !attachmentsEnabled ||
    composerInteractionBlocked ||
    isSending ||
    attachments.length >= maxAttachments;

  // Drag-and-drop: route dropped files through the same handler as the picker.
  // We always `preventDefault` for *file* drags so the browser/CEF never
  // navigates away to the dropped file — even when ingest is disabled — and only
  // attach (and show the overlay) when ingest is actually allowed.
  const isFileDrag = (e: React.DragEvent) =>
    Array.from(e.dataTransfer?.types ?? []).includes('Files');
  const handleDragOver = (e: React.DragEvent) => {
    if (!isFileDrag(e)) return;
    e.preventDefault();
    if (!attachDisabled) setIsDragging(true);
  };
  const handleDragLeave = (e: React.DragEvent) => {
    // Ignore leave events that bubble while the cursor is still over a child.
    if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
    setIsDragging(false);
  };
  const handleDrop = (e: React.DragEvent) => {
    if (!isFileDrag(e)) return;
    // Stop the default file-navigation before any early-return below.
    e.preventDefault();
    setIsDragging(false);
    if (attachDisabled) return;
    const files = e.dataTransfer?.files;
    if (!files || files.length === 0) return;
    debug('[chat-composer] drop: ingesting %d file(s)', files.length);
    void onAttachFiles(files);
  };

  // Clipboard paste: pull image/video files out of the paste payload (e.g. a
  // screenshot copied to the clipboard) and attach them; leave plain-text paste
  // untouched so normal typing still works. `addAttachmentOnPaste={false}` on
  // the primitive keeps assistant-ui's own paste-to-attach path out of the way —
  // it would route files to a runtime attachment adapter this app does not use.
  const handlePaste = (e: React.ClipboardEvent<HTMLTextAreaElement>) => {
    if (attachDisabled) return;
    const items = Array.from(e.clipboardData?.items ?? []);
    const files = items
      .filter(item => item.kind === 'file' && /^(image|video)\//.test(item.type))
      .map(item => item.getAsFile())
      .filter((file): file is File => file !== null);
    if (files.length === 0) return;
    e.preventDefault();
    debug('[chat-composer] paste: ingesting %d media file(s)', files.length);
    void onAttachFiles(files);
  };

  /**
   * `preventDefault` is load-bearing, not ceremony: Radix's
   * `composeEventHandlers` runs this handler first and skips the primitive's own
   * submit handler when the default was prevented. Without it a form submit
   * would ALSO fire `aui.composer.send()`, double-sending the turn through a
   * path that drops attachments.
   */
  const handleSubmit = (e: React.FormEvent<HTMLFormElement>) => {
    e.preventDefault();
    if (!hasContent || composerLocked) return;
    debug('[chat-composer] submit: parallel=%s sending=%s', allowParallelSend, isSending);
    void onSend();
  };

  return (
    <ComposerPrimitive.Root className={COMPOSER_ROOT} onSubmit={handleSubmit}>
      {/* Header stack (e.g. queued follow-ups, thread-goal editor): rendered
          above the input box and OUTSIDE its focus highlight, but still within
          the overall composer so they move as one unit. */}
      {headerSlots}

      {/* `.aui-composer-attachment-dropzone` — the only element carrying the
          focus-within highlight and the drag state. */}
      <div
        className={COMPOSER_DROPZONE}
        data-dragging={isDragging ? 'true' : undefined}
        onDragOver={handleDragOver}
        onDragLeave={handleDragLeave}
        onDrop={handleDrop}>
        {/* Anchored to the input box (not the header stack) so it stands on the
            box's top edge regardless of whether follow-ups / the goal editor
            are open above it. */}
        {mascotDock}

        {isDragging && (
          <div className={COMPOSER_DRAG_OVERLAY}>{t('chat.attachment.dropToAttach')}</div>
        )}

        {/* Hidden file input for attachment (gated — see attachmentsEnabled). */}
        {attachmentsEnabled && (
          <input
            ref={fileInputRef}
            type="file"
            // No `accept` filter: Chromium 146 / CEF on macOS greys out valid files
            // at the native open panel regardless of the filter shape (MIME, mixed,
            // or extension-only). Selection is gated in `onAttachFiles` →
            // `validateAndReadFile` instead (rejects unsupported types + images on
            // non-vision models). `allowedMimeTypes` is kept for that JS validation.
            accept={allowedMimeTypes.length ? allowedMimeTypes.join(',') : undefined}
            multiple
            className="hidden"
            onChange={e => {
              void onAttachFiles(e.target.files);
              e.target.value = '';
            }}
          />
        )}

        {/* `.aui-composer-attachments` */}
        {attachmentsEnabled && attachments.length > 0 && (
          <div className={COMPOSER_ATTACHMENTS}>
            <AttachmentPreview
              attachments={attachments}
              onRemove={onRemoveAttachment}
              disabled={composerInteractionBlocked || isSending}
            />
          </div>
        )}

        {/* `.aui-composer-input`, with the inline-completion ghost layer behind
            it. The ghost is positioned against this wrapper and repeats the
            input's own padding so the suffix lands where the caret is. */}
        <div className="relative">
          <div aria-hidden className={COMPOSER_GHOST_OVERLAY}>
            <span className="invisible">{inputValue}</span>
            <span className="text-content-muted dark:text-content-muted/50">
              {inlineCompletionSuffix}
            </span>
          </div>
          <ComposerPrimitive.Input
            ref={textInputRef}
            rows={1}
            className={`relative z-10 ${COMPOSER_INPUT}`}
            placeholder={
              placeholder ?? (allowParallelSend ? t('chat.followupHint') : t('chat.typeMessage'))
            }
            disabled={composerLocked}
            // Enter/modifier semantics are entirely OpenHuman's. `submitMode`
            // is NOT cosmetic here: assistant-ui's own Enter handler runs after
            // ours (Radix composes them) and fires whenever ours merely
            // *declines* to act rather than calling `preventDefault` — which is
            // exactly what `handleInputKeyDown` does mid-IME-composition, where
            // the keydown carries no `isComposing` flag for the primitive to
            // notice either (legacy keyCode 229, and Korean/Japanese IMEs that
            // omit the flag). Leaving it on made the composer send a
            // half-composed word. `"none"` disables the primitive's keyboard
            // submission outright, so Enter reaches only `handleInputKeyDown`
            // and an unhandled Enter inserts a newline, as it did before.
            submitMode="none"
            onKeyDown={handleInputKeyDown}
            onChange={e => setInputValue(e.target.value)}
            onCompositionStart={() => {
              isComposingTextRef.current = true;
            }}
            onCompositionEnd={() => {
              isComposingTextRef.current = false;
            }}
            onPaste={attachmentsEnabled ? handlePaste : undefined}
            addAttachmentOnPaste={false}
          />
        </div>

        {/* `.aui-composer-action-wrapper` — add-attachment and the model chip
            on the left; the mic and the send/cancel action on the right. The
            left group is `min-w-0` so the model chip (the only element with a
            text label, and therefore the only one that can grow) truncates
            instead of pushing the right group off the edge at narrow widths. */}
        <div className={COMPOSER_ACTION_WRAPPER}>
          <div className="flex min-w-0 items-center gap-1">
            {attachmentsEnabled && (
              <Button
                type="button"
                iconOnly
                variant="tertiary"
                size="xs"
                analyticsId="chat-composer-attach-file"
                aria-label={t('composer.attachFile')}
                title={t('composer.attachFile')}
                onClick={() => fileInputRef.current?.click()}
                disabled={
                  composerInteractionBlocked || isSending || attachments.length >= maxAttachments
                }
                className={COMPOSER_ADD_ATTACHMENT}>
                <AddAttachmentIcon />
              </Button>
            )}

            {/* Read-only model chip. Carries the only text label in the row, so
                it is the element that must give way when space runs out — see
                its own `min-w-0` + truncating name span. */}
            <ModelQualityPill
              className="min-w-0"
              value={modelOverride}
              onValueChange={onModelOverrideChange}
            />
          </div>

          {/* Right group — mic, then the primary send/stop action. */}
          <div className="flex shrink-0 items-center gap-1">
            {micEnabled && (
              <Button
                type="button"
                iconOnly
                variant="tertiary"
                size="xs"
                analyticsId="chat-composer-voice-mode"
                aria-label={t('composer.voiceMode')}
                title={t('composer.voiceMode')}
                onClick={onSwitchToMicCloud}
                disabled={composerInteractionBlocked || isSending}
                className={COMPOSER_MIC}>
                <MicIcon />
              </Button>
            )}

            {/* Human / Send / Stop — upstream's `ComposerAction`, with one extra
                state. Empty and idle, the slot is the Human-page shortcut (see
                `showHumanModeButton`). While a turn is in flight and a cancel
                handler is wired, Send becomes Stop so generation can be halted
                from inside the composer. Once a follow-up is typed the Send
                arrow returns so the follow-up can be queued (parallel send)
                instead of cancelling the current turn. */}
            {showHumanModeButton ? (
              <Button
                type="button"
                iconOnly
                variant="tertiary"
                size="xs"
                analyticsId="chat-composer-human-mode"
                data-testid="human-mode-button"
                aria-label={t('composer.humanMode')}
                title={t('composer.humanMode')}
                onClick={() => {
                  debug('[chat-composer] human-mode click');
                  onOpenHumanMode?.();
                }}
                className={COMPOSER_SEND}>
                <HumanModeIcon />
              </Button>
            ) : showStopButton ? (
              <Button
                type="button"
                iconOnly
                variant="primary"
                size="xs"
                analyticsId="chat-composer-stop"
                data-testid="stop-generation-button"
                aria-label={t('chat.stopGeneration')}
                title={t('chat.stopGeneration')}
                onClick={onStopGeneration}
                className={COMPOSER_SEND}>
                <StopIcon />
              </Button>
            ) : (
              <Button
                type="button"
                iconOnly
                variant="primary"
                size="xs"
                analyticsId="chat-composer-send"
                data-testid="send-message-button"
                aria-label={t('chat.send')}
                title={t('chat.send')}
                onClick={() => {
                  debug('[chat-composer] send click: parallel=%s', allowParallelSend);
                  void onSend();
                }}
                disabled={!hasContent || composerLocked}
                className={COMPOSER_SEND}>
                {showSendingSpinner ? <SendingSpinnerIcon /> : <SendIcon />}
              </Button>
            )}
          </div>
        </div>
      </div>
    </ComposerPrimitive.Root>
  );
}

/**
 * Public entry point. The boundary guarantees a runtime is present before any
 * `ComposerPrimitive` renders — see `ComposerRuntimeBoundary` for why that is a
 * net under the per-surface scoping decision rather than a substitute for it.
 */
export default function ChatComposer(props: ChatComposerProps) {
  return (
    <ComposerRuntimeBoundary>
      <ChatComposerBody {...props} />
    </ComposerRuntimeBoundary>
  );
}
