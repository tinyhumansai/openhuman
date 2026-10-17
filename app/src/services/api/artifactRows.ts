/**
 * Artifact rows in a thread's message list.
 *
 * The core writes one row per artifact made in a chat (`type: "artifact"`,
 * `extraMetadata: { artifactId, requestId }`; see the core's
 * `threads::artifact_rows`). The row records which turn made the artifact, so
 * the card renders inside that turn's reply on every load, on every device.
 *
 * Rows are never shown as messages. {@link attachArtifactRows} takes them out
 * of the list and records each artifact id on the reply it belongs to, under
 * `extraMetadata[ARTIFACT_IDS_METADATA_KEY]`, where `MessageArtifactCards`
 * reads it.
 */
import type { ThreadMessage } from '../../types/thread';

export const ARTIFACT_MESSAGE_TYPE = 'artifact';

/** Reply metadata key listing the artifacts whose cards render in that reply. */
export const ARTIFACT_IDS_METADATA_KEY = 'artifactIds';

function stringField(metadata: Record<string, unknown> | undefined, key: string) {
  const value = metadata?.[key];
  return typeof value === 'string' && value ? value : undefined;
}

/**
 * The reply an artifact row belongs to: the last agent message of the row's
 * turn, or, for a row without a turn, the first agent message saved at or after
 * the artifact. A backfilled row is appended at the end of the log but keeps
 * its artifact's time, so a row without a turn is placed by time, and by
 * position only when a time cannot be read.
 */
function replyIndexFor(messages: readonly ThreadMessage[], rowIndex: number, requestId?: string) {
  // Rows are agent messages too; a row is never a reply.
  const isReply = (message: ThreadMessage) =>
    message.sender === 'agent' && message.type !== ARTIFACT_MESSAGE_TYPE;
  if (requestId) {
    for (let i = messages.length - 1; i >= 0; i--) {
      const message = messages[i];
      if (isReply(message) && stringField(message.extraMetadata, 'requestId') === requestId)
        return i;
    }
    return -1;
  }
  const madeAt = Date.parse(messages[rowIndex].createdAt);
  if (!Number.isNaN(madeAt)) {
    return messages.findIndex(message => {
      const savedAt = Date.parse(message.createdAt);
      return isReply(message) && !Number.isNaN(savedAt) && savedAt >= madeAt;
    });
  }
  return messages.findIndex((message, i) => i > rowIndex && isReply(message));
}

/**
 * `messages` without artifact rows, each row's artifact id recorded on its
 * reply. A row whose reply is not in the list (its turn is still running) is
 * dropped; the live artifact event places that card meanwhile. Messages that
 * gain no artifact keep their identity.
 */
export function attachArtifactRows(messages: readonly ThreadMessage[]): ThreadMessage[] {
  if (!messages.some(message => message.type === ARTIFACT_MESSAGE_TYPE)) return [...messages];

  const visible: ThreadMessage[] = [];
  // Index in `messages` → index in `visible`, so a row's reply can be found in
  // the original order (a row without a turn is placed by position).
  const visibleIndex = new Map<number, number>();
  messages.forEach((message, i) => {
    if (message.type === ARTIFACT_MESSAGE_TYPE) return;
    visibleIndex.set(i, visible.length);
    visible.push(message);
  });

  const idsByReply = new Map<number, string[]>();
  messages.forEach((message, i) => {
    if (message.type !== ARTIFACT_MESSAGE_TYPE) return;
    const artifactId = stringField(message.extraMetadata, 'artifactId');
    if (!artifactId) return;
    const reply = replyIndexFor(messages, i, stringField(message.extraMetadata, 'requestId'));
    const target = visibleIndex.get(reply);
    if (target === undefined) return;
    const ids = idsByReply.get(target) ?? [];
    if (!ids.includes(artifactId)) ids.push(artifactId);
    idsByReply.set(target, ids);
  });

  for (const [index, ids] of idsByReply) {
    const reply = visible[index];
    const existing = reply.extraMetadata?.[ARTIFACT_IDS_METADATA_KEY];
    const merged = Array.isArray(existing)
      ? [...new Set([...existing.filter((id): id is string => typeof id === 'string'), ...ids])]
      : ids;
    visible[index] = {
      ...reply,
      extraMetadata: { ...reply.extraMetadata, [ARTIFACT_IDS_METADATA_KEY]: merged },
    };
  }
  return visible;
}
