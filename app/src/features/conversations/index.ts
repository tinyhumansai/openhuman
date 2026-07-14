/**
 * Public surface for the conversations feature — the reusability contract other
 * parts of the app import against (see `docs/plans/conversations-timeline-refactor.md`).
 *
 * Today this is the monolithic `Conversations` panel plus its page-variant
 * wrapper and pure helpers. Later phases add the split shells
 * (`ConversationsSidebar`) and the `ConversationTimeline` renderer here.
 */
export {
  default as Conversations,
  ConversationsPage,
  isComposerInteractionBlocked,
  isImeCompositionKeyEvent,
  formatThreadLoadError,
} from './Conversations';
