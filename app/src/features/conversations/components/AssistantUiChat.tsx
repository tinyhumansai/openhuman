import { ConversationMapAui } from '@/components/assistant-ui/elements/conversation-map.aui';
import { Thread, type ThreadComponents } from '@/components/assistant-ui/thread';
import { type AssistantState, useAui, useAuiState } from '@assistant-ui/react';
import { PlusIcon } from 'lucide-react';
import { type ReactNode, startTransition, useCallback, useEffect, useMemo, useRef } from 'react';

import { COMPOSER_HUMAN_MASCOT_DATA_URL } from '../../../assets/composerHumanMascot';
import AttachmentPreview from '../../../components/chat/AttachmentPreview';
import { Button } from '../../../components/ui';
import type { Attachment } from '../../../lib/attachments';
import { useT } from '../../../lib/i18n/I18nContext';
import { AssistantUiRuntimeProvider } from '../../../providers/AssistantUiRuntimeProvider';
import { useAppSelector } from '../../../store/hooks';
import { AgentRunningStatus } from '../aui/AgentRunningStatus';
import { ChatConversationMap } from '../aui/ChatConversationMap';
import { ComposerTriggers } from '../aui/ComposerTriggers';
import { ContextUsage } from '../aui/ContextUsage';
import { InterruptedTurnNotice } from '../aui/InterruptedTurnNotice';
import { MessageArtifactCards } from '../aui/MessageArtifactCards';
import { ActiveTurnTaskCards, TurnTaskCards, TurnTaskProvider } from '../aui/TurnTaskCards';
import { ChatSources } from './aui/ChatSources';
import { ChatToolFallback } from './ChatToolParts';

const selectComposerText = (state: AssistantState) => state.composer.text;

/** Keep the map on the former Timeline button's edge, with previews opening inward. */
const ChatConversationMapRail = () => <ConversationMapAui side="right" />;

/** Fixed Human-mode portrait supplied for the empty composer's primary action. */
function ComposerHumanMascotIcon() {
  return (
    <img
      data-testid="composer-human-mascot-icon"
      width="24"
      height="24"
      src={COMPOSER_HUMAN_MASCOT_DATA_URL}
      alt=""
      className="rounded-full object-cover"
      aria-hidden="true"
    />
  );
}

/**
 * Keep the host's draft (`inputValue`) and assistant-ui's composer text in step.
 *
 * Two directions, and the one that fires on every keystroke is the dangerous
 * one. Editor → host used to call `onChange` (a `useState` setter) inside this
 * effect synchronously. A keystroke is a discrete event, so React flushes the
 * effect — and the update it schedules — synchronously too, and a burst of
 * keystrokes (key-repeat, fast typing, an automated driver) chained those sync
 * updates past React's nested-update limit: "Maximum update depth exceeded",
 * and the whole chat surface fell to the error boundary.
 *
 * So editor → host now runs as a transition, which is not a sync update. The
 * host value then lags the editor by a render, and the other direction must
 * not mistake that lag for a host write: a host value that is one we emitted
 * ourselves is an echo and is never written back into the editor (doing so
 * would overwrite what was typed since). Only a value we did not emit —
 * dictation, ESC restore, clear after send — is a host write, and it wins.
 */
export function ComposerTextBridge({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  const aui = useAui();
  const composerText = useAuiState(selectComposerText);
  const previousHostValue = useRef(value);
  // Editor texts sent to the host that it has not echoed back yet.
  const inFlight = useRef<string[]>([]);

  useEffect(() => {
    if (previousHostValue.current !== value) {
      previousHostValue.current = value;
      const echoAt = inFlight.current.indexOf(value);
      if (echoAt >= 0) {
        // Our own write coming back (possibly behind newer typing): drop it and
        // everything sent before it; the editor already holds newer text.
        inFlight.current = inFlight.current.slice(echoAt + 1);
        // Do not return: the editor may already contain text typed after this
        // echo. Let the normal editor-to-host comparison forward it now.
      } else {
        // A host-side write (dictation, ESC restore, clear) wins for this pass.
        inFlight.current = [];
        if (composerText !== value) aui.composer.setText(value);
        return;
      }
    }
    // Otherwise the editor changed and the host draft follows it, once per text.
    if (composerText === value || inFlight.current.at(-1) === composerText) return;
    inFlight.current.push(composerText);
    startTransition(() => onChange(composerText));
  }, [aui, composerText, onChange, value]);

  return null;
}

/**
 * The assistant-ui `Thread`, projected from OpenHuman's Redux transcript.
 *
 * The library runtime manages UI interaction; Redux and the core remain
 * authoritative for messages, streaming and persistence. Composer sends go through
 * the chat-surface registration owned by `Conversations`, so this uses the same
 * send/cancel path as the legacy composer.
 */
export function AssistantUiChat({
  model,
  modelContextWindow,
  onModelChange,
  composerHeader,
  composerFooterExtras,
  composerReplacement,
  afterMessages,
  inputValue,
  onInputValueChange,
  onEscape,
  onRecallLastPrompt,
  composerPlaceholder,
  attachments,
  onAttachFiles,
  onRemoveAttachment,
  maxAttachments,
  attachmentsEnabled,
  attachmentInteractionBlocked,
  onAttachmentOnlySend,
  onOpenHumanMode,
  onSwitchToMicCloud,
}: {
  model: string | null;
  modelContextWindow?: number | null;
  onModelChange: (value: string | null, contextWindow?: number | null) => void;
  composerHeader?: ReactNode;
  /**
   * Host controls for the composer's own toolbar row, beside the model pill —
   * the assistant-ui equivalent of the legacy panel's footer row (the
   * background-processes button and the thread files chip).
   */
  composerFooterExtras?: ReactNode;
  /**
   * Replaces the built-in text composer entirely, keeping the assistant-ui
   * transcript above it. The mic-first voice composer (`mic-cloud`) uses this:
   * its input is a push-to-talk button, not a text box. `undefined` keeps the
   * normal composer.
   */
  composerReplacement?: ReactNode;
  afterMessages?: ReactNode;
  inputValue: string;
  onInputValueChange: (value: string) => void;
  /** Returns whether Escape acted (stopped a run); `false` lets the key through. */
  onEscape?: () => boolean | void;
  /** ArrowUp in an empty composer: recall the last prompt; returns whether it did. */
  onRecallLastPrompt?: () => boolean;
  /** State-aware placeholder (running / waiting on the user); default otherwise. */
  composerPlaceholder?: string;
  attachments: Attachment[];
  onAttachFiles: (files: FileList | File[] | null) => Promise<void>;
  onRemoveAttachment: (id: string) => void;
  maxAttachments: number;
  attachmentsEnabled: boolean;
  attachmentInteractionBlocked: boolean;
  onAttachmentOnlySend: () => void;
  /** Opens the Human page from the composer's idle primary slot. */
  onOpenHumanMode?: () => void;
  /** Switches to the existing microphone-first chat composer. */
  onSwitchToMicCloud?: () => void;
}) {
  const { t } = useT();
  const fileInputRef = useRef<HTMLInputElement>(null);
  // The idle composer button wears the user's own mascot (yellow by default),
  // so the control looks like the thing it opens rather than a generic glyph.
  //
  // Read defensively rather than through `selectMascotColor` /
  // `selectCustomPrimaryColor`: this component is mounted by suites that build
  // a partial store, and those selectors dereference `state.mascot` unguarded,
  // so a store without the slice crashes the whole chat surface on render.
  const selectedThreadId = useAppSelector(state => state.thread.selectedThreadId);
  const loadError = useAppSelector(state => state.thread.messagesError);

  // Every prop the composer slots below read, refreshed on each host render.
  //
  // Each of those slots is rendered by type (see the `ComposerHeader` note
  // below), so none of them may close over a prop: a dep list that changes
  // hands React a new element type and remounts the subtree. Reading through
  // this ref lets all of them take `[]` deps and keep a constant identity,
  // while still seeing current values — `thread.tsx` memoizes nothing, so a
  // slot re-renders with its host.
  const slotPropsRef = useRef({
    attachments,
    attachmentInteractionBlocked,
    maxAttachments,
    modelContextWindow,
    onAttachFiles,
    onOpenHumanMode,
    onRemoveAttachment,
    selectedThreadId,
  });
  slotPropsRef.current = {
    attachments,
    attachmentInteractionBlocked,
    maxAttachments,
    modelContextWindow,
    onAttachFiles,
    onOpenHumanMode,
    onRemoveAttachment,
    selectedThreadId,
  };
  // Read through a ref for the same reason `ComposerHeader` does below: the
  // slot is rendered by type, so closing over the node would remount the whole
  // row on every host render.
  const composerFooterExtrasRef = useRef(composerFooterExtras);
  composerFooterExtrasRef.current = composerFooterExtras;
  const ComposerExtras = useCallback(() => <>{composerFooterExtrasRef.current}</>, []);
  const ComposerRightExtras = useCallback(() => {
    const { modelContextWindow, selectedThreadId } = slotPropsRef.current;
    return <ContextUsage threadId={selectedThreadId} modelContextWindow={modelContextWindow} />;
  }, []);
  // Stable component identity, latest node read through a ref.
  //
  // `<ComposerHeader />` is rendered by type (`thread.tsx:489`), so a callback
  // that closes over `composerHeader` gives React a NEW type on every host
  // render — the whole header subtree unmounts and remounts. That was invisible
  // while the header only held an error string and the queued-followup strip,
  // but the turn-gate cards it now carries (`PlanReviewCard`,
  // `WorkflowProposalCard`) own local state: half-typed plan feedback and an
  // in-flight "Save & enable" would be wiped by any unrelated re-render, e.g.
  // a keystroke in the composer. Nothing in `thread.tsx` is memoized, so this
  // subtree re-renders with its host and the ref is always current.
  const composerHeaderRef = useRef(composerHeader);
  composerHeaderRef.current = composerHeader;
  const ComposerHeader = useCallback(() => <>{composerHeaderRef.current}</>, []);
  // Same stable-type-through-a-ref pattern: the voice composer owns recording
  // state (MicComposer) that a remount would drop mid-utterance.
  const composerReplacementRef = useRef(composerReplacement);
  composerReplacementRef.current = composerReplacement;
  const hasComposerReplacement = composerReplacement !== undefined;
  const ComposerReplacement = useCallback(() => <>{composerReplacementRef.current}</>, []);
  // Artifact cards from versioned tools render as part of the transcript
  // (after the last message), not in the composer header — the cards are
  // messages' content, not attachments. Stable type through a ref, same
  // pattern as composerHeader above.
  const afterMessagesRef = useRef(afterMessages);
  afterMessagesRef.current = afterMessages;
  const AfterMessages = useCallback(() => <>{afterMessagesRef.current}</>, []);
  const ComposerAttachments = useCallback(() => {
    const { attachments, attachmentInteractionBlocked, onRemoveAttachment } = slotPropsRef.current;
    return (
      <AttachmentPreview
        attachments={attachments}
        onRemove={onRemoveAttachment}
        disabled={attachmentInteractionBlocked}
      />
    );
  }, []);
  // The hidden input must survive a host re-render: it is the element the
  // native file dialog is attached to, and a remount while that dialog is open
  // detaches it, so its `change` never reaches React's delegated listener and
  // the picked file is dropped with no error (#6246).
  const ComposerAddAttachment = useCallback(() => {
    const { attachmentInteractionBlocked, attachments, maxAttachments } = slotPropsRef.current;
    return (
      <>
        <input
          ref={fileInputRef}
          type="file"
          multiple
          className="hidden"
          onChange={event => {
            void slotPropsRef.current.onAttachFiles(event.target.files);
            event.target.value = '';
          }}
        />
        <Button
          type="button"
          iconOnly
          variant="tertiary"
          size="xs"
          aria-label={t('composer.attachFile')}
          title={t('composer.attachFile')}
          disabled={attachmentInteractionBlocked || attachments.length >= maxAttachments}
          onClick={() => fileInputRef.current?.click()}>
          <PlusIcon className="h-4 w-4" />
        </Button>
      </>
    );
  }, [t]);
  /**
   * Primary-slot control for an empty composer: a circular button carrying the
   * user's mascot, opening the Human page. Same 28px circle as the Send button
   * it stands in for, so the row's metrics don't shift when a character is
   * typed; the avatar is inset a little so the mascot reads inside the circle
   * rather than filling it edge to edge.
   */
  const ComposerIdleAction = useCallback(() => {
    const { onOpenHumanMode } = slotPropsRef.current;
    return onOpenHumanMode ? (
      <Button
        type="button"
        iconOnly
        variant="secondary"
        size="xs"
        analyticsId="chat-composer-human-mode"
        data-testid="composer-human-mode"
        aria-label={t('composer.humanMode')}
        title={t('composer.humanMode')}
        className="size-7 shrink-0 rounded-full p-0"
        onClick={onOpenHumanMode}>
        <ComposerHumanMascotIcon />
      </Button>
    ) : null;
  }, [t]);

  // Files arriving from a drop or a paste, routed to the same host validator
  // the picker uses. Stable like the slots above, and for the same reason: it
  // is handed to `thread.tsx` through the components object.
  const handleComposerFiles = useCallback((files: FileList | File[] | null) => {
    return slotPropsRef.current.onAttachFiles(files);
  }, []);

  const components: ThreadComponents = useMemo(
    () => ({
      ToolFallback: ChatToolFallback,
      MessageTasks: TurnTaskCards,
      ActiveTasks: ActiveTurnTaskCards,
      // `/` commands (builtins + core `commands_list` + registry actions) and
      // `@` mentions (memory recall, thread files); see `aui/ComposerTriggers`.
      ComposerTriggers,
      ConversationMap: ChatConversationMapRail,
      ComposerExtras,
      ComposerRightExtras,
      ComposerHeader,
      ComposerIdleAction,
      // Phase / reasoning round / active tool for the turn in flight. Reads the
      // runtime's `extras`, so it needs no props and no dependency here.
      RunningStatus: AgentRunningStatus,
      // A crashed turn's partial reply and its "Interrupted" marker.
      TranscriptFooter: InterruptedTurnNotice,
      // Versioned-artifact cards live in the transcript after the last
      // message (see `AfterMessages` slot in thread.tsx).
      AfterMessages,
      // Artifact cards render INSIDE the message that produced them
      // (matched by requestId), not just at the end of the transcript.
      // Pass the component itself: an inline arrow is a new component type
      // each time this memo recomputes (e.g. when a turn starts or ends), which
      // remounts every card and replays its fade-in, so the cards flicker.
      MessageArtifacts: MessageArtifactCards,
      // The web pages the turn fetched, grouped from its `source` parts into
      // one collapsed disclosure under the answer.
      SourceGroup: ChatSources,
      onSwitchToMicCloud,
      ...(hasComposerReplacement ? { Composer: ComposerReplacement } : {}),
      ...(attachmentsEnabled
        ? {
            ComposerAttachments,
            ComposerAddAttachment,
            hasComposerAttachments: attachments.length > 0,
            onComposerAttachmentSend: onAttachmentOnlySend,
            // Drop and paste reach the same validator as the picker. Without
            // this the assistant surface had no host file path at all: its only
            // dropzone was the assistant-ui primitive, which hands files to a
            // runtime attachment adapter this app does not use.
            onComposerFiles: handleComposerFiles,
            canAcceptComposerFiles:
              !attachmentInteractionBlocked && attachments.length < maxAttachments,
          }
        : {}),
    }),
    [
      ComposerAddAttachment,
      ComposerAttachments,
      ComposerExtras,
      ComposerRightExtras,
      ComposerHeader,
      ComposerIdleAction,
      ComposerReplacement,
      hasComposerReplacement,
      attachmentInteractionBlocked,
      handleComposerFiles,
      maxAttachments,
      // The array itself, not just its length: the slot components above hold a
      // constant identity now, so this memo is what makes `thread.tsx`'s
      // context change and re-render them against the latest attachments.
      attachments,
      attachmentsEnabled,
      onAttachmentOnlySend,
      onSwitchToMicCloud,
    ]
  );

  return (
    <AssistantUiRuntimeProvider>
      <ComposerTextBridge value={inputValue} onChange={onInputValueChange} />
      <TurnTaskProvider>
        <ChatConversationMap>
          <Thread
            components={components}
            model={model}
            onModelChange={onModelChange}
            loadError={loadError}
            onEscape={onEscape}
            onRecallLastPrompt={onRecallLastPrompt}
            composerPlaceholder={composerPlaceholder}
          />
        </ChatConversationMap>
      </TurnTaskProvider>
    </AssistantUiRuntimeProvider>
  );
}

export default AssistantUiChat;
