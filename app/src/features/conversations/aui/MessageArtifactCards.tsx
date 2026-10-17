'use client';

import { useAuiState } from '@assistant-ui/react';
import { useMemo } from 'react';

import { ARTIFACT_IDS_METADATA_KEY } from '../../../services/api/artifactRows';
import { type ArtifactSnapshot, openArtifact } from '../../../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { ArtifactCardAdapter } from './ArtifactCardAdapter';

/**
 * Renders artifact cards inside the assistant message that produced them.
 *
 * - **Saved replies** list their artifacts under
 *   `extraMetadata.artifactIds`, attached from the core's artifact rows when the
 *   thread's messages load (`services/api/artifactRows.ts`). This is the
 *   durable placement: it survives refreshes, cleared storage and other
 *   devices.
 * - **The turn in flight**, or one that just finished, has no rows yet (the
 *   list reloads with them on the next load), so its cards come from the
 *   artifact events, which name the turn (`ownerRequestId`). A reply that has
 *   rows ignores owners: the rows are the record, and an owner can be stale.
 *
 * Only artifacts WITHOUT an owning tool call (`!toolCallId`) are shown here —
 * ones WITH a tool call id render inline next to their tool call via
 * `MediaAndDocumentCalls`.
 */
export function MessageArtifactCards() {
  // Message metadata lives at different paths for live vs settled messages:
  // - live streaming message: metadata.custom
  // - settled message: metadata.custom.extraMetadata
  const meta = useAuiState(s => s.message.metadata?.custom as Record<string, unknown> | undefined);
  const extra = meta?.extraMetadata as Record<string, unknown> | undefined;
  const requestId =
    typeof meta?.requestId === 'string'
      ? meta.requestId
      : typeof extra?.requestId === 'string'
        ? extra.requestId
        : undefined;
  const rawIds = extra?.[ARTIFACT_IDS_METADATA_KEY];
  const artifactIdsKey = Array.isArray(rawIds)
    ? rawIds.filter((id): id is string => typeof id === 'string').join('\n')
    : '';
  const dispatch = useAppDispatch();

  const artifactsByThread = useAppSelector(s => s.chatRuntime.artifactsByThread);
  // An artifact id or request id belongs to one thread, so look across every
  // thread rather than depend on which one is selected.
  const cards = useMemo(() => {
    const listed = artifactIdsKey ? artifactIdsKey.split('\n') : [];
    // Rows when the reply has them; the live owner only until it does.
    const byOwner = listed.length === 0 ? requestId : undefined;
    if (listed.length === 0 && !byOwner) return [];
    const shown = new Map<string, ArtifactSnapshot>();
    for (const list of Object.values(artifactsByThread)) {
      for (const a of list) {
        if (a.toolCallId || (a.status !== 'in_progress' && a.status !== 'ready')) continue;
        if (listed.includes(a.artifactId) || (byOwner && a.ownerRequestId === byOwner)) {
          shown.set(a.artifactId, a);
        }
      }
    }
    // Rows first, in the order the turn made them.
    return [...shown.values()].sort(
      (a, b) => indexOrEnd(listed, a.artifactId) - indexOrEnd(listed, b.artifactId)
    );
  }, [artifactsByThread, artifactIdsKey, requestId]);

  if (cards.length === 0) return null;

  return (
    <div className="mt-3 flex flex-col gap-2" data-testid="message-artifact-cards">
      {cards.map(artifact => (
        <ArtifactCardAdapter
          key={artifact.artifactId}
          artifact={artifact}
          onOpenPreview={id => dispatch(openArtifact(id))}
        />
      ))}
    </div>
  );
}

export default MessageArtifactCards;

function indexOrEnd(list: readonly string[], id: string): number {
  const index = list.indexOf(id);
  return index === -1 ? list.length : index;
}
