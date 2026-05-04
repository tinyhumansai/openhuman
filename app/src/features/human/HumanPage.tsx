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

  // Sidebar reserves ~436px (420px panel + 16px gutter) on the right; the
  // mascot stage takes the remaining width so the two never overlap.
  return (
    <div className="absolute inset-0 bg-stone-100 overflow-hidden">
      <div
        className="pointer-events-none absolute inset-0"
        style={{
          background: 'radial-gradient(ellipse at 35% 40%, rgba(74,131,221,0.10), transparent 60%)',
        }}
      />

      {/* Mascot stage — fills the area to the left of the reserved sidebar column. */}
      <div className="absolute inset-y-0 left-0 right-[436px] flex items-center justify-center">
        <div className="relative w-[min(80vh,90%)] aspect-square">
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

      {/* Chat sidebar — vertically centered above the BottomTabBar (~80px). */}
      <div className="absolute right-4 top-0 bottom-20 z-10 flex items-center">
        <aside className="w-[420px] h-[min(720px,calc(100vh-160px))] rounded-2xl border border-stone-300 bg-white shadow-soft flex flex-col overflow-hidden">
          <Conversations variant="sidebar" />
        </aside>
      </div>
    </div>
  );
};

export default HumanPage;
