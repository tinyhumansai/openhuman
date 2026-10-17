import { useEffect } from 'react';

import { ARTIFACT_IDS_METADATA_KEY } from '../../../services/api/artifactRows';
import { listArtifactsForThread } from '../../../services/artifactDownloadService';
import { upsertArtifactReadyForThread } from '../../../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';

/**
 * Load a thread's ready artifacts from the core into
 * `chatRuntime.artifactsByThread` when the thread is opened (#3024), so the
 * artifact cards, the preview panel and the `@` mention list have them after
 * a restart or on a thread the app has not seen yet.
 *
 * Where each card renders comes from the thread's artifact rows, not from
 * here (`services/api/artifactRows.ts`). A reply can list an artifact the
 * store does not have — its live event went to a socket that has since
 * reconnected, and the reconnect re-read the messages but not the artifacts —
 * so whenever the thread's replies list an id the store lacks, the artifacts
 * are loaded again. The rows are the record; the live event only makes the
 * card appear sooner.
 */
export function useThreadArtifactsHydration(threadId: string | null | undefined): void {
  const dispatch = useAppDispatch();
  const normalizedThreadId = threadId?.trim() ?? '';
  const missingKey = useAppSelector(state => {
    if (!normalizedThreadId) return '';
    const known = new Set(
      (state.chatRuntime.artifactsByThread[normalizedThreadId] ?? []).map(a => a.artifactId)
    );
    const missing = new Set<string>();
    for (const message of state.thread.messagesByThreadId[normalizedThreadId] ?? []) {
      const ids = message.extraMetadata?.[ARTIFACT_IDS_METADATA_KEY];
      if (!Array.isArray(ids)) continue;
      for (const id of ids) if (typeof id === 'string' && !known.has(id)) missing.add(id);
    }
    return [...missing].sort().join('\n');
  });

  useEffect(() => {
    if (!normalizedThreadId) return;
    let cancelled = false;
    void listArtifactsForThread(normalizedThreadId).then(outcome => {
      if (cancelled) return;
      if (!outcome.ok) {
        console.warn('[artifact] failed to hydrate thread artifacts', {
          threadId: normalizedThreadId,
          error: outcome.error,
        });
        return;
      }
      if (outcome.artifacts.length === 0) return;
      console.debug('[artifact] hydrating thread artifacts', {
        threadId: normalizedThreadId,
        count: outcome.artifacts.length,
        missing: missingKey ? missingKey.split('\n').length : 0,
      });
      for (const artifact of outcome.artifacts) {
        dispatch(
          upsertArtifactReadyForThread({
            threadId: normalizedThreadId,
            artifactId: artifact.artifactId,
            kind: artifact.kind,
            title: artifact.title,
            path: artifact.path,
            sizeBytes: artifact.sizeBytes,
            loaded: true,
          })
        );
      }
    });
    return () => {
      cancelled = true;
    };
    // `missingKey` re-runs the load when a reply lists an artifact the store
    // lacks; it settles to '' once loaded, so this does not loop.
  }, [dispatch, normalizedThreadId, missingKey]);
}
