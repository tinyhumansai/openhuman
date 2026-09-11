/**
 * Derives a skill-card-friendly status for Voice Intelligence,
 * matching the state vocabulary used by third-party skills (Gmail, etc.).
 *
 * Speech-to-text is hosted, so the prerequisite is a *configured* engine
 * (`voice_server.stt_engine` resolving to a provider), not a downloaded model.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';

import { isTauri } from '../../utils/tauriCommands/common';
import {
  openhumanVoiceServerStatus,
  openhumanVoiceStatus,
  type VoiceServerStatus,
  type VoiceStatus,
} from '../../utils/tauriCommands/voice';
import {
  activeStatus,
  errorStatus,
  offlineStatus,
  setupRequiredStatus,
  type SkillCardStatusDescriptor,
  transientStatus,
} from '../skills/skillCardStatus';

export interface VoiceSkillStatus extends SkillCardStatusDescriptor {
  /** True when the configured STT engine does not resolve to a provider —
   *  i.e. the user picked a third-party engine with no credentials set up. */
  sttModelMissing: boolean;
  /** Voice system availability info (null before first fetch). */
  voiceStatus: VoiceStatus | null;
  /** Voice server runtime state (null before first fetch). */
  serverStatus: VoiceServerStatus | null;
}

export function useVoiceSkillStatus(): VoiceSkillStatus {
  const [voiceStatus, setVoiceStatus] = useState<VoiceStatus | null>(null);
  const [serverStatus, setServerStatus] = useState<VoiceServerStatus | null>(null);

  const fetchStatuses = useCallback(async () => {
    if (!isTauri()) return;
    try {
      const [vs, ss] = await Promise.all([openhumanVoiceStatus(), openhumanVoiceServerStatus()]);
      setVoiceStatus(vs);
      setServerStatus(ss);
    } catch (err) {
      console.debug('[voice-skill-status] status fetch failed, will retry on next poll:', err);
    }
  }, []);

  // Poll voice status every 3s (lighter than the panel's 2s — just for card state)
  useEffect(() => {
    void fetchStatuses();
    const id = window.setInterval(() => void fetchStatuses(), 3000);
    return () => window.clearInterval(id);
  }, [fetchStatuses]);

  const sttReady = useMemo(() => {
    if (!voiceStatus) return false;
    // `stt_available` is the authoritative check: it asks whether the
    // configured hosted engine resolves to a provider at all. Nothing has to be
    // installed for STT any more, so the local-AI asset state is no longer
    // consulted here — a workspace that never downloaded a model is still ready.
    return voiceStatus.stt_available;
  }, [voiceStatus]);

  return useMemo(() => {
    // No data yet
    if (!voiceStatus || !serverStatus) {
      return { ...offlineStatus(), sttModelMissing: false, voiceStatus, serverStatus };
    }

    // No usable STT engine — needs setup
    if (!sttReady) {
      return { ...setupRequiredStatus(), sttModelMissing: true, voiceStatus, serverStatus };
    }

    // Error
    if (serverStatus.last_error) {
      return { ...errorStatus(), sttModelMissing: false, voiceStatus, serverStatus };
    }

    // Active states: recording, transcribing, or idle (server running)
    if (serverStatus.state === 'recording' || serverStatus.state === 'transcribing') {
      return {
        ...transientStatus(serverStatus.state === 'recording' ? 'Recording' : 'Transcribing'),
        sttModelMissing: false,
        voiceStatus,
        serverStatus,
      };
    }

    if (serverStatus.state === 'idle') {
      return { ...activeStatus(), sttModelMissing: false, voiceStatus, serverStatus };
    }

    // Stopped
    return { ...offlineStatus('Stopped'), sttModelMissing: false, voiceStatus, serverStatus };
  }, [voiceStatus, serverStatus, sttReady]);
}
