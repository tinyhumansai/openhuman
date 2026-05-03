import { useEffect, useState } from 'react';

import Conversations from '../../pages/Conversations';
import { Ghosty } from './Mascot';
import { useHumanMascot } from './useHumanMascot';

const SPEAK_REPLIES_KEY = 'human.speakReplies';

const HumanPage = () => {
  const [speakReplies, setSpeakReplies] = useState<boolean>(() => {
    const raw = window.localStorage.getItem(SPEAK_REPLIES_KEY);
    return raw === null ? true : raw === '1';
  });

  useEffect(() => {
    window.localStorage.setItem(SPEAK_REPLIES_KEY, speakReplies ? '1' : '0');
  }, [speakReplies]);

  const { face, viseme } = useHumanMascot({ speakReplies });

  return (
    <div className="absolute inset-0 bg-stone-100 overflow-hidden">
      {/* Mascot stage — full bleed under the floating sidebar. */}
      <div className="absolute inset-0 flex items-center justify-center">
        <div
          className="pointer-events-none absolute inset-0"
          style={{
            background:
              'radial-gradient(ellipse at 50% 40%, rgba(74,131,221,0.10), transparent 60%)',
          }}
        />
        <div className="relative w-[min(80vh,80vw)] aspect-square">
          <Ghosty face={face} viseme={viseme} />
        </div>
      </div>

      <label className="absolute top-4 left-4 z-10 inline-flex items-center gap-2 px-3 py-1.5 rounded-full bg-white/80 backdrop-blur-sm border border-stone-300 text-xs text-stone-700 shadow-soft cursor-pointer select-none">
        <input
          type="checkbox"
          checked={speakReplies}
          onChange={e => setSpeakReplies(e.target.checked)}
          className="cursor-pointer"
        />
        Speak replies
      </label>

      {/* Floating chat sidebar — leaves room for BottomTabBar (~80px) at the foot. */}
      <aside
        className="absolute top-4 right-4 bottom-24 w-[420px] z-10 rounded-2xl border border-stone-300 bg-white shadow-soft flex flex-col overflow-hidden">
        <Conversations variant="sidebar" />
      </aside>
    </div>
  );
};

export default HumanPage;
