import type { ThreadWireTypes } from './threadWire.generated';

export type Thread = ThreadWireTypes['thread'];

/** UI narrows the core's open sender string to the roles it renders. */
export type ThreadMessage = Omit<ThreadWireTypes['message'], 'sender'> & {
  sender: 'user' | 'agent';
};

export type ThreadsListData = ThreadWireTypes['threadsList'];

export type ThreadMessagesData = Omit<ThreadWireTypes['threadMessages'], 'messages'> & {
  messages: ThreadMessage[];
};

export type ThreadDeleteData = ThreadWireTypes['threadDelete'];

export type PurgeResultData = ThreadWireTypes['purgeResult'];

/** One message matched by `openhuman.threads_search` (global search). */
export type ThreadSearchHit = ThreadWireTypes['threadSearchHit'];
