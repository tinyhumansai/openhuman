import { useEffect, useMemo, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { useAppSelector } from '../../store/hooks';
import {
  selectCustomMascotGifUrl,
  selectCustomPrimaryColor,
  selectCustomSecondaryColor,
  selectMascotColor,
  selectVoiceMode,
} from '../../store/mascotSlice';
import { VOICE_MODE_FLAG_ENABLED } from '../../utils/config';
import Conversations from '../conversations/Conversations';
import {
  CustomGifMascot,
  getMascotPalette,
  hexToArgbInt,
  ManifestRiveMascot,
  RiveMascot,
} from './Mascot';
import { useMascotManifest } from './Mascot/manifest/useMascotManifest';
import RealtimeVoiceControls from './RealtimeVoiceControls';
import { useHumanMascot } from './useHumanMascot';

const SPEAK_REPLIES_KEY = 'human.speakReplies';

const HumanPage = () => {
  const { t } = useT();
  const [speakReplies, setSpeakReplies] = useState<boolean>(() => {
    const raw = window.localStorage.getItem(SPEAK_REPLIES_KEY);
    return raw === null ? true : raw === '1';
  });

  useEffect(() => {
    window.localStorage.setItem(SPEAK_REPLIES_KEY, speakReplies ? '1' : '0');
  }, [speakReplies]);

  const { face, visemeCode } = useHumanMascot({ speakReplies });
  const voiceMode = useAppSelector(selectVoiceMode);
  const realtimeEnabled = VOICE_MODE_FLAG_ENABLED && voiceMode === 'realtime';
  const mascotColor = useAppSelector(selectMascotColor);
  const customPrimary = useAppSelector(selectCustomPrimaryColor);
  const customSecondary = useAppSelector(selectCustomSecondaryColor);
  const customMascotGifUrl = useAppSelector(selectCustomMascotGifUrl);
  // Active mascot resolved from the GitHub manifest (selection + default).
  const { entry: mascotEntry } = useMascotManifest();
  const palette = getMascotPalette(mascotColor);
  const primaryColor = useMemo(
    () => hexToArgbInt(mascotColor === 'custom' ? customPrimary : palette.bodyFill),
    [mascotColor, customPrimary, palette]
  );
  const secondaryColor = useMemo(
    () => hexToArgbInt(mascotColor === 'custom' ? customSecondary : palette.neckShadowColor),
    [mascotColor, customSecondary, palette]
  );

  // The mascot drives a ~60fps lipsync re-render while the agent is speaking
  // (useHumanMascot forces a frame each rAF tick). Conversations is a heavy
  // subtree, so co-rendering it here would reconcile the whole chat tree every
  // frame and starve the main thread — which is what made tab switching feel
  // locked during TTS playback (#5357). Its props are constant, so hold a stable
  // element: React short-circuits reconciliation of an unchanged child, keeping
  // the per-frame mascot re-render off the chat tree and the UI responsive.
  const chatPanel = useMemo(
    () => <Conversations variant="sidebar" composer="mic-cloud" projectThreadList />,
    []
  );

  return (
    <div className="absolute inset-0 bg-surface-subtle dark:bg-surface-canvas overflow-hidden">
      <div
        className="pointer-events-none absolute inset-0"
        style={{
          background: 'radial-gradient(ellipse at 35% 40%, rgba(74,131,221,0.10), transparent 60%)',
        }}
      />

      {/* Mascot stage — fills the area to the left of the reserved chat column. */}
      <div className="absolute inset-y-0 left-0 right-[436px] flex items-center justify-center">
        <div className="relative w-[min(80vh,90%)] aspect-square">
          {customMascotGifUrl ? (
            <CustomGifMascot src={customMascotGifUrl} face={face} />
          ) : mascotEntry ? (
            <ManifestRiveMascot
              key={mascotEntry.id}
              entry={mascotEntry}
              face={face}
              primaryColor={primaryColor}
              secondaryColor={secondaryColor}
              visemeCode={visemeCode}
              idlePoseRotation
            />
          ) : (
            <RiveMascot
              face={face}
              primaryColor={primaryColor}
              secondaryColor={secondaryColor}
              visemeCode={visemeCode}
              idlePoseRotation
            />
          )}
        </div>
      </div>

      {/* Realtime voice-chat controls (#5399) — additive overlay shown only when
          the flag + realtime mode are on; the classic push-to-talk path below
          is untouched. */}
      {realtimeEnabled && (
        <div className="absolute bottom-8 left-0 right-[436px] z-10 flex justify-center">
          <RealtimeVoiceControls />
        </div>
      )}

      <label className="absolute top-4 left-4 z-10 inline-flex items-center gap-2 px-3 py-1.5 rounded-full bg-surface/80 backdrop-blur-sm border border-line-strong text-xs text-content-secondary shadow-soft cursor-pointer select-none">
        <input
          type="checkbox"
          checked={speakReplies}
          onChange={e => setSpeakReplies(e.target.checked)}
          className="cursor-pointer"
        />
        {t('voice.pushToTalk')}
      </label>

      {/* Chat panel — kept on the right (the Human page is intentionally the
          one surface that leaves the root sidebar's dynamic region empty). */}
      <div className="absolute right-4 top-4 bottom-4 z-10 flex items-center">
        <aside className="w-[420px] h-[min(760px,100%)] rounded-2xl border border-line-strong bg-surface shadow-soft flex flex-col overflow-hidden">
          {/* Right-rail chat, but its thread list is surfaced in the (otherwise
              empty) root sidebar so the Human page shows the user's threads.
              Held as a stable element (chatPanel) so mascot lipsync re-renders
              don't reconcile it — see #5357. */}
          {chatPanel}
        </aside>
      </div>
    </div>
  );
};

export default HumanPage;
