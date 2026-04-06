/**
 * DictationHotkeyManager
 *
 * Headless component that auto-registers the global dictation hotkey on mount
 * and logs toggle events. Mount inside ServiceBlockingGate so the core RPC
 * is available when it initialises.
 */
import { useEffect } from 'react';

import { useDictationHotkey } from '../hooks/useDictationHotkey';

export default function DictationHotkeyManager() {
  const { dictationEnabled, hotkeyRegistered, toggleCount, hotkey } = useDictationHotkey();

  useEffect(() => {
    if (toggleCount === 0) return;
    console.debug(`[dictation] toggle #${toggleCount} — dictation overlay should show/hide`);
  }, [toggleCount]);

  useEffect(() => {
    if (hotkeyRegistered) {
      console.debug(`[dictation] global hotkey active: ${hotkey}`);
    }
  }, [hotkeyRegistered, hotkey]);

  useEffect(() => {
    console.debug(`[dictation] enabled=${dictationEnabled}`);
  }, [dictationEnabled]);

  return null;
}
