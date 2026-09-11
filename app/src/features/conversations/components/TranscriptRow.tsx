import { memo } from 'react';

import { Message, MessageAction, MessageContent } from '../../../components/ai-elements';
import { parseMessageImages } from '../../../lib/attachments';
import { unwrapToolCallEnvelope } from '../../../lib/chat/toolCallEnvelope';
import { useT } from '../../../lib/i18n/I18nContext';
import type { ProcessingTranscriptItem, ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import type { ThreadMessage } from '../../../types/thread';
import { splitAgentMessageIntoBubbles } from '../../../utils/agentMessageBubbles';
import { ShareMessageButton } from '../../share/ShareMessageButton';
import { type AgentBubblePosition, formatRelativeTime } from '../utils/format';
import { AgentMessageBubble, AgentMessageText } from './AgentMessageBubble';
import { CitationChips, type MessageCitation } from './CitationChips';
import { PastTurnInsights } from './PastTurnInsights';
import { UserTurnBody } from './UserTurnBody';

/** Emoji offered by the inline reaction picker. */
const REACTION_EMOJI = ['👍', '❤️', '😂', '🔥', '👀', '🎯'] as const;

/** The persisted process trail of an older, settled turn. */
export interface PastTurnAnchor {
  entries: ToolTimelineEntry[];
  transcript: ProcessingTranscriptItem[];
}

export interface TranscriptRowProps {
  /** The turn this row renders. */
  msg: ThreadMessage;
  /** Thread the turn belongs to — threaded into the share affordance. */
  threadId: string | null;
  /** `'text'` renders one flowing answer; `'bubbles'` splits it into chat bubbles. */
  agentMessageViewMode: 'text' | 'bubbles';
  /** True for the newest visible turn, which alone shows its timestamp and
   *  reaction affordance. Passed as a boolean rather than as "the latest
   *  message" so a new tail does not invalidate every other row's props. */
  isLatestVisible: boolean;
  /** True while this row's copy affordance is showing its "copied" tick. */
  isCopied: boolean;
  /** True while this row owns the open reaction picker. */
  isReactionPickerOpen: boolean;
  /** This turn's restored process trail, or `undefined` if it has none. */
  pastTurn?: PastTurnAnchor;
  /** Agent display name for the share affordance. */
  shareAgentName: string;
  /** Copy the turn's plain text. Must be referentially stable. */
  onCopy: (messageId: string, content: string) => void;
  /** Toggle a reaction on the turn. Must be referentially stable. */
  onReact: (messageId: string, emoji: string) => void;
  /** Open (`messageId`) or close (`null`) the reaction picker. Must be stable. */
  onOpenReactionPicker: (messageId: string | null) => void;
}

/** Shallow one-level comparison of a message's metadata bag. */
function sameMetadata(
  a: ThreadMessage['extraMetadata'],
  b: ThreadMessage['extraMetadata']
): boolean {
  if (a === b) return true;
  if (!a || !b) return false;
  const keysA = Object.keys(a);
  if (keysA.length !== Object.keys(b).length) return false;
  return keysA.every(key => a[key] === b[key]);
}

/**
 * Two messages render identically.
 *
 * Deliberately compares FIELDS, not object identity. Redux keeps a settled
 * message's identity stable, but a re-hydrated transcript (a reconnect, a
 * refetch, a test that rebuilds its fixture) hands back structurally identical
 * objects with fresh identities — and identity-only memoization would re-render
 * and re-parse the entire transcript on each of those, which is exactly the
 * cost this row exists to avoid.
 */
export function sameThreadMessage(a: ThreadMessage, b: ThreadMessage): boolean {
  return (
    a === b ||
    (a.id === b.id &&
      a.sender === b.sender &&
      a.type === b.type &&
      a.content === b.content &&
      a.createdAt === b.createdAt &&
      sameMetadata(a.extraMetadata, b.extraMetadata))
  );
}

function TranscriptRowImpl({
  msg,
  threadId,
  agentMessageViewMode,
  isLatestVisible,
  isCopied,
  isReactionPickerOpen,
  pastTurn,
  shareAgentName,
  onCopy,
  onReact,
  onOpenReactionPicker,
}: TranscriptRowProps) {
  const { t } = useT();

  const isAgentTextMode = msg.sender === 'agent' && agentMessageViewMode === 'text';
  // B25: an agent turn that both talks AND calls a tool can leak the provider
  // wire-format `{ content, tool_calls }` JSON envelope as its raw `content`
  // (see `unwrapToolCallEnvelope`). Unwrap agent messages to the human text so
  // no surface (home chat OR the workflow copilot, which shares this renderer)
  // ever paints raw JSON. Shape-based + a strict passthrough for ordinary
  // prose, so this is a no-op for every non-envelope message — the tool
  // activity itself renders via the timeline, so the extracted tool names are
  // intentionally dropped here.
  //
  // This call is the reason the row is memoized. `unwrapToolCallEnvelope` runs
  // a `JSON.parse` in a `try/catch`, and for ordinary prose that parse THROWS
  // and is caught. Left in the parent's `.map()` body it re-ran for every
  // settled message on every streamed token — O(N) throwaway work per token.
  // Pinned by `ChatThreadView.renderPerf.test.tsx`.
  const displayContent =
    msg.sender === 'agent' ? unwrapToolCallEnvelope(msg.content ?? '').text : (msg.content ?? '');
  // Parsed once per message: for current messages (extraMetadata present, or
  // agent messages) the content already has no markers, so this is a no-op. For
  // legacy persisted user messages with raw [IMAGE:...]/[FILE:...] markers and
  // no extraMetadata, this is what keeps the marker text out of both the
  // rendered bubble and the copy-to-clipboard action.
  const parsedContent = parseMessageImages(displayContent);

  return (
    <>
      {/* Past-turn process trail (Phase 5 + restore-fidelity fix 1):
          each older settled turn's interleaved reasoning/narration +
          tool steps (and restored sub-agent transcripts), collapsed,
          above the answer it produced. Falls back to tool-cards-only
          for legacy snapshots with no persisted transcript. */}
      {pastTurn ? (
        <div data-testid="past-turn-insights">
          <PastTurnInsights entries={pastTurn.entries} transcript={pastTurn.transcript} />
        </div>
      ) : null}
      <div>
        <Message from={msg.sender} data-testid="chat-message-row" data-sender={msg.sender}>
          <MessageContent className={isAgentTextMode ? 'w-full max-w-full' : 'w-fit max-w-[75%]'}>
            {msg.sender === 'agent' ? (
              <div className="space-y-1" data-testid="agent-message">
                <div className="relative space-y-1">
                  {agentMessageViewMode === 'text' ? (
                    <AgentMessageText content={displayContent} />
                  ) : (
                    splitAgentMessageIntoBubbles(displayContent).map((segment, index, parts) => {
                      const position: AgentBubblePosition =
                        parts.length === 1
                          ? 'single'
                          : index === 0
                            ? 'first'
                            : index === parts.length - 1
                              ? 'last'
                              : 'middle';

                      return (
                        <AgentMessageBubble
                          key={`${msg.id}:${index}`}
                          content={segment}
                          position={position}
                        />
                      );
                    })
                  )}
                  {/* Reaction affordance — the closed "+", the open picker,
                      and the resulting reaction chips all live here, tucked
                      onto the bubble's bottom-left corner so the control
                      never jumps to a separate row below the timestamp. */}
                  {isLatestVisible && (
                    <ReactionRail
                      messageId={msg.id}
                      myReactions={(msg.extraMetadata?.myReactions as string[] | undefined) ?? []}
                      pickerOpen={isReactionPickerOpen}
                      onReact={onReact}
                      onOpenReactionPicker={onOpenReactionPicker}
                    />
                  )}
                </div>
                {/* Stopped marker (#4862): the partial reply that was
                    preserved when the user hit Stop / ESC mid-stream. */}
                {msg.extraMetadata?.stopped === true && (
                  <p
                    data-testid="stopped-marker"
                    className="flex items-center gap-1 px-1 text-[10px] font-medium text-content-faint">
                    <svg
                      className="h-2.5 w-2.5"
                      fill="currentColor"
                      viewBox="0 0 24 24"
                      aria-hidden>
                      <rect x="6" y="6" width="12" height="12" rx="1.5" />
                    </svg>
                    {t('chat.stoppedByUser')}
                  </p>
                )}
                <MessageCitations raw={msg.extraMetadata?.citations} />
                {isLatestVisible && (
                  <p className="px-1 text-[10px] text-content-faint">
                    {formatRelativeTime(msg.createdAt)}
                  </p>
                )}
              </div>
            ) : (
              <UserTurnBody
                msg={msg}
                displayText={parsedContent.text}
                fallbackDataUris={parsedContent.dataUris}
                showTime={isLatestVisible}
              />
            )}
            <MessageAction
              analyticsId="chat-message-copy"
              label={t('chat.copyResponse')}
              onClick={() => onCopy(msg.id, parsedContent.text)}
              className={`absolute -top-1 ${
                isAgentTextMode ? 'right-0' : msg.sender === 'user' ? '-left-8' : '-right-8'
              } opacity-0 group-hover/msg:opacity-100 text-content-faint hover:text-content-secondary`}>
              {isCopied ? (
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
            </MessageAction>
            {msg.sender === 'agent' && (
              <ShareMessageButton
                content={parsedContent.text}
                agentName={shareAgentName}
                threadId={threadId ?? undefined}
                className={`absolute top-6 ${isAgentTextMode ? 'right-0' : '-right-8'}`}
              />
            )}
          </MessageContent>
        </Message>
      </div>
    </>
  );
}

/** Citation chips, when the turn carries well-formed citation metadata. */
function MessageCitations({ raw }: { raw: unknown }) {
  if (!Array.isArray(raw)) return null;
  const citations = raw.filter(
    (item): item is MessageCitation =>
      typeof item === 'object' &&
      item !== null &&
      typeof (item as MessageCitation).id === 'string' &&
      typeof (item as MessageCitation).key === 'string' &&
      typeof (item as MessageCitation).snippet === 'string' &&
      typeof (item as MessageCitation).timestamp === 'string'
  );
  if (citations.length === 0) return null;
  return <CitationChips citations={citations} />;
}

interface ReactionRailProps {
  messageId: string;
  myReactions: string[];
  pickerOpen: boolean;
  onReact: (messageId: string, emoji: string) => void;
  onOpenReactionPicker: (messageId: string | null) => void;
}

function ReactionRail({
  messageId,
  myReactions,
  pickerOpen,
  onReact,
  onOpenReactionPicker,
}: ReactionRailProps) {
  const { t } = useT();
  return (
    <div className="absolute -bottom-2 left-3 z-10 flex items-center gap-1">
      {myReactions.map(emoji => (
        <button
          key={emoji}
          type="button"
          data-analytics-id="chat-message-reaction-remove"
          onClick={() => onReact(messageId, emoji)}
          className="flex items-center rounded-full border border-primary-200 bg-primary-100 px-1.5 text-xs leading-normal shadow-xs transition-colors hover:bg-primary-200 dark:border-primary-400/40 dark:bg-primary-500/25"
          title={t('chat.removeReaction').replace('{emoji}', emoji)}>
          {emoji}
        </button>
      ))}
      {pickerOpen ? (
        <div className="flex items-center gap-0.5 rounded-full bg-surface px-1 py-0.5 shadow-xs ring-1 ring-line dark:ring-line-strong">
          {REACTION_EMOJI.map(emoji => (
            <button
              key={emoji}
              type="button"
              data-analytics-id="chat-message-reaction-pick"
              onClick={() => {
                onReact(messageId, emoji);
                onOpenReactionPicker(null);
              }}
              className="rounded px-0.5 text-sm transition-transform hover:scale-125"
              title={emoji}>
              {emoji}
            </button>
          ))}
          <button
            type="button"
            data-analytics-id="chat-message-reaction-close"
            onClick={() => onOpenReactionPicker(null)}
            aria-label={t('common.close')}
            className="ml-0.5 px-0.5 text-xs text-content-secondary hover:text-content-faint dark:hover:text-content-faint">
            ✕
          </button>
        </div>
      ) : (
        <button
          type="button"
          data-analytics-id="chat-message-reaction-open"
          onClick={() => onOpenReactionPicker(messageId)}
          className="flex h-[18px] items-center rounded-full bg-surface px-1.5 text-xs leading-none text-content-muted opacity-0 shadow-xs ring-1 ring-line transition-opacity hover:bg-surface-hover hover:text-content-secondary group-hover/msg:opacity-100 dark:ring-line-strong"
          title={t('chat.addReaction')}
          aria-label={t('chat.addReaction')}>
          +
        </button>
      )}
    </div>
  );
}

/**
 * One transcript row, memoized.
 *
 * The transcript re-renders on every streamed token (the live preview reads
 * `streamingAssistantByThread`), so a settled row must be able to bail out.
 * The comparator is field-wise on `msg` — see `sameThreadMessage` — and
 * identity-wise on everything else, which is why every callback prop the parent
 * passes has to be referentially stable and why the "is this the latest
 * message?" / "is the picker open?" questions are answered as booleans by the
 * parent instead of being handed down as the ids to compare against.
 *
 * This component deliberately consumes NO Redux context: a `useSelector` or
 * `useDispatch` here would subscribe it to the store, and a context change
 * re-renders consumers regardless of `memo`.
 */
export const TranscriptRow = memo(TranscriptRowImpl, (prev, next) => {
  return (
    prev.threadId === next.threadId &&
    prev.agentMessageViewMode === next.agentMessageViewMode &&
    prev.isLatestVisible === next.isLatestVisible &&
    prev.isCopied === next.isCopied &&
    prev.isReactionPickerOpen === next.isReactionPickerOpen &&
    prev.pastTurn === next.pastTurn &&
    prev.shareAgentName === next.shareAgentName &&
    prev.onCopy === next.onCopy &&
    prev.onReact === next.onReact &&
    prev.onOpenReactionPicker === next.onOpenReactionPicker &&
    sameThreadMessage(prev.msg, next.msg)
  );
});
TranscriptRow.displayName = 'TranscriptRow';
