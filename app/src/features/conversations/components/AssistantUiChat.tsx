import { Thread, type ThreadComponents } from '@/components/assistant-ui/thread';
import { type AssistantState, useAui, useAuiState } from '@assistant-ui/react';
import { PlusIcon } from 'lucide-react';
import { type ReactNode, useCallback, useEffect, useMemo, useRef } from 'react';

import AttachmentPreview from '../../../components/chat/AttachmentPreview';
import { Button } from '../../../components/ui';
import type { Attachment } from '../../../lib/attachments';
import { useRegisterAction } from '../../../lib/commands/useRegisterAction';
import { useSlashCommands } from '../../../lib/commands/useSlashCommands';
import { useT } from '../../../lib/i18n/I18nContext';
import type { TurnProcessTrail } from '../../../providers/assistantUiMessages';
import { AssistantUiRuntimeProvider } from '../../../providers/AssistantUiRuntimeProvider';
import { emptySessionTokenUsage } from '../../../store/chatRuntimeSlice';
import { useAppSelector } from '../../../store/hooks';
import { DEFAULT_MASCOT_COLOR } from '../../../store/mascotSlice';
import { MascotChipAvatar } from '../../human/Mascot/MascotChipAvatar';
import { AssistantUiInferenceStatus } from './AssistantUiInferenceStatus';
import { SubagentDrawerHost } from './aui/subagentDrawerHost';
import { TurnFooter } from './aui/TurnFooter';
import { TurnFooterHost } from './aui/turnFooterHost';
import { ChatToolFallback, ChatToolGroup } from './ChatToolParts';
import { contextUsageFromTokenUsage, ContextWindowPill } from './composer/ContextWindowPill';
import {
  type ThreadGoalController,
  ThreadGoalEditorPanel,
  ThreadGoalFooterTrigger,
} from './ThreadGoalChip';

const EMPTY_TOKEN_USAGE = emptySessionTokenUsage();
const selectComposerText = (state: AssistantState) => state.composer.text;

function ComposerTextBridge({
  value,
  onChange,
}: {
  value: string;
  onChange: (value: string) => void;
}) {
  const aui = useAui();
  const composerText = useAuiState(selectComposerText);
  const previousHostValue = useRef(value);

  useEffect(() => {
    // A host-side write (dictation, ESC restore, clear) wins for this pass.
    if (previousHostValue.current !== value) {
      previousHostValue.current = value;
      if (composerText !== value) aui.composer.setText(value);
      return;
    }
    // Otherwise the editor changed and the host draft follows it.
    if (composerText !== value) onChange(composerText);
  }, [aui, composerText, onChange, value]);

  return null;
}

/**
 * The assistant-ui `Thread`, projected from OpenHuman's Redux transcript.
 *
 * The runtime is a read-only projection; Redux and the core remain authoritative
 * for messages, streaming and persistence. Composer sends are forwarded through
 * the chat-surface registration owned by `Conversations`, so this uses the same
 * send/cancel path as the legacy composer.
 */
export function AssistantUiChat({
  threadGoal,
  model,
  modelContextWindow,
  onModelChange,
  composerHeader,
  composerFooterExtras,
  inputValue,
  onInputValueChange,
  onEscape,
  attachments,
  onAttachFiles,
  onRemoveAttachment,
  maxAttachments,
  attachmentsEnabled,
  attachmentInteractionBlocked,
  onAttachmentOnlySend,
  onOpenHumanMode,
  onSwitchToMicCloud,
  onOpenSubagent,
  canOpenSubagent,
  onOpenTurnProcess,
}: {
  threadGoal: ThreadGoalController;
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
  inputValue: string;
  onInputValueChange: (value: string) => void;
  onEscape?: () => void;
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
  /**
   * Opens the host's `SubagentDrawer` on a delegation, by spawn `taskId`.
   * Handed down by context rather than by prop because the caller is a tool
   * part rendered from inside the transcript; see `subagentDrawerHost`.
   */
  onOpenSubagent?: (taskId: string) => void;
  /** Whether the host's drawer can resolve that delegation; see the same file. */
  canOpenSubagent?: (taskId: string) => boolean;
  /** Opens the host's process rail on one settled turn's trail (`TurnFooter`). */
  onOpenTurnProcess?: (trail: TurnProcessTrail) => void;
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
  // `ChatThreadView` reads `state.theme?.` the same way for the same reason.
  const mascotColor = useAppSelector(state => state.mascot?.color ?? DEFAULT_MASCOT_COLOR);
  const mascotCustomPrimary = useAppSelector(state => state.mascot?.customPrimaryColor ?? null);
  const selectedThreadId = useAppSelector(state => state.thread.selectedThreadId);
  const loadError = useAppSelector(state => state.thread.messagesError);
  const tokenUsage = useAppSelector(state =>
    selectedThreadId
      ? (state.chatRuntime.usageByThread[selectedThreadId] ?? EMPTY_TOKEN_USAGE)
      : EMPTY_TOKEN_USAGE
  );
  const contextUsage = useMemo(
    () => contextUsageFromTokenUsage(tokenUsage, modelContextWindow),
    [modelContextWindow, tokenUsage]
  );
  const openThreadGoal = threadGoal.open;

  useRegisterAction({
    id: 'chat.goal',
    label: 'Set thread goal',
    labelKey: 'conversations.composer.command.goal',
    group: 'Chat',
    handler: openThreadGoal,
    enabled: () => selectedThreadId !== null,
    keywords: ['goal', 'objective', 'thread goal'],
    slashCommand: { id: 'goal', descriptionKey: 'conversations.composer.command.goal' },
  });
  const slashCommands = useSlashCommands();

  // Read through a ref for the same reason `ComposerHeader` does below: the
  // slot is rendered by type, so closing over the node would remount the whole
  // row on every host render.
  const composerFooterExtrasRef = useRef(composerFooterExtras);
  composerFooterExtrasRef.current = composerFooterExtras;
  const ComposerExtras = useCallback(
    () => (
      <>
        <ContextWindowPill usage={contextUsage} />
        <div className="absolute right-0 bottom-full left-0 pb-2">
          <ThreadGoalEditorPanel ctl={threadGoal} />
        </div>
        <ThreadGoalFooterTrigger ctl={threadGoal} />
        {composerFooterExtrasRef.current}
      </>
    ),
    [contextUsage, threadGoal]
  );
  // Stable component identity, latest node read through a ref.
  //
  // `<ComposerHeader />` is rendered by type (`thread.tsx:385`), so a callback
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
  const ComposerAttachments = useCallback(
    () => (
      <AttachmentPreview
        attachments={attachments}
        onRemove={onRemoveAttachment}
        disabled={attachmentInteractionBlocked}
      />
    ),
    [attachmentInteractionBlocked, attachments, onRemoveAttachment]
  );
  const ComposerAddAttachment = useCallback(
    () => (
      <>
        <input
          ref={fileInputRef}
          type="file"
          multiple
          className="hidden"
          onChange={event => {
            void onAttachFiles(event.target.files);
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
    ),
    [attachmentInteractionBlocked, attachments.length, maxAttachments, onAttachFiles, t]
  );
  /**
   * Primary-slot control for an empty composer: a circular button carrying the
   * user's mascot, opening the Human page. Same 28px circle as the Send button
   * it stands in for, so the row's metrics don't shift when a character is
   * typed; the avatar is inset a little so the mascot reads inside the circle
   * rather than filling it edge to edge.
   */
  const ComposerIdleAction = useCallback(
    () =>
      onOpenHumanMode ? (
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
          <MascotChipAvatar color={mascotColor} customPrimary={mascotCustomPrimary} size={18} />
        </Button>
      ) : null,
    [mascotColor, mascotCustomPrimary, onOpenHumanMode, t]
  );

  const components: ThreadComponents = useMemo(
    () => ({
      ToolFallback: ChatToolFallback,
      ToolGroup: ChatToolGroup,
      ComposerExtras,
      ComposerHeader,
      ComposerIdleAction,
      // Phase / reasoning round / active tool for the turn in flight. Reads the
      // runtime's `extras`, so it needs no props and no dependency here.
      RunningStatus: AssistantUiInferenceStatus,
      // One-line process summary under a settled answer, and the only door to
      // the reasoning / narration / tool detail that no longer renders inline.
      TurnFooter,
      onSwitchToMicCloud,
      ...(attachmentsEnabled
        ? {
            ComposerAttachments,
            ComposerAddAttachment,
            hasComposerAttachments: attachments.length > 0,
            onComposerAttachmentSend: onAttachmentOnlySend,
          }
        : {}),
    }),
    [
      ComposerAddAttachment,
      ComposerAttachments,
      ComposerExtras,
      ComposerHeader,
      ComposerIdleAction,
      attachments.length,
      attachmentsEnabled,
      onAttachmentOnlySend,
      onSwitchToMicCloud,
    ]
  );

  return (
    <AssistantUiRuntimeProvider>
      <ComposerTextBridge value={inputValue} onChange={onInputValueChange} />
      <TurnFooterHost onOpenTurnProcess={onOpenTurnProcess}>
        <SubagentDrawerHost onOpenSubagent={onOpenSubagent} canOpenSubagent={canOpenSubagent}>
          <Thread
            components={components}
            model={model}
            onModelChange={onModelChange}
            loadError={loadError}
            onEscape={onEscape}
            slashCommands={slashCommands}
          />
        </SubagentDrawerHost>
      </TurnFooterHost>
    </AssistantUiRuntimeProvider>
  );
}

export default AssistantUiChat;
