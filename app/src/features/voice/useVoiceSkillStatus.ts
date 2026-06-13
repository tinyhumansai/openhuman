/**
 * Derives a skill-card-friendly status for Voice Intelligence,
 * matching the state vocabulary used by third-party skills (Gmail, etc.).
 *
 * Voice has a dependency on Local AI models (STT must be downloaded),
 * so the status reflects that prerequisite.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';

import { useCoreState } from '../../providers/CoreStateProvider';
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
  /** True when STT model is not yet downloaded. */
  sttModelMissing: boolean;
  /** Voice system availability info (null before first fetch). */
  voiceStatus: VoiceStatus | null;
  /** Voice server runtime state (null before first fetch). */
  serverStatus: VoiceServerStatus | null;
}

export function useVoiceSkillStatus(): VoiceSkillStatus {
  const { snapshot } = useCoreState();
  const localAi = snapshot.runtime.localAi;

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
    if (!voiceStatus.stt_available) return false;
    // The in-memory stt_state starts as "idle" and only flips to "ready"
    // after the first download or transcription.  The authoritative check
    // is `voiceStatus.stt_available` (which inspects the filesystem and
    // engine readiness).  Only block when stt_state is explicitly an error
    // state — "missing" means the model file really isn't on disk.
    if (localAi && localAi.stt_state === 'missing') return false;
    return true;
  }, [voiceStatus, localAi]);

  return useMemo(() => {
    // No data yet
    if (!voiceStatus || !serverStatus) {
      return { ...offlineStatus(), sttModelMissing: false, voiceStatus, serverStatus };
    }

    // STT model not downloaded — needs setup
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
