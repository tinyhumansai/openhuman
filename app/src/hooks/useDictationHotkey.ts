/**
 * useDictationHotkey
 *
 * Fetches dictation config from the core RPC on mount and listens for
 * `dictation:toggle` Socket.IO events emitted by the Rust core when
 * the global hotkey is pressed. The hotkey listener runs in the core
 * process (via rdev), not in the Tauri shell.
 *
 * Dictation events are received over a **dedicated** Socket.IO
 * connection to the core process that does not require authentication.
 * This ensures dictation works regardless of whether the user is
 * logged in.
 *
 * Consumers receive:
 *   - `dictationEnabled`: whether dictation is configured on
 *   - `hotkeyRegistered`: true once the core confirms the hotkey is active
 *   - `toggleCount`: increments each time the hotkey fires (use to trigger effects)
 *   - `activationMode`: "toggle" or "push"
 *   - `hotkey`: the configured hotkey string
 */
import { invoke, isTauri } from '@tauri-apps/api/core';
import { useEffect, useRef, useState } from 'react';
import { io, Socket } from 'socket.io-client';

import { callCoreRpc } from '../services/coreRpcClient';
import { CORE_RPC_URL } from '../utils/config';

/** Resolve the core process base URL (without /rpc suffix) for Socket.IO. */
async function resolveCoreSocketUrl(): Promise<string> {
  let rpcUrl = CORE_RPC_URL;
  if (isTauri()) {
    try {
      const url = await invoke<string>('core_rpc_url');
      if (url) rpcUrl = String(url);
    } catch {
      // fall through to default
    }
  }
  const trimmed = rpcUrl.trim().replace(/\/+$/, '');
  return trimmed.endsWith('/rpc') ? trimmed.slice(0, -4) : trimmed;
}

interface DictationSettings {
  enabled: boolean;
  hotkey: string;
  activation_mode: string;
  llm_refinement: boolean;
  streaming: boolean;
  streaming_interval_ms: number;
}

export interface DictationHotkeyState {
  /** Whether dictation is enabled in the core config. */
  dictationEnabled: boolean;
  /** Whether the core hotkey listener is active. */
  hotkeyRegistered: boolean;
  /** Increments each time the hotkey is pressed (consumers can use as a trigger). */
  toggleCount: number;
  /** The configured activation mode ("toggle" or "push"). */
  activationMode: string;
  /** The configured hotkey string. */
  hotkey: string;
}

export function useDictationHotkey(): DictationHotkeyState {
  const [dictationEnabled, setDictationEnabled] = useState(false);
  const [hotkeyRegistered, setHotkeyRegistered] = useState(false);
  const [toggleCount, setToggleCount] = useState(0);
  const [activationMode, setActivationMode] = useState('toggle');
  const [hotkey, setHotkey] = useState('');
  const socketRef = useRef<Socket | null>(null);

  // Fetch config from core RPC on mount.
  useEffect(() => {
    let disposed = false;

    const init = async () => {
      try {
        const settings = await callCoreRpc<DictationSettings>({
          method: 'openhuman.config_get_dictation_settings',
        });

        if (disposed) return;

        if (!settings || typeof settings !== 'object') {
          console.debug('[dictation] no dictation settings from core');
          return;
        }

        // Handle RpcOutcome wrapper — the result may be nested in .result
        const s = (
          'result' in settings ? (settings as Record<string, unknown>).result : settings
        ) as DictationSettings;

        setDictationEnabled(s.enabled);
        setActivationMode(s.activation_mode ?? 'toggle');
        setHotkey(s.hotkey ?? '');

        if (s.enabled && s.hotkey) {
          // The core process registers the hotkey via rdev — we just note it.
          setHotkeyRegistered(true);
          console.debug(`[dictation] core hotkey active: ${s.hotkey}`);
        } else {
          console.debug('[dictation] dictation disabled or no hotkey configured');
        }
      } catch (err) {
        console.warn('[dictation] failed to fetch dictation settings', err);
      }
    };

    void init();

    return () => {
      disposed = true;
    };
  }, []);

  // Open a dedicated Socket.IO connection to the core for dictation
  // events. This is independent of the main socketService (which
  // requires auth) so dictation works even when not logged in.
  useEffect(() => {
    let socket: Socket | null = null;
    let disposed = false;

    const connect = async () => {
      try {
        const baseUrl = await resolveCoreSocketUrl();
        if (disposed) return;

        socket = io(baseUrl, {
          path: '/socket.io/',
          transports: ['websocket', 'polling'],
          reconnection: true,
          reconnectionDelay: 2000,
          reconnectionAttempts: Infinity,
          forceNew: true,
        });
        socketRef.current = socket;

        socket.on('connect', () => {
          console.debug('[dictation] dedicated socket connected', socket?.id);
        });

        socket.on('connect_error', (err: Error) => {
          console.debug('[dictation] socket connect error:', err.message);
        });

        // Hotkey toggle events.
        const handleToggle = () => {
          console.debug('[dictation] hotkey toggle event received');
          setToggleCount(c => c + 1);
        };
        socket.on('dictation:toggle', handleToggle);
        socket.on('dictation_toggle', handleToggle);

        // Transcription results — dispatch the custom DOM event that
        // Conversations.tsx uses to insert text into the chat input.
        socket.on('dictation:transcription', (data: { text?: string }) => {
          const text = data?.text?.trim();
          if (!text) return;
          console.debug(`[dictation] transcription received: ${text.length} chars — "${text}"`);

          window.dispatchEvent(new CustomEvent('dictation://insert-text', { detail: { text } }));
        });

        socket.connect();
      } catch (err) {
        console.warn('[dictation] failed to open dedicated socket', err);
      }
    };

    void connect();

    return () => {
      disposed = true;
      if (socket) {
        socket.disconnect();
        socket = null;
      }
      socketRef.current = null;
    };
  }, []);

  return { dictationEnabled, hotkeyRegistered, toggleCount, activationMode, hotkey };
}
