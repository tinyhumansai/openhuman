import type { DictationAdapter } from '@assistant-ui/react';
import { useCallback, useContext, useLayoutEffect, useMemo, useRef, useState } from 'react';

import {
  callCoreRpc,
  CoreRpcError,
  subscribeCoreRpcTokenInvalidated,
} from '../services/coreRpcClient';
import type { VoiceStatus } from '../utils/tauriCommands/voice';
import { CoreStateContext } from './coreStateContext';
import {
  createOpenHumanDictationAdapter,
  type DictationErrorCode,
  type DictationSnapshot,
  isDictationCaptureSupported,
  type OpenHumanDictationAdapter,
} from './dictationAdapter';

export type ComposerDictationStatus = DictationSnapshot['phase'] | 'checking' | 'unavailable';
export type ComposerDictationError = DictationErrorCode | 'stt-unavailable' | 'voice-status-failed';

interface ScopedDictationState {
  scope: object;
  adapter?: OpenHumanDictationAdapter;
  status: ComposerDictationStatus;
  error: ComposerDictationError | null;
}

const PROBE_RETRY_DELAYS_MS = [1_000, 2_000, 4_000];

/**
 * Offer dictation only after the active core confirms a usable STT provider.
 * `voice_status` is a config-only read: it does not record or contact a provider,
 * and its absence also identifies cores whose `voice` gate is disabled.
 *
 * Neither capability nor capture is shared across thread/profile/core changes.
 * The existing bearer invalidation subscription covers connection preference
 * changes and an embedded-core restart, even when its URL stays the same.
 */
export function useComposerDictation(threadId: string | null): {
  adapter: DictationAdapter | undefined;
  status: ComposerDictationStatus;
  error: ComposerDictationError | null;
  cancel: () => void;
} {
  const core = useContext(CoreStateContext);
  const [coreRevision, setCoreRevision] = useState(0);
  const captureSupported = isDictationCaptureSupported();
  const userId = core?.snapshot.auth.userId;
  const profileId = core?.snapshot.auth.profileId;
  const sessionToken = core?.snapshot.sessionToken;
  const scope = useMemo(
    () => ({ threadId, userId, profileId, sessionToken, coreRevision, captureSupported }),
    [threadId, userId, profileId, sessionToken, coreRevision, captureSupported]
  );
  const [state, setState] = useState<ScopedDictationState | null>(null);
  const activeAdapter = useRef<OpenHumanDictationAdapter | null>(null);

  useLayoutEffect(
    () =>
      subscribeCoreRpcTokenInvalidated(() => {
        // Stop a hot mic before waiting for React to apply the new scope.
        activeAdapter.current?.cancel();
        setCoreRevision(previous => previous + 1);
      }),
    []
  );

  useLayoutEffect(() => {
    if (!threadId || !captureSupported) {
      setState({ scope, status: 'unavailable', error: null });
      return;
    }

    let disposed = false;
    let revoked = false;
    let adapter: OpenHumanDictationAdapter | undefined;
    let unsubscribe: (() => void) | undefined;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;
    let retryCount = 0;
    let probing = false;
    setState({ scope, status: 'checking', error: null });

    /** Coalesce availability reads and retry transient failures within this scope. */
    const probe = () => {
      if (disposed || probing || adapter || revoked) return;
      probing = true;
      clearTimeout(retryTimer);
      void callCoreRpc<VoiceStatus>({
        method: 'openhuman.voice_status',
        params: {},
        // Availability is a narrow read, not evidence that the login expired.
        suppressAuthExpiredEvent: true,
      })
        .then(voice => {
          if (disposed) return;
          if (voice?.stt_available !== true) {
            setState({ scope, status: 'unavailable', error: 'stt-unavailable' });
            return;
          }
          adapter = createOpenHumanDictationAdapter();
          activeAdapter.current = adapter;
          /** Project adapter state, withdrawing it if dispatch is unsupported. */
          const publish = () => {
            if (disposed || revoked || !adapter) return;
            const snapshot = adapter.getSnapshot();
            if (snapshot.error === 'voice-unavailable') {
              // The STT call can discover a missing dispatch method after the
              // status probe succeeded (for example with a mismatched core).
              revoked = true;
              adapter.cancel();
              unsubscribe?.();
              adapter.dispose();
              if (activeAdapter.current === adapter) activeAdapter.current = null;
              setState({ scope, status: 'unavailable', error: 'voice-unavailable' });
              return;
            }
            setState({ scope, adapter, status: snapshot.phase, error: snapshot.error });
          };
          unsubscribe = adapter.subscribe(publish);
          publish();
        })
        .catch(error => {
          if (disposed) return;
          const missingVoice = error instanceof CoreRpcError && error.kind === 'method_not_found';
          console.debug('[composer-dictation] voice availability probe failed', {
            kind: missingVoice ? 'method_not_found' : 'unavailable',
          });
          setState({
            scope,
            status: 'unavailable',
            error: missingVoice ? 'stt-unavailable' : 'voice-status-failed',
          });
          if (!missingVoice && retryCount < PROBE_RETRY_DELAYS_MS.length) {
            retryTimer = setTimeout(probe, PROBE_RETRY_DELAYS_MS[retryCount++]);
          }
        })
        .finally(() => {
          probing = false;
        });
    };

    /** Recheck on focus after a failed probe or speech setup in another window. */
    const onFocus = () => {
      if (probing || adapter || revoked) return;
      retryCount = 0;
      probe();
    };
    window.addEventListener('focus', onFocus);
    probe();

    return () => {
      disposed = true;
      clearTimeout(retryTimer);
      window.removeEventListener('focus', onFocus);
      unsubscribe?.();
      adapter?.dispose();
      if (activeAdapter.current === adapter) activeAdapter.current = null;
    };
  }, [scope, threadId, captureSupported]);

  const cancel = useCallback(() => activeAdapter.current?.cancel(), []);
  // A render with a different scope must withdraw the old adapter immediately,
  // before effects run or a slow probe returns for the new connection/thread.
  const current = state?.scope === scope ? state : null;
  const adapter = current?.adapter;
  const status = current?.status ?? (threadId && captureSupported ? 'checking' : 'unavailable');
  const error = current?.error ?? null;
  return useMemo(() => ({ adapter, status, error, cancel }), [adapter, status, error, cancel]);
}
