/**
 * The AI-elements layer — chat-surface components adapted from
 * vercel/ai-elements (Apache-2.0) onto this app's own primitives and tokens.
 *
 * Same rule as `components/ui`: if a component exists here, import it from
 * `components/ai-elements` rather than reaching into the file. These are
 * compound components — a root plus its slots — so each family is exported as
 * a group.
 *
 * WHAT IS NOT HERE, AND WHY. The port originally landed ten families. Eight of
 * them — Artifact, ChainOfThought, Confirmation, Plan, Reasoning, Suggestion,
 * Task and Tool — were written, tested, exported and then imported by nothing
 * outside the dev gallery. That state is worse than absence: a component that
 * renders nowhere reads as "already migrated" in every audit while the
 * hand-rolled UI it was meant to replace stays in place, and its passing tests
 * give false confidence about code no user reaches. Each was checked against
 * this product's real transcript surfaces and deleted rather than force-fitted:
 *
 * - `Reasoning` / `ChainOfThought` — this product renders the agent's thinking
 *   INLINE at the position it streamed (the assistant-ui delegation transcript,
 *   explicitly "no heading, no collapse"; `ProcessingTranscriptView`'s
 *   interleaved narration). Both upstream components are whole-panel
 *   collapsibles that hide that trail behind one "Thought for N seconds"
 *   summary — the opposite of the chosen design.
 * - `Task` / `Plan` — `ThreadTodoStrip` and `PlanReviewCard` are the real
 *   surfaces. The strip wants a plain `ui/Collapsible`, not `Task`'s
 *   search-icon trigger and bordered rail; the review card is a blocking
 *   decision surface that must not be collapsible at all.
 * - `Confirmation` — models the AI SDK's tool-part approval lifecycle.
 *   `ApprovalRequestCard` resolves approvals over an RPC with optimistic
 *   dispatch; the state machines do not correspond.
 * - `Artifact` — a side-panel document-viewer shell. `ArtifactCard` is a
 *   compact inline in_progress/ready/failed status row with a download action.
 * - `Suggestion` — there are no suggested-reply chips in this product.
 * - `Tool` — `ToolTimelineBlock` carries run-loop coalescing that upstream has
 *   no equivalent for and stays. Nothing else needs a tool-call renderer.
 *
 * If one of these is wanted later, it is a `git log` away — but it should come
 * back with a caller in the same change.
 */

// Web sources the agent visited
export {
  Source,
  Sources,
  SourcesContent,
  SourcesTrigger,
  type SourceProps,
  type SourcesContentProps,
  type SourcesProps,
  type SourcesTriggerProps,
} from './Sources';

// Transcript shell
export {
  Conversation,
  ConversationContent,
  type ConversationContentProps,
  type ConversationProps,
} from './Conversation';
export {
  Message,
  MessageAction,
  MessageActions,
  MessageContent,
  type MessageActionProps,
  type MessageActionsProps,
  type MessageContentProps,
  type MessageProps,
  type MessageRole,
} from './Message';

// Icons
export { BookIcon, ChevronDownIcon } from './icons';
