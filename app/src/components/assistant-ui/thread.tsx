'use client';

import { ActivityGroup as DefaultActivityGroup } from '@/components/assistant-ui/activity-group';
import {
  ComposerAddAttachment,
  ComposerAttachments,
  UserMessageAttachments,
} from '@/components/assistant-ui/attachment';
import { ComposerTriggerPopover } from '@/components/assistant-ui/composer-trigger-popover';
import { DirectiveText } from '@/components/assistant-ui/directive-text';
import { EditMessage } from '@/components/assistant-ui/elements/edit-message';
import { ErrorState } from '@/components/assistant-ui/elements/error-state';
import { Image } from '@/components/assistant-ui/elements/image';
import { MessageTiming } from '@/components/assistant-ui/elements/message-timing.aui';
import { StoppedRun } from '@/components/assistant-ui/elements/stopped-run';
import { ToolFallback } from '@/components/assistant-ui/elements/tool-fallback';
import { File } from '@/components/assistant-ui/file';
import { ThreadFollowupSuggestions } from '@/components/assistant-ui/follow-up-suggestions';
import { cn } from '@/components/assistant-ui/lib/utils';
import { MarkdownText } from '@/components/assistant-ui/markdown-text';
import { ComposerQuotePreview, SelectionToolbar } from '@/components/assistant-ui/quote';
import { Reasoning } from '@/components/assistant-ui/reasoning';
import { TooltipIconButton } from '@/components/assistant-ui/tooltip-icon-button';
import { Button } from '@/components/assistant-ui/ui/button';
import { Skeleton } from '@/components/assistant-ui/ui/skeleton';
import { ChatErrorNotice } from '@/features/conversations/aui/ChatErrorNotice';
import { ChatSettingsPanel } from '@/features/conversations/aui/ChatSettingsPanel';
import { ConnectionStateBanner } from '@/features/conversations/aui/ConnectionStateBanner';
import {
  useAuiEditCapabilities,
  useAuiReloadCapability,
} from '@/features/conversations/components/aui/auiThreadState';
import { useT } from '@/lib/i18n/I18nContext';
import { useAuiThreadId } from '@/providers/AssistantUiRuntimeProvider';
import { CHAT_ERROR_METADATA_KEY } from '@/store/threadSlice';
import { useActionBarReload, useMessageError } from '@assistant-ui/core/react';
import {
  ActionBarMorePrimitive,
  ActionBarPrimitive,
  type AssistantState,
  AuiIf,
  BranchPickerPrimitive,
  ComposerPrimitive,
  type FileMessagePartComponent,
  groupPartByType,
  type ImageMessagePartComponent,
  MessagePrimitive,
  SuggestionPrimitive,
  ThreadPrimitive,
  type ToolCallMessagePartComponent,
  type Unstable_SlashCommand,
  unstable_useSlashCommandAdapter,
  useAui,
  useAuiState,
} from '@assistant-ui/react';
import { LexicalComposerInput } from '@assistant-ui/react-lexical';
import debugFactory from 'debug';
import {
  ArrowDownIcon,
  ArrowUpIcon,
  CheckIcon,
  ChevronLeftIcon,
  ChevronRightIcon,
  CopyIcon,
  DownloadIcon,
  MicIcon,
  MoreHorizontalIcon,
  PencilIcon,
  RefreshCwIcon,
  SlashIcon,
  SquareIcon,
  ThumbsDownIcon,
  ThumbsUpIcon,
  Volume2Icon,
  VolumeXIcon,
} from 'lucide-react';
import {
  type ComponentType,
  createContext,
  type FC,
  type PropsWithChildren,
  type RefObject,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';

export type ThreadGroupPart = MessagePrimitive.GroupedParts.GroupPart;

/**
 * Optional component overrides for the thread. `AssistantMessage` and
 * `Welcome` replace whole sections; the remaining slots override how the
 * assistant message renders tool calls and part groups. Tool UIs registered
 * by name (toolkit `render`, `useAssistantDataUI`) take precedence over
 * `ToolFallback`.
 */
export type ThreadComponents = {
  AssistantMessage?: ComponentType | undefined;
  Welcome?: ComponentType | undefined;
  ToolFallback?: ToolCallMessagePartComponent | undefined;
  /**
   * Wraps one run of reasoning and tool calls — everything between the input
   * and the answer — as a single group. Defaults to `ActivityGroup`.
   */
  ActivityGroup?: ComponentType<PropsWithChildren<{ group: ThreadGroupPart }>> | undefined;
  /**
   * Host-owned disclosure for the source parts emitted after an answer:
   * `url` sources (web fetch/search) and `document` sources (memory
   * citations, `sourceType: 'document'`).
   */
  SourceGroup?: ComponentType<{ sources: readonly SourceItemPart[] }> | undefined;
  /**
   * Extra controls in the composer's action row, to the right of the model
   * selector. A seam rather than a fixed set because what belongs there is
   * host-specific — OpenHuman puts the context-window meter and the thread
   * goal here — and hard-coding either would make this component unusable by
   * anything else.
   */
  ComposerExtras?: ComponentType | undefined;
  /** Host-owned controls rendered in the right action cluster before voice. */
  ComposerRightExtras?: ComponentType | undefined;
  /** Host-owned navigation rail mounted inside the scrolling viewport. */
  ConversationMap?: ComponentType | undefined;
  /** Full-width host content immediately above the composer shell. */
  ComposerHeader?: ComponentType | undefined;
  /**
   * Host-owned progress line for the turn in flight, rendered under the last
   * message while `thread.isRunning`.
   *
   * A seam rather than a fixed widget because `isRunning` is all this file
   * knows: what the model is actually doing right now — reasoning round, active
   * tool, delegated sub-agent — lives in the host's own transport state, and
   * without somewhere to put it a long turn is an unlabelled spinner. The host
   * component returns `null` when it has nothing to say.
   */
  RunningStatus?: ComponentType | undefined;
  /** Host-owned attachment previews rendered above the editor. */
  ComposerAttachments?: ComponentType | undefined;
  /** Host-owned attachment picker rendered in the action row. */
  ComposerAddAttachment?: ComponentType | undefined;
  /** Enables sending when the host has attachments but the editor is empty. */
  hasComposerAttachments?: boolean | undefined;
  /** Sends an attachment-only message through the host's normal send path. */
  onComposerAttachmentSend?: (() => void) | undefined;
  /**
   * Host-owned control for the composer's primary slot while there is nothing
   * to send — the slot ChatGPT gives its voice mode. Send takes the slot back
   * on the first character or attachment, and a running turn always shows
   * Cancel. A component rather than a callback for the same reason
   * `ComposerAddAttachment` is one: what belongs there is the host's own
   * branding and behaviour, and this file should not learn about either.
   */
  ComposerIdleAction?: ComponentType | undefined;
  /** Switches the host chat surface into its microphone-first composer. */
  onSwitchToMicCloud?: (() => void) | undefined;
  /**
   * Host sink for files dropped on the composer or pasted into it.
   *
   * Supplying it also replaces `ComposerPrimitive.AttachmentDropzone` with the
   * equivalent host-driven handlers, because that primitive routes files to the
   * runtime's attachment adapter and refuses the drag outright
   * (`dataTransfer.dropEffect = 'none'`) when the runtime declares no
   * attachment capability — which is every runtime that keeps attachments on
   * the host side, as this app does.
   */
  onComposerFiles?: ((files: FileList | File[] | null) => void | Promise<void>) | undefined;
  /**
   * Whether the host can take files right now (feature enabled, composer
   * unlocked, budget left). Drives the drag affordance only; the host still
   * validates whatever arrives.
   */
  canAcceptComposerFiles?: boolean | undefined;
  /**
   * Host composer that REPLACES the built-in one (and the welcome suggestions
   * that belong to it) in the viewport footer, while the transcript above it
   * stays assistant-ui: the mic-first voice composer, whose input is a
   * push-to-talk button, and the workflow copilot, whose sends are structured
   * builder turns rather than chat turns.
   */
  Composer?: ComponentType | undefined;
  /**
   * Host-owned trigger pickers (`/` commands, `@` mentions), mounted inside the
   * composer's `Unstable_TriggerPopoverRoot` in place of the built-in `/`
   * popover fed by `slashCommands`. A component for the same reason the other
   * slots are: its sources are host behaviour this file should not learn.
   */
  ComposerTriggers?: ComponentType | undefined;
};

export type ThreadProps = {
  components?: ThreadComponents | undefined;
  /** Host-owned model route used for real sends. */
  model?: string | null | undefined;
  /** Updates the host's composer route and selected model metadata. */
  onModelChange?: ((value: string | null, contextWindow?: number | null) => void) | undefined;
  /** Host transport error shown in place of an empty welcome state. */
  loadError?: string | null | undefined;
  /** Host-specific Escape behavior (for example cancel + restore prompt). */
  onEscape?: (() => void) | undefined;
  /**
   * Commands offered when the composer input starts with `/`. Supplied by the
   * host because a command's `execute` is host behaviour (`/clear` has to
   * reach a runtime this component does not own).
   */
  slashCommands?: readonly Unstable_SlashCommand[] | undefined;
};

/**
 * Whether Lexical's own `SyncPlugin` is driving the composer store, making the
 * host's DOM→store bridge below not merely redundant but harmful.
 *
 * Lexical reconciles from `beforeinput` and needs `getTargetRanges()` to know
 * what the event will change. jsdom implements neither, so there the plugin
 * never commits editor state and the bridge is the ONLY path from a synthetic
 * `input` to the store — which is exactly why #5763 gated that bridge rather
 * than deleting it, and why 54 composer tests depend on it.
 *
 * In a real browser the plugin does commit, and then the bridge's write moves
 * the store through the *external* path. `SyncPlugin`'s runtime subscription
 * reads that as a foreign edit, calls `root.clear()` and rebuilds the editor —
 * and the rebuild restores the caret to the offset captured from the editor
 * state, which still lags the DOM by one keystroke. So the caret never
 * advances: typing `hello` one key at a time produced `holle` with the caret
 * stuck at 1 (#6163).
 *
 * Feature-detected rather than `import.meta.env` because the condition is a
 * real capability, not a build mode: any environment that reconciles from
 * `beforeinput` must not be bridged, and any that cannot must be.
 */
const lexicalDrivesTheStore = (): boolean =>
  typeof InputEvent !== 'undefined' && 'getTargetRanges' in InputEvent.prototype;

// Counts only — never a filename, a MIME type or clipboard content.
const debug = debugFactory('openhuman:assistant-composer');

/**
 * The files a drop is actually carrying.
 *
 * `dataTransfer.files` is the obvious source and is empty more often than it
 * looks: several macOS drag sources — the floating screenshot thumbnail among
 * them — hand the webview promise-backed items instead, leaving `files` at
 * length 0 while `items` holds the same content. Reading `files` alone made
 * those drops do nothing at all, with no error, because there was nothing to
 * reject.
 */
function filesFromDrop(dataTransfer: DataTransfer | null): File[] {
  const direct = Array.from(dataTransfer?.files ?? []);
  if (direct.length > 0) return direct;
  return Array.from(dataTransfer?.items ?? [])
    .filter(item => item.kind === 'file')
    .map(item => item.getAsFile())
    .filter((file): file is File => file !== null);
}

/**
 * Host-driven file drop for the whole open thread, not just the composer box:
 * a file dropped anywhere over the transcript lands as a composer attachment.
 * Mirrors the legacy composer's handlers (`ChatComposer.tsx`) and feeds the
 * same host path as the picker and paste, whose validator decides what the
 * active model can take (images only with vision, documents text-extracted).
 *
 * `preventDefault` on a *file* drag happens whether or not ingest is allowed:
 * without it the webview navigates away to the dropped file and the whole chat
 * is gone. Outside a thread, `installFileDropGuard` refuses the drop instead.
 */
function useThreadFileDrop() {
  const { onComposerFiles, canAcceptComposerFiles } = useContext(ThreadComponentsContext);
  const [isDraggingFiles, setIsDraggingFiles] = useState(false);
  // Attachment validation updates host state asynchronously. Keep drops in
  // arrival order so a second batch cannot validate against stale attachments
  // or overwrite the first batch while it is still being processed.
  const ingestQueueRef = useRef<Promise<void>>(Promise.resolve());

  const isFileDrag = (event: React.DragEvent) =>
    Array.from(event.dataTransfer?.types ?? []).includes('Files');
  const onDragOver = (event: React.DragEvent) => {
    if (!isFileDrag(event)) return;
    event.preventDefault();
    if (!onComposerFiles || !canAcceptComposerFiles) {
      event.dataTransfer.dropEffect = 'none';
      return;
    }
    event.dataTransfer.dropEffect = 'copy';
    setIsDraggingFiles(true);
  };
  const onDragLeave = (event: React.DragEvent) => {
    // Ignore leave events that bubble while the cursor is still over a child.
    if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
    setIsDraggingFiles(false);
  };
  const onDrop = (event: React.DragEvent) => {
    if (!isFileDrag(event)) return;
    event.preventDefault();
    setIsDraggingFiles(false);
    if (!onComposerFiles || !canAcceptComposerFiles) {
      debug('[assistant-composer] drop: refused, ingest not accepting');
      return;
    }
    const files = filesFromDrop(event.dataTransfer);
    if (files.length === 0) {
      debug('[assistant-composer] drop: file drag carried no readable files');
      return;
    }
    debug('[assistant-composer] drop: queueing %d file(s) for ingest', files.length);
    ingestQueueRef.current = ingestQueueRef.current
      .catch(() => undefined)
      .then(() => onComposerFiles(files))
      .catch(error => {
        debug('[assistant-composer] drop: file ingest failed: %o', error);
      });
  };

  return { isDraggingFiles, dropHandlers: { onDragOver, onDragLeave, onDrop } };
}

const EMPTY_COMPONENTS: ThreadComponents = {};

const ThreadComponentsContext = createContext<ThreadComponents>(EMPTY_COMPONENTS);

const NO_SLASH_COMMANDS: readonly Unstable_SlashCommand[] = [];
const SlashCommandsContext = createContext<readonly Unstable_SlashCommand[]>(NO_SLASH_COMMANDS);

// Startup exposes a loading placeholder thread; treat it as a new chat so
// the composer mounts centered. Loads after startup keep the docked layout.
const isNewChatView = (s: AssistantState) =>
  s.thread.messages.length === 0 && (!s.thread.isLoading || s.threads.isLoading);

// A switched thread that is still fetching its history: skeleton, not welcome.
const isHistoryLoadingView = (s: AssistantState) =>
  s.thread.messages.length === 0 &&
  s.thread.isLoading &&
  !s.thread.isDisabled &&
  !s.threads.isLoading;

const ThreadHistorySkeleton: FC = () => (
  <div
    data-slot="aui_thread-history-skeleton"
    role="status"
    className="animate-in fade-in fill-mode-both flex flex-col gap-y-6 [animation-delay:150ms] [animation-duration:200ms]">
    <span className="sr-only">Loading conversation</span>
    <Skeleton className="ml-auto h-9 w-2/5 rounded-xl motion-reduce:animate-none" />
    <div className="flex flex-col gap-y-2">
      <Skeleton className="h-4 w-11/12 motion-reduce:animate-none" />
      <Skeleton className="h-4 w-4/5 motion-reduce:animate-none" />
      <Skeleton className="h-4 w-3/5 motion-reduce:animate-none" />
    </div>
    <Skeleton className="ml-auto h-9 w-1/3 rounded-xl motion-reduce:animate-none" />
    <div className="flex flex-col gap-y-2">
      <Skeleton className="h-4 w-10/12 motion-reduce:animate-none" />
      <Skeleton className="h-4 w-2/3 motion-reduce:animate-none" />
    </div>
  </div>
);

export const Thread: FC<ThreadProps> = ({
  components = EMPTY_COMPONENTS,
  model = 'hint:chat',
  onModelChange,
  loadError = null,
  onEscape,
  slashCommands = NO_SLASH_COMMANDS,
}) => {
  const isEmpty = useAuiState(isNewChatView);

  return (
    <ThreadComponentsContext.Provider value={components}>
      <SlashCommandsContext.Provider value={slashCommands}>
        <ThreadRoot
          isEmpty={isEmpty}
          model={model}
          onModelChange={onModelChange}
          loadError={loadError}
          onEscape={onEscape}
        />
      </SlashCommandsContext.Provider>
    </ThreadComponentsContext.Provider>
  );
};

const ThreadRoot: FC<{
  isEmpty: boolean;
  model: string | null;
  onModelChange?: (value: string | null, contextWindow?: number | null) => void;
  loadError: string | null;
  onEscape?: () => void;
}> = ({ isEmpty, model, onModelChange, loadError, onEscape }) => {
  const {
    Welcome = ThreadWelcome,
    Composer: HostComposer,
    ConversationMap,
  } = useContext(ThreadComponentsContext);
  const viewportRef = useRef<HTMLDivElement>(null);
  const messageGroupRef = useRef<HTMLDivElement>(null);
  // Everything the viewport scrolls over, which is MORE than the message group:
  // the footer below it holds the follow-up suggestions, and those appear when
  // a reply finishes. Observing only the messages misses that growth and leaves
  // the transcript stranded short of the bottom at the end of every turn.
  const scrollContentRef = useRef<HTMLDivElement>(null);

  const { claimScroll } = useFollowBottom(viewportRef, scrollContentRef);
  useOpenThreadAtBottom(viewportRef, claimScroll);
  const { isDraggingFiles, dropHandlers } = useThreadFileDrop();

  return (
    <ThreadPrimitive.Root
      className="aui-root aui-thread-root bg-background @container flex h-full flex-col"
      {...dropHandlers}
      style={{
        ['--thread-max-width' as string]: '44rem',
        ['--composer-bg' as string]: 'var(--color-card)',
        ['--composer-radius' as string]: '1.5rem',
        ['--composer-padding' as string]: '8px',
      }}>
      <ThreadPrimitive.Viewport
        ref={viewportRef}
        // The host follower below checks the reader's live distance from the
        // bottom. Disable assistant-ui's unconditional run-start jump so it
        // cannot override a reader who intentionally scrolled into history.
        autoScroll={false}
        scrollToBottomOnRunStart={false}
        data-slot="aui_thread-viewport"
        className="relative flex flex-1 flex-col overflow-x-auto overflow-y-scroll scroll-smooth">
        {ConversationMap ? <ConversationMap /> : null}
        <div
          ref={scrollContentRef}
          className={cn(
            'mx-auto flex w-full max-w-(--thread-max-width) flex-1 flex-col px-4 pt-4',
            isEmpty && 'justify-center'
          )}>
          {loadError ? (
            <div className="flex flex-1 flex-col items-center justify-center gap-2 text-center">
              <p className="text-sm font-medium text-destructive">Failed to load messages</p>
              <p className="text-muted-foreground max-w-md text-xs">{loadError}</p>
            </div>
          ) : (
            <>
              <AuiIf condition={isNewChatView}>
                <Welcome />
              </AuiIf>
              <AuiIf condition={isHistoryLoadingView}>
                <ThreadHistorySkeleton />
              </AuiIf>
            </>
          )}

          <div
            ref={messageGroupRef}
            data-slot="aui_message-group"
            className="mb-14 flex flex-col gap-y-6 empty:hidden">
            <ThreadPrimitive.Messages>{() => <ThreadMessage />}</ThreadPrimitive.Messages>
            <RunningStatusSlot />
          </div>
          <ThreadBottomFollower
            viewportRef={viewportRef}
            contentRef={messageGroupRef}
            claimScroll={claimScroll}
          />

          <ThreadPrimitive.ViewportFooter
            className={cn(
              'aui-thread-viewport-footer bg-background flex flex-col gap-4 overflow-visible pb-4 md:pb-6',
              !isEmpty && 'sticky bottom-0 mt-auto rounded-t-(--composer-radius)'
            )}>
            <ThreadScrollToBottom />
            <ThreadFollowupSuggestions />
            <ConnectionStateBanner />
            {HostComposer ? (
              <HostComposer />
            ) : (
              <>
                <Composer
                  model={model}
                  onModelChange={onModelChange}
                  onEscape={onEscape}
                  isDraggingFiles={isDraggingFiles}
                />
                <AuiIf condition={s => isNewChatView(s) && s.composer.isEmpty}>
                  <ThreadSuggestions />
                </AuiIf>
              </>
            )}
          </ThreadPrimitive.ViewportFooter>
        </div>
      </ThreadPrimitive.Viewport>

      {/*
       * Select text in any message and a floating "Quote" button appears over
       * the selection; clicking it drops the excerpt into the composer.
       *
       * It lives OUTSIDE the viewport on purpose: it portals itself to the
       * selection's screen position, so nesting it inside the scroller would
       * only give it a clipped, scrolling ancestor for no benefit. It finds the
       * message by the `data-message-id` that `MessagePrimitive.Root` already
       * emits, so neither message component needed changing.
       */}
      {!HostComposer && <SelectionToolbar />}
    </ThreadPrimitive.Root>
  );
};

/**
 * Opening a thread lands on its newest message.
 *
 * assistant-ui has two stock knobs for this and BOTH are inert here:
 *
 * - `scrollToBottomOnThreadSwitch` listens for `threads.selectionChanged`,
 *   which fires only when its `mainThreadId` changes. That id is
 *   `adapter.threadId ?? DEFAULT_THREAD_ID`, and `useOpenHumanExternalStore`
 *   returns no `threadId` — the real thread travels out-of-band through
 *   `AuiThreadIdContext` — so `mainThreadId` never leaves the default and the
 *   event never fires.
 * - `scrollToBottomOnInitialize` latches on the first non-empty render and
 *   re-arms only while the thread has zero messages. `<AssistantUiChat>` is
 *   mounted without a `key`, so this viewport survives thread switches with
 *   that latch still set.
 *
 * The second one is why the defect is intermittent rather than total, and it
 * is the case to keep in mind. `useOpenHumanExternalStore` reads
 * `state.thread.messagesByThreadId[threadId]`, a cache cleared only on delete
 * or sign-out, so a thread visited earlier this session hands its messages
 * over on the very render the id changes: it never passes through the empty
 * state that re-arms the latch, and the viewport keeps the PREVIOUS thread's
 * `scrollTop`. A thread not yet cached does briefly read empty and therefore
 * scrolls correctly even unfixed — so a fix checked only against a fresh
 * thread looks right and fixes nothing.
 *
 * Hence: latch on the thread id rather than on emptiness. Nothing here is
 * conditional on the reader's scroll position, unlike `ThreadBottomFollower`
 * below — "don't yank the reader who scrolled up" is about a new turn arriving
 * in the thread being read, and a scroll offset left over from a different
 * thread is not a reading position worth restoring.
 *
 * This lives in `ThreadRoot`, which owns `viewportRef`, rather than in
 * `ThreadBottomFollower`, which is handed it: a descendant's layout effect
 * runs before its ancestor's ref is attached, so the follower sees
 * `viewportRef.current === null` on the mount that matters and would burn the
 * latch without scrolling.
 */
function useOpenThreadAtBottom(
  viewportRef: RefObject<HTMLDivElement | null>,
  claimScroll: () => void
) {
  const hasMessages = useAuiState(s => s.thread.messages.length > 0);
  const threadId = useAuiThreadId();
  // Which thread this viewport has already been dropped to the bottom for.
  // `undefined` (nothing opened yet) is deliberately distinct from the
  // `string | null` a thread id can be, so the initial value cannot collide
  // with a genuine "no thread selected".
  const openedThreadRef = useRef<string | null | undefined>(undefined);

  useLayoutEffect(() => {
    // Wait for the transcript: on the uncached path the messages arrive a tick
    // after the id changes, and a scroll issued against an empty viewport goes
    // nowhere. Leaving the latch alone here is what lets that second pass run.
    if (!hasMessages) return;
    if (openedThreadRef.current === threadId) return;
    const viewport = viewportRef.current;
    if (!viewport) return;

    openedThreadRef.current = threadId;
    // `behavior: 'instant'` overrides the viewport's `scroll-smooth` class:
    // opening a thread should start at the bottom, not animate down through the
    // entire history to get there.
    viewport.scrollTo({ top: viewport.scrollHeight, behavior: 'instant' });
    // Re-arm following for the NEW thread. `followRef` lives as long as this
    // viewport, which outlives any one thread, so without this a reader who
    // scrolled up in thread A carries that `false` into thread B — and if both
    // threads sit at the same `scrollTop` (0 and 0 is the easy case) the open
    // scroll produces no `scroll` event to re-enable it, so B never follows its
    // own reply. Opening a thread is not a reader scrolling away from it.
    claimScroll();
  }, [claimScroll, hasMessages, threadId, viewportRef]);
}

const FOLLOW_BOTTOM_THRESHOLD_PX = 80;

/**
 * How long after a scroll-capable input a falling `scrollTop` still counts as
 * the reader moving. Generous enough for a wheel's momentum tail and a held
 * key's repeat; far shorter than any gap between a gesture and an unrelated
 * layout shift worth ignoring.
 */
const USER_SCROLL_INTENT_WINDOW_MS = 1000;

/** Keys that scroll a focused scroller up (or anywhere — any of them is intent). */
const SCROLL_KEYS = new Set(['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End', ' ']);

/**
 * Keep the newest content in view while the assistant streams.
 *
 * `ThreadBottomFollower` below cannot do this. It keys on
 * `latestMessage.id`/`.role`, and a streaming reply is ONE message whose id
 * never changes (`STREAMING_TAIL_ID`, `providers/assistantUiMessages.ts`) and
 * whose role is `assistant` — so it neither passes that hook's `role ===
 * 'user'` guard nor re-runs as tokens land. assistant-ui's own `autoScroll` is
 * off here deliberately: its stick threshold is ~1px against this host's 80,
 * and its run-start jump is unconditional, so enabling it would put two
 * followers with different ideas of "at the bottom" on one viewport.
 *
 * So follow the content box rather than the message list. A `ResizeObserver`
 * fires on every height change from any cause — tokens, markdown reflow, a
 * code block, a tool timeline expanding, an image decoding — none of which the
 * message identity reports.
 *
 * The reader stays in charge: `followRef` tracks their live distance from the
 * bottom, so scrolling up into history stops the following, and scrolling back
 * within `FOLLOW_BOTTOM_THRESHOLD_PX` resumes it. That is the contract the
 * viewport comment states — never yank a reader who deliberately left the
 * bottom — and it is enforced here rather than assumed.
 *
 * ## Pin-then-follow, and what that costs
 *
 * `ThreadBottomFollower` aligns a new USER message to the TOP of the viewport
 * so the reply streams beneath it. For a reply taller than the viewport that
 * alignment and this following are mutually exclusive: keep the question
 * pinned and the answer streams below the fold — the reported defect — or
 * follow the answer and the question eventually scrolls off the top.
 *
 * The choice made here is **pin at turn start, follow thereafter**, with the
 * reader overriding both by scrolling away. The pin is an alignment for the
 * moment a turn begins, not a claim on the whole turn, and every mainstream
 * chat client resolves it the same way. This is a visible change to how a long
 * reply reads, so it is recorded rather than left to be rediscovered.
 *
 * What a reader gives up: on a reply taller than the viewport, their own
 * question scrolls off the top as the answer streams. What they get back is
 * the answer being on screen while it arrives, which is the defect this fixes.
 */
function useFollowBottom(
  viewportRef: RefObject<HTMLDivElement | null>,
  contentRef: RefObject<HTMLDivElement | null>
) {
  // Starts true so a thread opens following; the first user scroll away from
  // the bottom is what turns it off.
  const followRef = useRef(true);
  const lastScrollTopRef = useRef(0);
  // `scrollHeight` as it stood when we last claimed a scroll. Growth up to this
  // mark is growth we already knew about; only growth BEYOND it is new content
  // worth following. `null` means no claim is outstanding.
  const claimedHeightRef = useRef<number | null>(null);

  /**
   * Declare a scroll as OURS, after performing it.
   *
   * Setting `followRef` alone is not enough and was the first thing I tried.
   * A programmatic scroll is synchronous but its `scroll` event is not, so the
   * listener runs AFTER the caller has re-armed the flag, sees a `scrollTop`
   * lower than the stale baseline, and clears it again. Re-baselining here —
   * while `scrollTop` already holds the post-scroll value — is what makes the
   * later event a no-op instead.
   */
  const claimScroll = useCallback(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    lastScrollTopRef.current = viewport.scrollTop;
    claimedHeightRef.current = viewport.scrollHeight;
    followRef.current = true;
  }, [viewportRef]);

  useEffect(() => {
    const viewport = viewportRef.current;
    const content = contentRef.current;
    if (!viewport || !content) return;

    const distanceFromBottom = () =>
      viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight;

    // Turning following ON needs only proximity to the bottom. Turning it OFF
    // requires the reader to have moved UP, which is the part that matters:
    //
    // a growth-induced `scroll` event carries the reply's NEW `scrollHeight`
    // against an unmoved `scrollTop`, so a bare proximity test would read "far
    // from the bottom" and clear the flag for a reader who never moved —
    // silently ending the follow this hook exists to provide. Requiring a
    // decrease in `scrollTop` makes that impossible: content growth does not
    // move it, scroll anchoring only ever moves it DOWN the document (it
    // preserves the visual position when content is inserted above), and this
    // hook's own `scrollTo` moves it to the maximum.
    //
    // `reasoning.tsx` solves the same problem by additionally requiring
    // `scrollHeight` to be unchanged. That is right for a small preview box and
    // wrong here: during a live stream the height changes on almost every
    // event, so the reader's scroll away would be ignored and they would be
    // dragged back down — breaking the "never yank a reader who left the
    // bottom" contract. Keying on `scrollTop` alone holds in both cases.
    lastScrollTopRef.current = viewport.scrollTop;

    // ...and the move up has to be the READER's. A falling `scrollTop` is not
    // proof of that: a disclosure collapsing above the fold, content shrinking
    // under a reply that swaps parts, and assistant-ui's `useScrollLock`
    // (which writes the old `scrollTop` back on every scroll event while a
    // disclosure animates) all lower it with nobody touching anything — and
    // each used to switch following off mid-turn, leaving the reply streaming
    // below the fold. So a decrease only counts within a short window after
    // an input that can scroll: wheel, touch, a scroll key, or a press on the
    // viewport itself (its scrollbar).
    let userIntentAt = Number.NEGATIVE_INFINITY;
    let scrollbarDragActive = false;
    const markIntent = () => {
      userIntentAt = window.performance.now();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (SCROLL_KEYS.has(event.key)) markIntent();
    };
    const onPointerDown = (event: PointerEvent) => {
      if (event.target === viewport) {
        scrollbarDragActive = true;
        markIntent();
      }
    };
    const clearScrollbarDrag = () => {
      scrollbarDragActive = false;
    };

    const onScroll = () => {
      if (distanceFromBottom() <= FOLLOW_BOTTOM_THRESHOLD_PX) {
        followRef.current = true;
      } else if (
        viewport.scrollTop < lastScrollTopRef.current &&
        (scrollbarDragActive ||
          window.performance.now() - userIntentAt <= USER_SCROLL_INTENT_WINDOW_MS)
      ) {
        followRef.current = false;
      }
      lastScrollTopRef.current = viewport.scrollTop;
    };
    viewport.addEventListener('scroll', onScroll, { passive: true });
    viewport.addEventListener('wheel', markIntent, { passive: true });
    viewport.addEventListener('touchmove', markIntent, { passive: true });
    // Scroll keys are delivered to whatever has focus, not to the viewport.
    const keyTarget = viewport.ownerDocument;
    keyTarget.addEventListener('keydown', onKeyDown);
    viewport.addEventListener('pointerdown', onPointerDown);
    viewport.addEventListener('pointerup', clearScrollbarDrag);
    viewport.addEventListener('pointercancel', clearScrollbarDrag);

    const observer = new ResizeObserver(() => {
      // Read the flag; do NOT recompute the distance here. By the time this
      // callback runs the content has already grown: `scrollHeight` is the new
      // larger value while `scrollTop` has not moved, so a fresh measurement
      // reads "far from the bottom" *because of the growth being reacted to*.
      // Recomputing would decline to follow on the first token batch and never
      // recover, which looks identical to the defect this hook fixes. The
      // question is "was the reader at the bottom BEFORE this growth", and only
      // a value captured before it can answer that.
      //
      // The flag is maintained by `onScroll` above, which only clears it on a
      // genuine upward move by the reader — see the note there for why a bare
      // proximity test would clear it on the growth being reacted to.
      if (!followRef.current) return;
      // Do not follow the growth that PROMPTED the claim.
      //
      // A new user message grows the content box, which queues a resize
      // notification; `ThreadBottomFollower` then aligns that message to the
      // top and claims the scroll. The queued callback runs afterwards, and
      // following it would scroll straight to the bottom — erasing the
      // alignment before it is ever painted, which makes the alignment
      // pointless rather than merely short-lived.
      //
      // So a claim records the height it was made at, and growth up to that
      // mark is ignored. The first growth BEYOND it is the reply arriving,
      // which is what following is for; the mark is then dropped so the rest
      // of the turn streams normally.
      const claimedHeight = claimedHeightRef.current;
      if (claimedHeight !== null) {
        if (viewport.scrollHeight <= claimedHeight) return;
        claimedHeightRef.current = null;
      }
      // `instant` overrides the viewport's `scroll-smooth`: a smooth animation
      // per token would lag permanently behind the stream.
      viewport.scrollTo({ top: viewport.scrollHeight, behavior: 'instant' });
    });
    observer.observe(content);

    return () => {
      viewport.removeEventListener('scroll', onScroll);
      viewport.removeEventListener('wheel', markIntent);
      viewport.removeEventListener('touchmove', markIntent);
      keyTarget.removeEventListener('keydown', onKeyDown);
      viewport.removeEventListener('pointerdown', onPointerDown);
      viewport.removeEventListener('pointerup', clearScrollbarDrag);
      viewport.removeEventListener('pointercancel', clearScrollbarDrag);
      observer.disconnect();
    };
  }, [contentRef, viewportRef]);

  return { followRef, claimScroll };
}

/**
 * Align a new turn only for a reader who remains near the bottom. assistant-ui's
 * run-start scroll is unconditional, which would pull a reader from older
 * messages into every new turn.
 */
const ThreadBottomFollower: FC<{
  viewportRef: RefObject<HTMLDivElement | null>;
  contentRef: RefObject<HTMLDivElement | null>;
  claimScroll: () => void;
}> = ({ viewportRef, contentRef, claimScroll }) => {
  const latestMessage = useAuiState(s => s.thread.messages.at(-1));

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    const distanceFromBottom = viewport
      ? viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight
      : Infinity;
    if (latestMessage?.role !== 'user' || distanceFromBottom > FOLLOW_BOTTOM_THRESHOLD_PX) {
      return;
    }
    const userMessages = contentRef.current?.querySelectorAll<HTMLElement>('[data-role="user"]');
    // `behavior: 'instant'`, like the other two scrolls in this file, and for a
    // second reason beyond overriding `scroll-smooth`: `claimScroll` below
    // re-baselines from `viewport.scrollTop`, which is only correct if the move
    // has already happened. A smooth alignment has NOT moved it by the time the
    // next line runs, so the baseline would capture the pre-scroll position and
    // the animation's own scroll events — a run of decreasing `scrollTop` —
    // would read as the reader scrolling away, clearing the flag exactly when
    // the reply starts. That failed intermittently rather than always, since a
    // growth event landing between animation frames could re-arm it.
    userMessages
      ?.item(userMessages.length - 1)
      ?.scrollIntoView({ block: 'start', behavior: 'instant' });
    // This alignment scrolls UP whenever the reader was at the bottom — the new
    // user message sits above the trailing `mb-14` and running-status slot, so
    // bringing its top to the viewport top lowers `scrollTop`. To
    // `useFollowBottom`'s listener that is indistinguishable from the reader
    // scrolling away, and it would clear the follow flag at the exact moment
    // the reply starts arriving. Re-arm: this scroll is ours, not theirs.
    claimScroll();
  }, [claimScroll, contentRef, latestMessage?.id, latestMessage?.role, viewportRef]);

  return null;
};

/**
 * The host's `RunningStatus`, gated on the thread actually running.
 *
 * Kept inside the message group so the line sits under the last message —
 * where the answer is about to appear — rather than pinned to the composer.
 */
const RunningStatusSlot: FC = () => {
  const { RunningStatus } = useContext(ThreadComponentsContext);
  if (!RunningStatus) return null;
  return (
    <AuiIf condition={s => s.thread.isRunning}>
      <RunningStatus />
    </AuiIf>
  );
};

const ThreadMessage: FC = () => {
  const { AssistantMessage: AssistantMessageComponent = AssistantMessage } =
    useContext(ThreadComponentsContext);
  const role = useAuiState(s => s.message.role);
  const isEditing = useAuiState(s => s.message.composer.isEditing);

  if (isEditing) return <EditComposer />;
  if (role === 'user') return <UserMessage />;
  return <AssistantMessageComponent />;
};

const ThreadScrollToBottom: FC = () => {
  return (
    <ThreadPrimitive.ScrollToBottom asChild>
      <TooltipIconButton
        tooltip="Scroll to bottom"
        variant="outline"
        className="aui-thread-scroll-to-bottom dark:border-border dark:bg-background dark:hover:bg-accent absolute -top-12 z-10 self-center rounded-full p-4 disabled:invisible">
        <ArrowDownIcon />
      </TooltipIconButton>
    </ThreadPrimitive.ScrollToBottom>
  );
};

const ThreadWelcome: FC = () => {
  return (
    <div className="aui-thread-welcome-root mb-6 flex flex-col items-center px-4 text-center">
      <h1 className="aui-thread-welcome-message-inner fade-in slide-in-from-bottom-1 animate-in fill-mode-both text-2xl font-medium tracking-tight duration-200">
        How can I help you today?
      </h1>
    </div>
  );
};

const ThreadSuggestions: FC = () => {
  return (
    <div className="aui-thread-welcome-suggestions flex w-full flex-wrap items-center justify-center gap-2 px-4">
      <ThreadPrimitive.Suggestions>{() => <ThreadSuggestionItem />}</ThreadPrimitive.Suggestions>
    </div>
  );
};

const ThreadSuggestionItem: FC = () => {
  return (
    <div className="aui-thread-welcome-suggestion-display fade-in slide-in-from-bottom-2 animate-in fill-mode-both duration-200">
      <SuggestionPrimitive.Trigger send asChild>
        <Button
          variant="ghost"
          className="aui-thread-welcome-suggestion text-foreground hover:bg-muted border-border/60 h-auto gap-1.5 rounded-full border px-3.5 py-1.5 text-sm font-normal whitespace-nowrap transition-colors">
          <SuggestionPrimitive.Title className="aui-thread-welcome-suggestion-text-1" />
          <SuggestionPrimitive.Description className="aui-thread-welcome-suggestion-text-2 empty:hidden" />
        </Button>
      </SuggestionPrimitive.Trigger>
    </div>
  );
};

const Composer: FC<{
  model: string | null;
  onModelChange?: (value: string | null, contextWindow?: number | null) => void;
  onEscape?: () => void;
  /** A file drag is over the thread and will land here; see `useThreadFileDrop`. */
  isDraggingFiles: boolean;
}> = ({ model, onModelChange, onEscape, isDraggingFiles }) => {
  const aui = useAui();
  const commands = useContext(SlashCommandsContext);
  const slash = unstable_useSlashCommandAdapter({ commands, fallbackIcon: SlashIcon });
  const inputWrapperRef = useRef<HTMLDivElement>(null);
  const {
    ComposerHeader,
    ComposerAttachments: HostComposerAttachments,
    ComposerTriggers: HostComposerTriggers,
    onComposerFiles,
    canAcceptComposerFiles,
  } = useContext(ThreadComponentsContext);
  useEffect(() => {
    const textbox = inputWrapperRef.current?.querySelector<HTMLElement>('[contenteditable="true"]');
    textbox?.setAttribute('aria-label', 'Message input');
    // The rich Lexical surface deliberately is not a native textarea, so give
    // it an explicit stable hook for browser tests and assistive tooling. The
    // old chat composer exposed a textarea with a placeholder; consumers must
    // not have to depend on Lexical's internal DOM shape to find the primary
    // message input.
    textbox?.setAttribute('data-testid', 'chat-message-input');
    return () => {
      textbox?.removeAttribute('aria-label');
      textbox?.removeAttribute('data-testid');
    };
  }, []);

  // Set for as long as an IME composition is open. The gate is a ref rather
  // than state because it is read from a microtask, not from a render.
  //
  // Adopted from #5764 (@ligjn), which identified the hazard this closes: the
  // store write below is deferred, and a fast CJK typist can open the next
  // composition before it runs. That stale write would rebuild the editor
  // mid-composition and cancel it -- #5763 again, one composition later.
  const isComposingTextRef = useRef(false);

  // DOM text -> composer store. The text is read at event time; only the write
  // is deferred by a microtask, so the editor has finished applying the event
  // before the store changes under it.
  //
  // The gate is re-checked INSIDE the microtask, not just at event time: a
  // composition that started in between makes this write stale, and dropping it
  // loses nothing, because the DOM is the source of truth and that
  // composition's own commit reads the whole of it.
  // Capture phase, so the media is pulled out and the default cancelled before
  // Lexical's own paste handling turns it into editor content.
  const handlePasteCapture = (event: React.ClipboardEvent) => {
    if (!onComposerFiles) return;
    if (!canAcceptComposerFiles) {
      debug('[assistant-composer] paste: refused, ingest not accepting');
      return;
    }
    const itemFiles = Array.from(event.clipboardData?.items ?? [])
      .filter(item => item.kind === 'file' && /^(image|video)\//.test(item.type))
      .map(item => item.getAsFile())
      .filter((file): file is File => file !== null);
    // Real clipboard events expose media through `items`; synthetic browser
    // events (including the Playwright path) may populate only `files`.
    // Accept both representations so paste follows the same ingest path as
    // the picker and dropzone.
    const files = itemFiles.length > 0 ? itemFiles : Array.from(event.clipboardData?.files ?? []);
    if (files.length === 0) {
      // The overwhelmingly common case: an ordinary text paste. Left for Lexical.
      return;
    }
    event.preventDefault();
    debug('[assistant-composer] paste: ingesting %d media file(s)', files.length);
    onComposerFiles(files);
  };

  const syncComposerFromDom = (target: EventTarget | null) => {
    if (!(target instanceof HTMLElement)) return;
    const text = target.textContent ?? '';
    globalThis.queueMicrotask(() => {
      if (isComposingTextRef.current) return;
      aui.composer.setText(text);
    });
  };

  return (
    <ComposerPrimitive.Unstable_TriggerPopoverRoot>
      <ComposerPrimitive.Root
        className="aui-composer-root relative flex w-full flex-col"
        data-walkthrough="chat-agent-panel">
        {ComposerHeader ? <ComposerHeader /> : null}
        {/*
         * Neutered whenever the host owns file ingest: every handler in the
         * primitive short-circuits on `disabled`, so the thread-wide handlers in
         * `useThreadFileDrop` are the only ones left and the `data-dragging`
         * styling runs off their state. Left enabled otherwise, so a host that does
         * use a runtime attachment adapter keeps the primitive's behaviour.
         */}
        <ComposerPrimitive.AttachmentDropzone asChild disabled={!!onComposerFiles}>
          <div
            data-slot="aui_composer-shell"
            data-dragging={onComposerFiles && isDraggingFiles ? 'true' : undefined}
            onPasteCapture={handlePasteCapture}
            // Keyed to `content-faint` rather than `line`/`line-strong`, which
            // sat too close to the composer's own surface to read as an edge at
            // all; `content-faint` is a real step along the grey ramp in both
            // themes and the alpha then pulls it back.
            //
            // The border is deliberately fainter than the content card's edge
            // (0.65 in `index.css`) because it is not carrying the definition
            // alone: `shadow-soft` lifts the composer off the transcript, and a
            // lifted surface needs less outline than a flat one to read as
            // separate. Border and shadow together at low strength read calmer
            // than either at full — a hard 0.65 line under a shadow reads as
            // two competing edges.
            //
            // Two roles, kept apart: the SHADOW is constant and the BORDER is
            // what moves.
            //
            // The shadow is an explicit near-black pair rather than
            // `shadow-soft`/`shadow-medium`. Those tokens are black at 0.08
            // alpha, which is a diffuse haze — on the themed chrome behind this
            // composer it reads as a smudge rather than a cast shadow.
            //
            // Both layers are pushed DOWN rather than spread evenly, because an
            // even shadow reads as a glow: it implies light from everywhere,
            // which is no light at all, and the composer ends up looking fuzzy
            // instead of raised. The offsets (6px, 22px) exceed each layer's
            // negative spread (-4px, -16px), so the cast clears the box on the
            // bottom edge and is pulled in at the top — the asymmetry is what
            // says "lit from above".
            //
            //   0 8px  12px -4px  / 0.18  — contact: tight, near the edge
            //   0 30px 44px -16px / 0.24  — cast: far, wide, and the stronger
            //
            // Both halved from 0.34 / 0.48: at those strengths the composer
            // read as hovering well above the page, and the cast crowded the
            // last message. Half keeps the lit-from-above asymmetry while the
            // surface sits closer to the transcript.
            //
            // The far layer carrying more alpha than the near one is
            // deliberate and is what gives depth; the usual instinct is the
            // reverse, which flattens it back out.
            //
            // `animate-composer-shadow` then orbits those offsets clockwise on
            // a slow loop (`composerShadowOrbit`, `index.css`), as though the
            // light above the composer circles the room. The static values here
            // are the orbit's 25% stop, so the animation starts from roughly
            // where the unanimated composer sits rather than jumping on load. The static `shadow-[…]` above is
            // not redundant: it is what `motion-reduce:animate-none` falls back
            // to, so the composer keeps its elevation when the OS asks for less
            // motion and merely stops moving. Keyframes override the utility
            // while the animation runs, which is why the two can coexist.
            //
            // Focus is now carried entirely by the border — 0.35 → 0.90 on the
            // same token, so the edge sharpens rather than changing colour —
            // and `transition` names border-color alone. Animating the shadow
            // as well meant two things moving at once for a single event; with
            // the elevation fixed, the composer stays put and only its outline
            // responds. `duration-200 ease-out` is the settle, and
            // `motion-reduce` drops it for anyone who asked the OS for less
            // motion — the cue still lands, just instantly.
            //
            // `border-ring` on drag is untouched — that state is meant to break
            // the pattern.
            className="border-content-faint/35 focus-within:border-content-faint/90 data-[dragging=true]:border-ring shadow-[0_8px_12px_-4px_rgb(0_0_0/0.09),0_30px_44px_-16px_rgb(0_0_0/0.12)] animate-composer-shadow motion-reduce:animate-none flex w-full cursor-text flex-col gap-2 rounded-(--composer-radius) border bg-(--composer-bg) p-(--composer-padding) transition-[border-color] duration-200 ease-out motion-reduce:transition-none data-[dragging=true]:border-dashed data-[dragging=true]:bg-[color-mix(in_oklab,var(--color-accent)_50%,var(--color-background))]">
            {/* Renders only while a quote is set; dismissing it clears the quote. */}
            <ComposerQuotePreview />
            {HostComposerAttachments ? <HostComposerAttachments /> : <ComposerAttachments />}
            {/*
             * Lexical rather than the plain `ComposerPrimitive.Input` textarea,
             * because `/` commands need a rich input: the trigger popover has to
             * anchor to the caret and the accepted command has to become a chip
             * rather than literal text the model would read. `commands` is empty
             * unless the host supplies some, and with none the popover never
             * opens, so a host that wants a plain box still gets one.
             */}
            <LexicalComposerInput
              ref={inputWrapperRef}
              placeholder="Send a message..."
              onCompositionStartCapture={() => {
                isComposingTextRef.current = true;
              }}
              onInputCapture={event => {
                // An IME fires `input` per keystroke while the candidate window is
                // still open, and the text on the DOM then is the pre-edit, not the
                // user's input. Writing it into the store re-renders the editor and
                // cancels the composition, so `nihao` + Enter committed as
                // `n ni nihao 你好` (#5763). The keydown guard below already refuses
                // to act mid-composition; this bridge was the one that did not.
                //
                // Two checks, because they catch different things: the ref covers
                // the whole composition from `compositionstart`, and the native flag
                // covers an `input` that arrives without one.
                if (isComposingTextRef.current) return;
                if ('isComposing' in event.nativeEvent && event.nativeEvent.isComposing) {
                  return;
                }
                if (lexicalDrivesTheStore()) return;
                syncComposerFromDom(event.target);
              }}
              onCompositionEndCapture={event => {
                // Re-open the gate before syncing: what the DOM holds now is what the
                // user committed, and it is the store's turn to catch up.
                //
                // Chromium emits a trailing `input` with `isComposing === false` that
                // the handler above picks up; WebKit does not, so on Safari the
                // committed text exists only here. Running in both is harmless -- the
                // second write carries the same string.
                isComposingTextRef.current = false;
                syncComposerFromDom(event.target);
              }}
              onKeyDownCapture={event => {
                if (event.key === 'Escape' && onEscape) {
                  event.preventDefault();
                  event.stopPropagation();
                  onEscape();
                  return;
                }
                const native = event.nativeEvent;
                if (
                  isComposingTextRef.current ||
                  native.isComposing ||
                  native.keyCode === 229 ||
                  ('which' in native && native.which === 229)
                ) {
                  event.preventDefault();
                  event.stopPropagation();
                }
              }}
              className="aui-composer-input caret-primary [&_.aui-lexical-placeholder]:text-muted-foreground/60 relative max-h-48 min-h-10 w-full resize-none bg-transparent px-2.5 py-1 text-base leading-6 outline-none [&_.aui-lexical-input]:min-h-lh [&_.aui-lexical-input]:outline-none [&_.aui-lexical-placeholder]:pointer-events-none [&_.aui-lexical-placeholder]:absolute [&_.aui-lexical-placeholder]:top-0 [&_.aui-lexical-placeholder]:right-0 [&_.aui-lexical-placeholder]:left-0 [&_.aui-lexical-placeholder]:truncate [&_.aui-lexical-placeholder]:px-2.5 [&_.aui-lexical-placeholder]:py-1"
              aria-label="Message input"
            />
            <ComposerAction model={model} onModelChange={onModelChange} />
          </div>
        </ComposerPrimitive.AttachmentDropzone>

        {HostComposerTriggers ? (
          <HostComposerTriggers />
        ) : (
          commands.length > 0 && (
            <ComposerTriggerPopover char="/" {...slash} emptyItemsLabel="No matching commands" />
          )
        )}
      </ComposerPrimitive.Root>
    </ComposerPrimitive.Unstable_TriggerPopoverRoot>
  );
};

const ComposerExtrasSlot: FC = () => {
  const { ComposerExtras } = useContext(ThreadComponentsContext);
  return ComposerExtras ? <ComposerExtras /> : null;
};

const ComposerAction: FC<{
  model: string | null;
  onModelChange?: (value: string | null, contextWindow?: number | null) => void;
}> = ({ model, onModelChange }) => {
  const aui = useAui();
  const composerText = useAuiState(state => state.composer.text);
  const {
    ComposerAddAttachment: HostComposerAddAttachment,
    hasComposerAttachments,
    onComposerAttachmentSend,
    ComposerIdleAction,
    ComposerRightExtras,
    onSwitchToMicCloud,
  } = useContext(ThreadComponentsContext);
  const isRunning = useAuiState(state => state.thread.isRunning);
  // Nothing to send: the primary slot goes to the host's idle control instead
  // of a Send button that would refuse the click anyway. Guarded on
  // `isRunning` by the surrounding `AuiIf`, so a streaming turn still shows
  // Cancel.
  const showIdleAction =
    !!ComposerIdleAction && composerText.trim().length === 0 && !hasComposerAttachments;
  return (
    <div className="aui-composer-action-wrapper relative flex items-center justify-between">
      <div className="flex min-w-0 items-center gap-1">
        {HostComposerAddAttachment ? <HostComposerAddAttachment /> : <ComposerAddAttachment />}
        <ChatSettingsPanel model={model} onModelChange={onModelChange} />
        <ComposerExtrasSlot />
      </div>
      <div className="flex items-center gap-1.5">
        {ComposerRightExtras ? <ComposerRightExtras /> : null}
        {onSwitchToMicCloud && (
          <TooltipIconButton
            tooltip="Voice mode"
            side="bottom"
            type="button"
            variant="ghost"
            size="icon"
            className="aui-composer-voice-mode text-muted-foreground hover:text-foreground size-7 rounded-full"
            aria-label="Voice mode"
            disabled={isRunning}
            onClick={onSwitchToMicCloud}>
            <MicIcon className="size-4" />
          </TooltipIconButton>
        )}
        {/*
          Permanently false, deliberately: `useOpenHumanExternalStore` supplies
          no `adapters.dictation`, and the reasoning for keeping it that way
          lives there. Short version — Web Speech's constructor exists in our
          WKWebView but `start()` never succeeds, and with the speech usage
          strings present it hangs silently rather than erroring, which would
          strand the composer in `dictation != null`. Working dictation already
          ships as the `mic-cloud` composer, whose "Voice mode" button is the
          one directly above this block.
        */}
        <AuiIf condition={s => s.thread.capabilities.dictation}>
          <AuiIf condition={s => s.composer.dictation == null}>
            <ComposerPrimitive.Dictate asChild>
              <TooltipIconButton
                tooltip="Voice input"
                side="bottom"
                type="button"
                variant="ghost"
                size="icon"
                className="aui-composer-dictate text-muted-foreground hover:text-foreground size-7 rounded-full"
                aria-label="Start voice input">
                <MicIcon className="aui-composer-dictate-icon size-4" />
              </TooltipIconButton>
            </ComposerPrimitive.Dictate>
          </AuiIf>
          <AuiIf condition={s => s.composer.dictation != null}>
            <ComposerPrimitive.StopDictation asChild>
              <TooltipIconButton
                tooltip="Stop dictation"
                side="bottom"
                type="button"
                variant="ghost"
                size="icon"
                className="aui-composer-stop-dictation text-destructive size-7 rounded-full"
                aria-label="Stop voice input">
                <SquareIcon className="aui-composer-stop-dictation-icon size-3.5 animate-pulse fill-current" />
              </TooltipIconButton>
            </ComposerPrimitive.StopDictation>
          </AuiIf>
        </AuiIf>
        <AuiIf condition={s => !s.thread.isRunning}>
          {showIdleAction ? (
            <ComposerIdleAction />
          ) : hasComposerAttachments && composerText.trim().length === 0 ? (
            // Pinned to `primary-500` rather than left on `variant="default"`.
            // That variant paints `bg-primary`, which `styles/shadcn-tokens.css`
            // aliases to `primary-500` in light but `primary-400` in DARK — a
            // pale sky blue. Its label is `--content-inverted`, which is white
            // in both themes (not actually inverted per theme), so in dark the
            // send button was white-on-pale-blue: washed out, and about 2.4:1,
            // which is below AA for a control. `primary-500` under white is
            // ~4.6:1 and reads as the accent in both themes.
            // Overriding here rather than repointing the dark `--primary`
            // alias: that token backs every `variant="default"` button in the
            // app, and dark-mode-lightens-the-accent is a defensible palette
            // choice to make deliberately, not as a side effect of fixing one
            // button. `cn` is tailwind-merge, so the later `bg-primary-500`
            // replaces the variant's `bg-primary` cleanly.
            <TooltipIconButton
              tooltip="Send message"
              side="bottom"
              type="button"
              variant="default"
              size="icon"
              className="aui-composer-send size-7 rounded-full bg-primary-500 text-content-inverted hover:bg-primary-600"
              data-testid="send-message-button"
              aria-label="Send message"
              onClick={() => {
                onComposerAttachmentSend?.();
                aui.composer.setText('');
              }}>
              <ArrowUpIcon className="aui-composer-send-icon size-4" />
            </TooltipIconButton>
          ) : (
            <ComposerPrimitive.Send asChild>
              <TooltipIconButton
                tooltip="Send message"
                side="bottom"
                type="button"
                variant="default"
                size="icon"
                className="aui-composer-send size-7 rounded-full bg-primary-500 text-content-inverted hover:bg-primary-600"
                data-testid="send-message-button"
                aria-label="Send message">
                <ArrowUpIcon className="aui-composer-send-icon size-4" />
              </TooltipIconButton>
            </ComposerPrimitive.Send>
          )}
        </AuiIf>
        <AuiIf condition={s => s.thread.isRunning}>
          <ComposerPrimitive.Cancel asChild>
            <Button
              type="button"
              variant="default"
              size="icon"
              className="aui-composer-cancel size-7 rounded-full bg-primary-500 text-content-inverted hover:bg-primary-600"
              data-testid="stop-generation-button"
              aria-label="Stop generating">
              <SquareIcon className="aui-composer-cancel-icon size-3.5 fill-current" />
            </Button>
          </ComposerPrimitive.Cancel>
        </AuiIf>
      </div>
    </div>
  );
};

/**
 * The runtime's error presentation for a failed message. The external-store
 * projection marks persisted `chat_error` rows as incomplete/error, so they
 * use this same card as errors raised directly by assistant-ui.
 *
 * `useMessageError` gives us the raw error value for the card. Failed turns
 * have no committed assistant reply id for `threads.regenerate`, so this card
 * must not offer Reload even when the runtime supports it for settled replies.
 */
const MessageError: FC = () => {
  const error = useMessageError();
  if (error === undefined) return null;
  const detail = typeof error === 'string' ? error : JSON.stringify(error);
  return (
    <MessagePrimitive.Error>
      <ErrorState
        className="aui-message-error-root mt-2"
        title="Something went wrong"
        detail={detail}
        retrying={false}
      />
    </MessagePrimitive.Error>
  );
};

/** A URL `source` part, e.g. a web fetch/search result. */
export type SourceUrlPart = { id: string; sourceType: 'url'; url: string; title?: string };
/** A document `source` part, e.g. a memory citation. */
export type SourceDocumentPart = { id: string; sourceType: 'document'; title?: string };
/** Either kind of `source` part this app emits. */
export type SourceItemPart = SourceUrlPart | SourceDocumentPart;

const selectMessageParts = (state: AssistantState) => state.message.parts;

/** Gives the host all source parts (`url` and `document`) represented by one grouped source node. */
const SourceGroupSlot: FC<{ Component: ComponentType<{ sources: readonly SourceItemPart[] }> }> = ({
  Component,
}) => {
  const parts = useAuiState(selectMessageParts);
  const sources = parts.flatMap((part): SourceItemPart[] => {
    if (part.type !== 'source') return [];
    if (part.sourceType === 'url') {
      return [
        {
          id: part.id,
          sourceType: 'url',
          url: part.url,
          ...(part.title ? { title: part.title } : {}),
        },
      ];
    }
    if (part.sourceType === 'document') {
      return [
        { id: part.id, sourceType: 'document', ...(part.title ? { title: part.title } : {}) },
      ];
    }
    return [];
  });
  return sources.length > 0 ? <Component sources={sources} /> : null;
};

/** Whether this message is a stopped/cancelled turn's partial reply. */
const isStoppedRun = (s: AssistantState): boolean =>
  s.message.status?.type === 'incomplete' && s.message.status.reason === 'cancelled';

/**
 * The stopped turn's own text, split into words for the vendored
 * `StoppedRun` element, plus `cancel_reason`/`superseded_by`
 * (wire-contract.md `chat_cancelled`, carried through
 * `metadata.custom.extraMetadata` by `assistantUiMessages.ts`) so the reason
 * chip can distinguish a user-initiated Stop from a turn the core superseded.
 */
// Two primitive selectors rather than one object-returning selector:
// `useAuiState`'s selector is compared by `Object.is`, so an inline `{...}`
// literal differs from itself on every store tick and free-runs the
// subscription — exactly the "Maximum update depth exceeded" loop this file
// hit once already. `words` (an array) is derived from `text` with
// `useMemo` in the component below instead of being computed here.
const selectStoppedRunText = (s: AssistantState): string =>
  s.message.parts.flatMap(part => (part.type === 'text' ? [part.text] : [])).join(' ');

const selectStoppedRunCancelReason = (s: AssistantState): string | undefined => {
  const custom = s.message.metadata?.custom as
    | { extraMetadata?: { cancelReason?: string; supersededBy?: string } }
    | undefined;
  return custom?.extraMetadata?.cancelReason;
};

/**
 * Renders in place of the normal part switch for a stopped/cancelled
 * assistant message (#4862 kept the raw text visible via a plain "Stopped"
 * label; this replaces that with the real vendored element). Continue re-runs
 * the turn through the same Reload capability `AssistantActionBar` uses;
 * Discard drops the partial reply from this client's view via `onDelete`
 * (`useOpenHumanExternalStore.ts` — the core keeps the persisted row, this
 * only stops showing it here).
 */
const StoppedRunSlot: FC = () => {
  const aui = useAui();
  const { t } = useT();
  const text = useAuiState(selectStoppedRunText);
  const cancelReason = useAuiState(selectStoppedRunCancelReason);
  const words = useMemo(() => (text.length > 0 ? text.split(/\s+/).filter(Boolean) : []), [text]);
  const { disabled: reloadDisabled, reload } = useActionBarReload();
  const reasonLabel =
    cancelReason === 'superseded'
      ? t('conversations.assistantUi.stoppedRun.reasonSuperseded')
      : t('conversations.assistantUi.stoppedRun.reasonUserStop');
  return (
    <StoppedRun
      data-testid="stopped-marker"
      words={words}
      reason={reasonLabel}
      onContinue={() => {
        if (!reloadDisabled) reload();
      }}
      onDiscard={() => aui.message.delete()}
      continueLabel={t('common.continue')}
      discardLabel={t('settings.ai.discard')}
    />
  );
};

const AssistantMessage: FC = () => {
  const {
    ToolFallback: ToolFallbackComponent = ToolFallback,
    ActivityGroup = DefaultActivityGroup,
    SourceGroup,
  } = useContext(ThreadComponentsContext);
  const stopped = useAuiState(isStoppedRun);

  const ACTION_BAR_PT = 'pt-1.5';
  // `min-h` reserves the bar's height (`pt-1.5` + a `size-6` button = 7.5) so a
  // bar revealed on hover does not shift the transcript, and `-mb` gives that
  // reservation back to the flow so it does not stack on top of the spacing the
  // message group already provides. Both MUST sit on this one element: the `-mb`
  // had drifted onto the root, where it only cancelled that element's own `pb`,
  // leaving the reservation uncompensated — a dead 30px band under every turn.
  //
  // The `-mb` step is `gap-y-6` from the message group, NOT the full `min-h`.
  // The bar is pulled into the inter-message gap and must stay inside it: give
  // back more than the gap and the bar's tail paints over the next message's
  // first line, which sits at the same left inset (`ms-2` here, `px-2` there).
  // So the bar occupies the gap exactly and the turns end up 7.5 apart.
  // Keep this in step with `aui_message-group`'s `gap-y-*`; the pairing is
  // asserted in `thread.actionBarSpacing.test.tsx`.
  const ACTION_BAR_HEIGHT = `-mb-6 min-h-7.5 ${ACTION_BAR_PT}`;
  // The root's own `-mb-7.5 pb-7.5` pair below is PAINT-ONLY and unrelated to
  // the above: `content-visibility:auto` implies `contain: paint`, so `pb`
  // widens the paint box to cover the bar that `-mb` pulls past the content
  // box, and the root's `-mb` cancels that padding again in flow.

  return (
    <MessagePrimitive.Root
      data-slot="aui_assistant-message-root"
      data-role="assistant"
      data-testid="agent-message"
      className="fade-in slide-in-from-bottom-1 animate-in relative -mb-7.5 pb-7.5 duration-150">
      {/*
       * One vertical rhythm for the whole message, rather than each part
       * bringing its own margin. Measured before this change the gaps ran
       * 16 / 0 / 0 / 16 / 0 / 0 px — `reasoning-root` carries `mb-4` and
       * nothing else carried anything, so a reasoning block sat apart while a
       * tool group and the prose beneath it touched. `[&>*+*]:mt-3` spaces
       * adjacent blocks evenly and the `mb-0` override neutralises the one
       * component with an opinion.
       *
       * Reasoning and tool calls share ONE group per run, in the order they
       * happened, so a turn reads input → work → answer. Splitting them into
       * reasoning and tool sub-groups turned an interleaved turn (think, call,
       * think, call) into a stack of unrelated collapsibles.
       */}
      <div
        data-slot="aui_assistant-message-content"
        className="text-foreground [&>*+*]:mt-3 [&_[data-slot=reasoning-root]]:mb-0 px-2 leading-relaxed wrap-break-word">
        <MessagePrimitive.GroupedParts
          groupBy={groupPartByType({
            reasoning: ['group-activity'],
            'tool-call': ['group-activity'],
            'standalone-tool-call': [],
            source: ['group-source'],
          })}>
          {({ part, children }) => {
            switch (part.type) {
              case 'group-activity':
                return <ActivityGroup group={part}>{children}</ActivityGroup>;
              case 'group-source':
                return SourceGroup ? <SourceGroupSlot Component={SourceGroup} /> : null;
              case 'text':
                // A stopped/cancelled turn's text renders once, inside
                // `StoppedRunSlot` below (as `words`), not here — see that
                // component's docstring.
                return stopped ? null : <MarkdownText />;
              case 'reasoning':
                // A step inside the activity group, not a disclosure of its own.
                return (
                  <div
                    data-slot="aui_activity-reasoning"
                    className="text-muted-foreground border-border border-s-2 ps-3 text-sm leading-relaxed">
                    <Reasoning {...part} />
                  </div>
                );
              case 'tool-call':
                return part.toolUI ?? <ToolFallbackComponent {...part} />;
              case 'data':
                return part.dataRendererUI;
              case 'file':
                return (
                  <div data-slot="aui_assistant-message-file" className="py-1">
                    <File {...part} />
                  </div>
                );
              case 'image':
                return (
                  <div data-slot="aui_assistant-message-image" className="py-1">
                    <Image {...part} />
                  </div>
                );
              case 'indicator':
                // The host RunningStatus slot renders the single shared loading
                // state below the message group. Rendering assistant-ui's raw
                // indicator part as well produces a second, disconnected dot.
                return null;
              default:
                return null;
            }
          }}
        </MessagePrimitive.GroupedParts>
        {stopped && <StoppedRunSlot />}
        <MessageError />
        <ChatErrorNotice />
      </div>

      <div
        data-slot="aui_assistant-message-footer"
        className={cn('ms-2 flex items-center', ACTION_BAR_HEIGHT)}>
        <BranchPicker />
        <AssistantActionBar />
      </div>
    </MessagePrimitive.Root>
  );
};

const AssistantActionBar: FC = () => {
  // assistant-ui's own disabled predicate for Reload is
  // `isRunning || isDisabled || role !== 'assistant'` — it never consults
  // `capabilities.reload`, so the button ships enabled on every settled
  // assistant message while the external-store adapter supplies no `onReload`
  // and the runtime throws on click.
  //
  // Hoisted to a `const` rather than written inline for the same coverage
  // reason as `editAction` in `UserActionBar`.
  const canReload = useAuiReloadCapability();
  const isFailedTurn = useAuiState(s => {
    if (s.message.status?.type === 'incomplete' && s.message.status.reason === 'error') return true;
    const custom = s.message.metadata?.custom as
      | { extraMetadata?: Record<string, unknown> }
      | undefined;
    return custom?.extraMetadata?.[CHAT_ERROR_METADATA_KEY] !== undefined;
  });
  const reloadAction =
    canReload && !isFailedTurn ? (
      <ActionBarPrimitive.Reload asChild>
        <TooltipIconButton tooltip="Refresh">
          <RefreshCwIcon />
        </TooltipIconButton>
      </ActionBarPrimitive.Reload>
    ) : null;

  return (
    <ActionBarPrimitive.Root
      hideWhenRunning
      autohide="not-last"
      className="aui-assistant-action-bar-root text-muted-foreground animate-in fade-in col-start-3 row-start-2 -ms-1 flex gap-1 duration-200">
      <ActionBarPrimitive.Copy asChild>
        <TooltipIconButton tooltip="Copy">
          <AuiIf condition={s => s.message.isCopied}>
            <CheckIcon className="animate-in zoom-in-50 fade-in duration-200 ease-out" />
          </AuiIf>
          <AuiIf condition={s => !s.message.isCopied}>
            <CopyIcon className="animate-in zoom-in-75 fade-in duration-150" />
          </AuiIf>
        </TooltipIconButton>
      </ActionBarPrimitive.Copy>
      {reloadAction}
      {/* Thumbs render only because the external store now supplies
          `adapters.feedback`; the runtime gates them on that key alone. The
          pressed state comes from `message.submittedFeedback`, which our message
          converter re-emits from the persisted rating — see the Defect A note
          there, without which a pressed thumb silently un-presses on the next
          store update.

          Deliberately NOT gated the way `reloadAction` above is. That gate
          exists because assistant-ui's Reload ignores `capabilities.reload` and
          the runtime *throws* on click when the adapter supplies no `onReload`.
          These primitives instead compute `disabled = disabled || !callback`
          from the adapter's own hook, so with no adapter they render disabled
          rather than throwing — and we supply `adapters.feedback`
          unconditionally, so they are always live here. */}
      <ActionBarPrimitive.FeedbackPositive asChild>
        <TooltipIconButton
          tooltip="Good response"
          data-testid="assistant-feedback-positive"
          className="data-[submitted=true]:text-primary-600 dark:data-[submitted=true]:text-primary-400">
          <ThumbsUpIcon />
        </TooltipIconButton>
      </ActionBarPrimitive.FeedbackPositive>
      <ActionBarPrimitive.FeedbackNegative asChild>
        <TooltipIconButton
          tooltip="Bad response"
          data-testid="assistant-feedback-negative"
          className="data-[submitted=true]:text-coral-600 dark:data-[submitted=true]:text-coral-400">
          <ThumbsDownIcon />
        </TooltipIconButton>
      </ActionBarPrimitive.FeedbackNegative>
      {/*
       * Read aloud, through the same TTS the chat mascot uses. Gated on the
       * capability rather than rendered unconditionally: `actionBarSpeakDisabled`
       * checks only the message's role and running status, NOT
       * `capabilities.speech`, so an ungated Speak button on a runtime with no
       * `adapters.speech` is enabled, clickable, and throws. The gate makes the
       * control appear exactly when it can work — the same rule `UserActionBar`
       * applies to Edit (#5897).
       */}
      <AuiIf condition={s => s.thread.capabilities.speech}>
        <AuiIf condition={s => s.message.speech == null}>
          <ActionBarPrimitive.Speak asChild>
            <TooltipIconButton tooltip="Read aloud">
              <Volume2Icon />
            </TooltipIconButton>
          </ActionBarPrimitive.Speak>
        </AuiIf>
        <AuiIf condition={s => s.message.speech != null}>
          <ActionBarPrimitive.StopSpeaking asChild>
            <TooltipIconButton tooltip="Stop reading">
              <VolumeXIcon className="text-destructive" />
            </TooltipIconButton>
          </ActionBarPrimitive.StopSpeaking>
        </AuiIf>
      </AuiIf>
      <ActionBarMorePrimitive.Root>
        <ActionBarMorePrimitive.Trigger asChild>
          <TooltipIconButton tooltip="More" className="data-[state=open]:bg-accent">
            <MoreHorizontalIcon />
          </TooltipIconButton>
        </ActionBarMorePrimitive.Trigger>
        <ActionBarMorePrimitive.Content
          side="bottom"
          align="start"
          sideOffset={6}
          className="aui-action-bar-more-content bg-popover text-popover-foreground data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95 data-[state=open]:animate-in data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95 data-[state=closed]:animate-out data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 z-50 min-w-32 overflow-hidden rounded-xl border p-1.5">
          <ActionBarPrimitive.ExportMarkdown asChild>
            <ActionBarMorePrimitive.Item className="aui-action-bar-more-item hover:bg-accent hover:text-accent-foreground focus:bg-accent focus:text-accent-foreground flex cursor-pointer items-center gap-2 rounded-lg px-2.5 py-1.5 text-sm outline-hidden select-none">
              <DownloadIcon className="size-4" />
              Export as Markdown
            </ActionBarMorePrimitive.Item>
          </ActionBarPrimitive.ExportMarkdown>
        </ActionBarMorePrimitive.Content>
      </ActionBarMorePrimitive.Root>
      {/*
       * Renders nothing until the stream completes and `chat_done.timing`
       * lands on `message.metadata.timing` (`assistantUiMessages.ts`); see
       * that element's own docstring for why it belongs inside this root.
       */}
      <MessageTiming />
    </ActionBarPrimitive.Root>
  );
};

const UserFilePart: FileMessagePartComponent = part => (
  <div data-slot="aui_user-message-file" className="py-1">
    <File {...part} />
  </div>
);

const UserImagePart: ImageMessagePartComponent = part => (
  <div data-slot="aui_user-message-image" className="py-1">
    <Image {...part} />
  </div>
);

const UserMessage: FC = () => {
  return (
    <MessagePrimitive.Root
      data-slot="aui_user-message-root"
      className="fade-in slide-in-from-bottom-1 animate-in grid auto-rows-auto grid-cols-[minmax(72px,1fr)_auto] content-start gap-y-2 px-2 duration-150 [&:where(>*)]:col-start-2"
      data-role="user">
      <UserMessageAttachments />

      <div className="aui-user-message-content-wrapper relative col-start-2 min-w-0">
        <div className="aui-user-message-content peer bg-muted text-foreground rounded-xl px-4 py-2 wrap-break-word empty:hidden">
          {/* `Text: DirectiveText` because the composer can put directive syntax
              into a user message without anyone opting in. The `/` popover is
              built from `unstable_useSlashCommandAdapter`, which returns an
              `action` behaviour and sets no `removeOnExecute`; the runtime's
              `triggerSelectionResource` then takes `else insertDirective()`,
              replacing the typed `/clear` with `formatter.serialize(item)` —
              `:command[/clear]{name=clear}` — as an audit-trail chip. Without a
              `Text` component here that renders as raw syntax and is sent to the
              model verbatim. Assistant text is unaffected: it renders through
              `MarkdownText` on the part switch below, a different slot. */}
          <MessagePrimitive.Parts
            components={{ Text: DirectiveText, File: UserFilePart, Image: UserImagePart }}
          />
        </div>
        <div className="aui-user-action-bar-wrapper absolute inset-s-0 top-1/2 -translate-x-full -translate-y-1/2 pe-2 peer-empty:hidden rtl:translate-x-full">
          <UserActionBar />
        </div>
      </div>

      <BranchPicker
        data-slot="aui_user-branch-picker"
        className="col-span-full col-start-1 row-start-3 -me-1 justify-end"
      />
    </MessagePrimitive.Root>
  );
};

const UserActionBar: FC = () => {
  // Edit is offered only when the bound runtime can honour it. The
  // external-store adapter supplies `onNew` / `onCancel` and neither `onEdit`
  // nor `setMessages`, so assistant-ui reports `edit: false` and
  // `EditComposer` below never renders — the button was clickable and did
  // nothing (#5897).
  //
  // Gated on the capability rather than hard-coded off, so the affordance
  // appears by itself the day the adapter grows `onEdit`.
  const { canEdit } = useAuiEditCapabilities();

  // Hoisted out of the JSX rather than written as `{canEdit && (…)}` inline: a
  // bare JSX logical expression emits no coverage record on its own line, so
  // `diff-cover` reported the gate as an uncovered changed line even while the
  // v8 report showed the surrounding function fully exercised. As a `const` it
  // is an ordinary statement, instrumented like any other.
  const editAction = canEdit ? (
    <ActionBarPrimitive.Edit asChild>
      <TooltipIconButton tooltip="Edit" className="aui-user-action-edit">
        <PencilIcon />
      </TooltipIconButton>
    </ActionBarPrimitive.Edit>
  ) : null;

  return (
    <ActionBarPrimitive.Root
      hideWhenRunning
      autohide="not-last"
      className="aui-user-action-bar-root flex flex-col items-end">
      <ActionBarPrimitive.Copy asChild>
        <TooltipIconButton tooltip="Copy response" title="Copy response">
          <CopyIcon />
        </TooltipIconButton>
      </ActionBarPrimitive.Copy>
      {editAction}
    </ActionBarPrimitive.Root>
  );
};

/**
 * How many later turns editing this message would discard — `onEdit`
 * (`useOpenHumanExternalStore.ts`) truncates the thread's single lineage from
 * this message on, exactly like `onReload`, so every message after it (not
 * just its direct reply) is what a Send here throws away. `s.message.index`
 * is the position `MessageState` already tracks;
 * `s.thread.messages.length - 1 - index` is everything after it. Kept as its
 * own primitive-returning selector (a plain number), never combined with
 * `value` below into one object literal — `useAuiState`'s selector is
 * compared by `Object.is`, so an object literal differs from itself on every
 * store tick and free-runs the subscription (the "Maximum update depth
 * exceeded" loop this file hit once already).
 */
const selectDiscardedReplies = (s: AssistantState): number =>
  Math.max(0, s.thread.messages.length - 1 - s.message.index);

const EditComposer: FC = () => {
  const aui = useAui();
  const { t } = useT();
  const value = useAuiState(s => s.composer.text);
  const discardedReplies = useAuiState(selectDiscardedReplies);
  return (
    <MessagePrimitive.Root data-slot="aui_edit-composer-wrapper" className="flex flex-col px-2">
      <EditMessage
        className="ms-auto"
        value={value}
        discardedReplies={discardedReplies}
        editing
        onValueChange={text => aui.message.composer().setText(text)}
        onSave={() => aui.message.composer().send()}
        onCancel={() => aui.message.composer().cancel()}
        cancelLabel={t('common.cancel')}
        sendLabel={t('chat.elicitation.send')}
        editAriaLabel={t('conversations.assistantUi.edit.ariaLabel')}
        discardedRepliesText={count =>
          t(
            count === 1
              ? 'conversations.assistantUi.edit.discardedRepliesOne'
              : 'conversations.assistantUi.edit.discardedRepliesOther'
          ).replace('{count}', String(count))
        }
      />
    </MessagePrimitive.Root>
  );
};

const BranchPicker: FC<BranchPickerPrimitive.Root.Props> = ({ className, ...rest }) => {
  // The same defect class as the Edit button above, one step from biting: this
  // is rendered unconditionally at both call sites and is invisible today only
  // because `hideWhenSingleBranch` happens to hold — the adapter implements no
  // `setMessages`, so there is never more than one branch. That is
  // assistant-ui's guard doing the work this app intended to do itself, and it
  // would become a second dead control if the prop ever went away.
  const { canSwitchToBranch } = useAuiEditCapabilities();
  if (!canSwitchToBranch) return null;

  return (
    <BranchPickerPrimitive.Root
      hideWhenSingleBranch
      className={cn(
        'aui-branch-picker-root text-muted-foreground -ms-2 me-2 inline-flex items-center text-xs',
        className
      )}
      {...rest}>
      <BranchPickerPrimitive.Previous asChild>
        <TooltipIconButton tooltip="Previous">
          <ChevronLeftIcon />
        </TooltipIconButton>
      </BranchPickerPrimitive.Previous>
      <span className="aui-branch-picker-state font-medium">
        <BranchPickerPrimitive.Number /> / <BranchPickerPrimitive.Count />
      </span>
      <BranchPickerPrimitive.Next asChild>
        <TooltipIconButton tooltip="Next">
          <ChevronRightIcon />
        </TooltipIconButton>
      </BranchPickerPrimitive.Next>
    </BranchPickerPrimitive.Root>
  );
};
