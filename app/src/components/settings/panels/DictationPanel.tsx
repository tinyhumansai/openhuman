import { useEffect, useState } from 'react';

import {
  checkDictationAvailability,
  setHotkey,
  setShowFloatingLauncher,
} from '../../../store/dictationSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { isTauri, registerDictationHotkey } from '../../../utils/tauriCommands';
import SettingsBackButton from '../components/SettingsBackButton';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const DictationPanel = () => {
  const dispatch = useAppDispatch();
  const { navigateBack } = useSettingsNavigation();
  const { hotkey, showFloatingLauncher, voiceStatus, statusCheckError, isCheckingStatus } =
    useAppSelector(s => s.dictation);
  const [hotkeyInput, setHotkeyInput] = useState(hotkey);
  const [isSavingHotkey, setIsSavingHotkey] = useState(false);
  const [hotkeyError, setHotkeyError] = useState<string | null>(null);
  const [hotkeySuccess, setHotkeySuccess] = useState(false);

  useEffect(() => {
    console.debug('[dictation-panel] mounting — checking availability');
    void dispatch(checkDictationAvailability());
  }, [dispatch]);

  // Keep local input in sync if hotkey changes externally
  useEffect(() => {
    setHotkeyInput(hotkey);
  }, [hotkey]);

  const handleSaveHotkey = async () => {
    const trimmed = hotkeyInput.trim();
    if (!trimmed) {
      setHotkeyError('Hotkey cannot be empty');
      return;
    }

    setIsSavingHotkey(true);
    setHotkeyError(null);
    setHotkeySuccess(false);

    console.debug('[dictation-panel] saving hotkey: %s', trimmed);

    try {
      if (isTauri()) {
        await registerDictationHotkey(trimmed);
      }
      dispatch(setHotkey(trimmed));
      setHotkeySuccess(true);
      setTimeout(() => setHotkeySuccess(false), 2000);
    } catch (err) {
      const msg = err instanceof Error ? err.message : 'Failed to register hotkey';
      console.error('[dictation-panel] hotkey error:', msg);
      setHotkeyError(msg);
    } finally {
      setIsSavingHotkey(false);
    }
  };

  const statusLabel = () => {
    if (isCheckingStatus) return 'Checking...';
    if (statusCheckError) return `Check failed: ${statusCheckError}`;
    if (!voiceStatus) return 'Not checked';
    if (voiceStatus.stt_available) return 'Ready (model loaded)';
    if (voiceStatus.stt_model_path) return 'Model found — will load on first use';
    return 'Model not found';
  };

  const statusColor = () => {
    if (isCheckingStatus) return 'bg-stone-500 animate-pulse';
    if (statusCheckError) return 'bg-amber-400';
    if (!voiceStatus) return 'bg-stone-500';
    if (voiceStatus.stt_available || voiceStatus.stt_model_path) return 'bg-green-400';
    return 'bg-red-400';
  };

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <SettingsBackButton onClick={navigateBack} title="Settings" />

      <div className="flex-1 overflow-y-auto p-4 space-y-5 max-w-md mx-auto w-full">
        <div>
          <h2 className="text-lg font-semibold text-white mb-1">Voice Dictation</h2>
          <p className="text-sm text-stone-400">
            Transcribe speech to text using your microphone and local AI.
          </p>
        </div>

        {/* STT Engine Status */}
        <div className="bg-stone-800/50 rounded-xl border border-stone-700/40 p-4 space-y-3">
          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium text-white">Speech-to-Text Engine</p>
              <p className="text-xs text-stone-400 mt-0.5">{statusLabel()}</p>
            </div>
            <div className="flex items-center gap-2">
              <button
                onClick={() => void dispatch(checkDictationAvailability())}
                disabled={isCheckingStatus}
                className="text-xs text-stone-400 hover:text-white transition-colors disabled:opacity-40 px-2 py-1 rounded border border-stone-700 hover:border-stone-500">
                {isCheckingStatus ? '...' : 'Refresh'}
              </button>
              <div className={`w-2.5 h-2.5 rounded-full ${statusColor()}`} />
            </div>
          </div>

          {/* Detailed status rows */}
          {voiceStatus && (
            <div className="space-y-1.5 pt-1 border-t border-stone-700/40">
              <StatusRow
                label="Whisper binary"
                value={voiceStatus.whisper_binary ?? 'not found'}
                ok={!!voiceStatus.whisper_binary}
              />
              <StatusRow
                label="In-process mode"
                value={voiceStatus.whisper_in_process ? 'loaded' : 'not loaded'}
                ok={voiceStatus.whisper_in_process}
              />
              <StatusRow
                label="STT model"
                value={voiceStatus.stt_model_path ?? `not found (id: ${voiceStatus.stt_model_id})`}
                ok={!!voiceStatus.stt_model_path}
              />
              <StatusRow label="Model ID" value={voiceStatus.stt_model_id} ok={true} muted />
            </div>
          )}
        </div>

        {/* Model not found guidance */}
        {voiceStatus && !voiceStatus.stt_model_path && !isCheckingStatus && (
          <div className="bg-amber-500/10 border border-amber-500/20 rounded-xl p-4">
            <p className="text-xs text-amber-400 leading-relaxed">
              Model file <code className="text-amber-300">{voiceStatus.stt_model_id}</code> was not
              found. Go to <strong className="text-amber-300">Settings → Local AI Model</strong> to
              download it, or place the file at{' '}
              <code className="text-amber-300 break-all">
                ~/.openhuman/models/local-ai/stt/{voiceStatus.stt_model_id}
              </code>
            </p>
          </div>
        )}

        {/* Global Hotkey */}
        <div className="bg-stone-800/50 rounded-xl border border-stone-700/40 p-4 space-y-3">
          <div>
            <p className="text-sm font-medium text-white">Global Hotkey</p>
            <p className="text-xs text-stone-400 mt-0.5">
              Press anywhere to start / stop dictation
            </p>
          </div>
          <div className="flex gap-2">
            <input
              type="text"
              value={hotkeyInput}
              onChange={e => setHotkeyInput(e.target.value)}
              placeholder="e.g. CmdOrCtrl+Shift+D"
              className="flex-1 bg-stone-700/60 border border-stone-600/50 rounded-lg px-3 py-2 text-sm text-white placeholder-stone-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
            />
            <button
              onClick={() => void handleSaveHotkey()}
              disabled={isSavingHotkey || hotkeyInput.trim() === hotkey}
              className="px-4 py-2 bg-primary-600 hover:bg-primary-500 disabled:opacity-50 text-white text-sm font-medium rounded-lg transition-colors">
              {isSavingHotkey ? 'Saving...' : hotkeySuccess ? 'Saved!' : 'Save'}
            </button>
          </div>
          {hotkeyError && <p className="text-xs text-red-400">{hotkeyError}</p>}
          <p className="text-xs text-stone-500">
            Modifiers: <code>CmdOrCtrl</code>, <code>Alt</code>, <code>Shift</code>,{' '}
            <code>Super</code> (also accepts CommandOrControl)
          </p>
        </div>

        {/* Floating launcher preference */}
        <div className="bg-stone-800/50 rounded-xl border border-stone-700/40 p-4 space-y-3">
          <div className="flex items-center justify-between gap-3">
            <div>
              <p className="text-sm font-medium text-white">Always show floating Start button</p>
              <p className="text-xs text-stone-400 mt-0.5">
                If disabled, dictation starts via hotkey only while idle.
              </p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={showFloatingLauncher}
              onClick={() => dispatch(setShowFloatingLauncher(!showFloatingLauncher))}
              className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${
                showFloatingLauncher ? 'bg-primary-600' : 'bg-stone-600'
              }`}>
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                  showFloatingLauncher ? 'translate-x-6' : 'translate-x-1'
                }`}
              />
            </button>
          </div>
        </div>

        {/* How to use */}
        <div className="bg-stone-800/50 rounded-xl border border-stone-700/40 p-4 space-y-2">
          <p className="text-sm font-medium text-white">How to use</p>
          <ol className="text-xs text-stone-400 space-y-1.5 list-decimal list-inside">
            <li>Press the global hotkey (or click Record in the overlay)</li>
            <li>Speak clearly into your microphone</li>
            <li>Press the hotkey again (or click Stop) to finish</li>
            <li>Wait a moment for transcription</li>
            <li>Click Insert to type into the focused field, or Copy to clipboard</li>
          </ol>
        </div>
      </div>
    </div>
  );
};

interface StatusRowProps {
  label: string;
  value: string;
  ok: boolean;
  muted?: boolean;
}

const StatusRow = ({ label, value, ok, muted }: StatusRowProps) => (
  <div className="flex items-start gap-2 text-xs">
    <span
      className={`mt-0.5 w-1.5 h-1.5 rounded-full flex-shrink-0 ${
        muted ? 'bg-stone-600' : ok ? 'bg-green-400' : 'bg-red-400'
      }`}
    />
    <span className="text-stone-500 flex-shrink-0 w-28">{label}</span>
    <span className="text-stone-300 break-all leading-relaxed">{value}</span>
  </div>
);

export default DictationPanel;
