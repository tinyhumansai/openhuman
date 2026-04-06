/**
 * useDictationHotkey
 *
 * On mount, fetches the dictation config from the core RPC and auto-registers
 * the global hotkey if dictation is enabled. Listens for `dictation://toggle`
 * events emitted by the Tauri shell when the hotkey is pressed.
 *
 * Consumers receive:
 *   - `dictationEnabled`: whether dictation is configured on
 *   - `hotkeyRegistered`: whether the global shortcut is active
 *   - `toggleCount`: increments each time the hotkey fires (use to trigger effects)
 */
import { listen } from '@tauri-apps/api/event';
import { useEffect, useRef, useState } from 'react';

import { callCoreRpc } from '../services/coreRpcClient';
import {
  isTauri,
  registerDictationHotkey,
  unregisterDictationHotkey,
} from '../utils/tauriCommands';

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
  /** Whether the global shortcut was successfully registered. */
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
  const registeredRef = useRef(false);

  useEffect(() => {
    if (!isTauri()) return;

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

        if (!s.enabled || !s.hotkey) {
          console.debug('[dictation] dictation disabled or no hotkey configured');
          return;
        }

        console.debug(`[dictation] auto-registering hotkey: ${s.hotkey}`);
        await registerDictationHotkey(s.hotkey);
        if (!disposed) {
          registeredRef.current = true;
          setHotkeyRegistered(true);
          console.debug('[dictation] hotkey registered successfully');
        }
      } catch (err) {
        console.warn('[dictation] failed to init dictation hotkey', err);
      }
    };

    void init();

    return () => {
      disposed = true;
      if (registeredRef.current) {
        registeredRef.current = false;
        unregisterDictationHotkey().catch(err =>
          console.warn('[dictation] cleanup unregister failed', err)
        );
      }
    };
  }, []);

  // Listen for hotkey toggle events
  useEffect(() => {
    if (!isTauri()) return;

    let unlisten: (() => void) | undefined;

    listen('dictation://toggle', () => {
      console.debug('[dictation] hotkey toggle event received');
      setToggleCount(c => c + 1);
    })
      .then(fn => {
        unlisten = fn;
      })
      .catch(err => {
        console.warn('[dictation] failed to listen for dictation toggle', err);
      });

    return () => {
      unlisten?.();
    };
  }, []);

  return { dictationEnabled, hotkeyRegistered, toggleCount, activationMode, hotkey };
}
