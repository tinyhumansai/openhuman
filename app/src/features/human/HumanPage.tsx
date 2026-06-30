import { useEffect, useMemo, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import Conversations from '../../pages/Conversations';
import { useAppSelector } from '../../store/hooks';
import {
  selectCustomMascotGifUrl,
  selectCustomPrimaryColor,
  selectCustomSecondaryColor,
  selectMascotColor,
} from '../../store/mascotSlice';
import {
  CustomGifMascot,
  getMascotPalette,
  hexToArgbInt,
  ManifestRiveMascot,
  RiveMascot,
} from './Mascot';
import { useMascotManifest } from './Mascot/manifest/useMascotManifest';
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
              empty) root sidebar so the Human page shows the user's threads. */}
          <Conversations variant="sidebar" composer="mic-cloud" projectThreadList />
        </aside>
      </div>
    </div>
  );
};

export default HumanPage;
