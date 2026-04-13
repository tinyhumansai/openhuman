/**
 * Derives a skill-card-friendly status for Voice Intelligence,
 * matching the state vocabulary used by third-party skills (Gmail, etc.).
 *
 * Voice has a dependency on Local AI models (STT must be downloaded),
 * so the status reflects that prerequisite.
 */
import { useCallback, useEffect, useMemo, useState } from 'react';

import { useCoreState } from '../../providers/CoreStateProvider';
import type { SkillConnectionStatus } from '../../types/skillStatus';
import { isTauri } from '../../utils/tauriCommands/common';
import {
  openhumanVoiceServerStatus,
  openhumanVoiceStatus,
  type VoiceServerStatus,
  type VoiceStatus,
} from '../../utils/tauriCommands/voice';

export interface VoiceSkillStatus {
  connectionStatus: SkillConnectionStatus;
  statusDot: string;
  statusLabel: string;
  statusColor: string;
  ctaLabel: string;
  ctaVariant: 'primary' | 'sage' | 'amber';
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
      return {
        connectionStatus: 'offline' as SkillConnectionStatus,
        statusDot: 'bg-stone-400',
        statusLabel: 'Offline',
        statusColor: 'text-stone-500',
        ctaLabel: 'Enable',
        ctaVariant: 'sage' as const,
        sttModelMissing: false,
        voiceStatus,
        serverStatus,
      };
    }

    // STT model not downloaded — needs setup
    if (!sttReady) {
      return {
        connectionStatus: 'setup_required' as SkillConnectionStatus,
        statusDot: 'bg-primary-400',
        statusLabel: 'Setup',
        statusColor: 'text-primary-400',
        ctaLabel: 'Setup',
        ctaVariant: 'primary' as const,
        sttModelMissing: true,
        voiceStatus,
        serverStatus,
      };
    }

    // Error
    if (serverStatus.last_error) {
      return {
        connectionStatus: 'error' as SkillConnectionStatus,
        statusDot: 'bg-coral-500',
        statusLabel: 'Error',
        statusColor: 'text-coral-400',
        ctaLabel: 'Retry',
        ctaVariant: 'amber' as const,
        sttModelMissing: false,
        voiceStatus,
        serverStatus,
      };
    }

    // Active states: recording, transcribing, or idle (server running)
    if (serverStatus.state === 'recording' || serverStatus.state === 'transcribing') {
      return {
        connectionStatus: 'connecting' as SkillConnectionStatus,
        statusDot: 'bg-amber-500 animate-pulse',
        statusLabel: serverStatus.state === 'recording' ? 'Recording' : 'Transcribing',
        statusColor: 'text-amber-400',
        ctaLabel: 'Manage',
        ctaVariant: 'primary' as const,
        sttModelMissing: false,
        voiceStatus,
        serverStatus,
      };
    }

    if (serverStatus.state === 'idle') {
      return {
        connectionStatus: 'connected' as SkillConnectionStatus,
        statusDot: 'bg-sage-500',
        statusLabel: 'Active',
        statusColor: 'text-sage-400',
        ctaLabel: 'Manage',
        ctaVariant: 'primary' as const,
        sttModelMissing: false,
        voiceStatus,
        serverStatus,
      };
    }

    // Stopped
    return {
      connectionStatus: 'offline' as SkillConnectionStatus,
      statusDot: 'bg-stone-400',
      statusLabel: 'Stopped',
      statusColor: 'text-stone-500',
      ctaLabel: 'Enable',
      ctaVariant: 'sage' as const,
      sttModelMissing: false,
      voiceStatus,
      serverStatus,
    };
  }, [voiceStatus, serverStatus, sttReady]);
}
