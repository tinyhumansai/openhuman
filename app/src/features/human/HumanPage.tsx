import { useMemo, useRef, useState } from 'react';
import { LuX } from 'react-icons/lu';
import { useNavigate } from 'react-router-dom';

import Button from '../../components/ui/Button';
import { useT } from '../../lib/i18n/I18nContext';
import { useAppSelector } from '../../store/hooks';
import {
  selectCustomMascotGifUrl,
  selectCustomPrimaryColor,
  selectCustomSecondaryColor,
  selectMascotColor,
  selectSpeakReplies,
} from '../../store/mascotSlice';
import { HUMAN_VOICE_REALTIME_ENABLED, HUMAN_VOICE_SHOW_BOTH } from '../../utils/config';
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
import { IDLE_REALTIME_VOICE_AUDIO, type RealtimeVoiceAudio } from './voice/amplitudeLipsync';
import { useAmplitudeLipsync } from './voice/useAmplitudeLipsync';
import { resolveHumanVoiceEntry } from './voiceEntry';

const HumanPage = () => {
  const { t } = useT();
  const navigate = useNavigate();
  // Reads the shared preference rather than the old
  // `localStorage['human.speakReplies']` this page used to own. That key is
  // consumed and deleted by the mascot slice's persist migration, so keeping the
  // local copy would leave this page and the chat mascot disagreeing about the
  // same setting — and would silently drop whatever the user had chosen before.
  const speakReplies = useAppSelector(selectSpeakReplies);

  const { face, visemeCode } = useHumanMascot({ speakReplies });

  // Lip-sync for the realtime voice session. The session lives inside
  // RealtimeVoiceControls (which owns its own ConversationProvider), so it
  // publishes its output-loudness accessor into this ref and the mascot samples
  // it per frame — a 60fps signal must not travel through React state.
  const realtimeAudioRef = useRef<RealtimeVoiceAudio>({ ...IDLE_REALTIME_VOICE_AUDIO });
  // The agent's speaking edge, lifted out of RealtimeVoiceControls so it can gate
  // the lip-sync loop below. Flips a couple of times per turn, so it is cheap as
  // state (the 60fps amplitude stays in the ref). While it is false — an idle
  // realtime session, or the classic voice path that never mounts the control —
  // the loop schedules no frames at all.
  const [realtimeSpeaking, setRealtimeSpeaking] = useState(false);
  const realtimeLipsync = useAmplitudeLipsync(realtimeAudioRef, realtimeSpeaking);

  // While the agent is speaking its own audio drives the mouth; otherwise the
  // classic path keeps ownership, so the two never fight over the same frame.
  const mascotFace = realtimeLipsync.active ? 'speaking' : face;
  const mascotVisemeCode = realtimeLipsync.active ? realtimeLipsync.visemeCode : visemeCode;
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

  // Which voice control the tab offers. Build-flag driven (#5399). With the
  // chat rail removed there is nowhere left for the two paths to sit side by
  // side, so `realtime` and `both` now render the same single icon toggle and
  // only `push-to-talk` differs (see the render comment below).
  const voiceEntry = resolveHumanVoiceEntry({
    realtimeEnabled: HUMAN_VOICE_REALTIME_ENABLED,
    showBoth: HUMAN_VOICE_SHOW_BOTH,
  });

  return (
    <div className="absolute inset-0 overflow-hidden bg-surface-subtle dark:bg-surface-canvas">
      <div
        className="pointer-events-none absolute inset-0"
        style={{
          background: 'radial-gradient(ellipse at 50% 40%, rgba(74,131,221,0.10), transparent 60%)',
        }}
      />

      {/* Close — back to the chat surface this page was opened from. Mirrors
          the composer's idle button, which is how you get here. */}
      <Button
        iconOnly
        variant="tertiary"
        size="sm"
        analyticsId="human-close"
        data-testid="human-close"
        aria-label={t('common.close')}
        title={t('common.close')}
        className="absolute right-4 top-4 z-20 size-9 rounded-full text-content-muted hover:text-content-secondary"
        onClick={() => navigate('/chat')}>
        <LuX className="size-5" />
      </Button>

      {/* Mascot stage — the whole page. The chat rail that used to reserve
          436px on the right is gone: this surface is the mascot and the voice
          session, and the transcript lives on /chat. */}
      <div className="absolute inset-0 flex items-center justify-center">
        <div className="relative aspect-square w-[min(70vh,80%)]">
          {customMascotGifUrl ? (
            <CustomGifMascot src={customMascotGifUrl} face={mascotFace} />
          ) : mascotEntry ? (
            <ManifestRiveMascot
              key={mascotEntry.id}
              entry={mascotEntry}
              face={mascotFace}
              primaryColor={primaryColor}
              secondaryColor={secondaryColor}
              visemeCode={mascotVisemeCode}
              idlePoseRotation
            />
          ) : (
            <RiveMascot
              face={mascotFace}
              primaryColor={primaryColor}
              secondaryColor={secondaryColor}
              visemeCode={mascotVisemeCode}
              idlePoseRotation
            />
          )}
        </div>
      </div>

      {/* The page's single control: start/stop the voice session. Centered at
          the bottom, below the mascot rather than beside it.

          Only the realtime path has a start/stop session to bind to. The
          classic push-to-talk mic was part of the chat rail's composer and
          submitted its transcript through that surface's send path, so a
          `VITE_HUMAN_VOICE_REALTIME=false` build has no control to show here
          and falls back to a mascot-only stage. */}
      {voiceEntry !== 'push-to-talk' && (
        <div className="absolute inset-x-0 bottom-10 z-10 flex justify-center">
          <RealtimeVoiceControls
            appearance="icon"
            audioRef={realtimeAudioRef}
            onSpeakingChange={setRealtimeSpeaking}
          />
        </div>
      )}
    </div>
  );
};

export default HumanPage;
