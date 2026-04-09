import { getCurrentWindow, LogicalSize } from '@tauri-apps/api/window';
import { useEffect, useMemo, useState } from 'react';

import RotatingTetrahedronCanvas from '../components/RotatingTetrahedronCanvas';

const OVERLAY_WIDTH = 248;
const OVERLAY_HEIGHT = 228;
const SCENARIO_TWO_TEXT = '"Noted. Need milk."';

type OverlayStatus = 'idle' | 'active';
type OverlayScenario = 1 | 2 | 3;

interface OverlayBubble {
  id: string;
  text: string;
  tone: 'neutral' | 'accent' | 'success';
  compact?: boolean;
}

function bubbleToneClass(tone: OverlayBubble['tone']) {
  switch (tone) {
    case 'accent':
      return 'bg-blue-700 text-white';
    case 'success':
      return 'bg-emerald-500 text-emerald-950';
    default:
      return 'bg-slate-700 text-white';
  }
}

function OverlayBubbleChip({ bubble }: { bubble: OverlayBubble }) {
  return (
    <div
      className={`max-w-[184px] rounded-[18px] px-3 py-2 text-right transition-all duration-200 ${bubbleToneClass(bubble.tone)} ${bubble.compact ? 'text-[12px] leading-[1.35]' : 'text-[13px] leading-[1.45]'}`}>
      {bubble.text}
    </div>
  );
}

export default function OverlayApp() {
  const appWindow = getCurrentWindow();
  const [scenario, setScenario] = useState<OverlayScenario>(1);
  const [typedText, setTypedText] = useState('');

  useEffect(() => {
    const size = new LogicalSize(OVERLAY_WIDTH, OVERLAY_HEIGHT);
    void appWindow.setSize(size).catch(error => {
      console.warn('[overlay] failed to resize overlay window', error);
    });
    void appWindow.setMinSize(size).catch(error => {
      console.warn('[overlay] failed to set overlay min size', error);
    });
    void appWindow.setMaxSize(size).catch(error => {
      console.warn('[overlay] failed to set overlay max size', error);
    });
  }, [appWindow]);

  useEffect(() => {
    if (scenario !== 3) {
      setTypedText('');
      return;
    }

    setTypedText('');
    let index = 0;
    const intervalId = window.setInterval(() => {
      index += 1;
      setTypedText(SCENARIO_TWO_TEXT.slice(0, index));
      if (index >= SCENARIO_TWO_TEXT.length) {
        window.clearInterval(intervalId);
      }
    }, 55);

    return () => {
      window.clearInterval(intervalId);
    };
  }, [scenario]);

  useEffect(() => {
    const timeoutId = window.setTimeout(() => {
      setScenario(current => {
        if (current === 1) return 2;
        if (current === 2) return 3;
        return 1;
      });
    }, 5000);

    return () => {
      window.clearTimeout(timeoutId);
    };
  }, [scenario]);

  const status: OverlayStatus = scenario === 1 ? 'idle' : 'active';

  const bubbles = useMemo<OverlayBubble[]>(() => {
    if (scenario === 1) {
      return [];
    }

    if (scenario === 2) {
      return [
        {
          id: 'assistant',
          text: '"Hey I think your coffee is getting cold. Want me to get you a new one?"',
          tone: 'accent',
        },
      ];
    }

    return [{ id: 'stt', text: typedText || ' ', tone: 'accent' }];
  }, [scenario, typedText]);

  const orbClassName = useMemo(() => {
    if (status === 'active') {
      return 'border-blue-950 bg-blue-700';
    }
    return 'border-slate-950 bg-slate-800';
  }, [status]);
  const tetrahedronInverted = status === 'active';

  return (
    <div className="flex h-screen w-screen items-end justify-end bg-transparent px-0 py-0">
      <div className="relative flex select-none flex-col items-end gap-3">
        <div className="flex max-w-[190px] flex-col items-end gap-2">
          {bubbles.map(bubble => (
            <div key={bubble.id} className="animate-[overlay-bubble-in_220ms_ease-out]">
              <OverlayBubbleChip bubble={bubble} />
            </div>
          ))}
        </div>

        <div className="relative">
          <button
            type="button"
            aria-label="Activate overlay orb"
            onClick={() => {
              setScenario(2);
            }}
            className={`group relative flex h-[56px] w-[56px] cursor-pointer items-center justify-center overflow-hidden rounded-full border transition-all duration-200 ${orbClassName}`}
            title="Click to start the demo.">
            <div className="pointer-events-none h-[92%] w-[92%] opacity-95 transition-transform duration-300 group-hover:scale-105">
              <RotatingTetrahedronCanvas inverted={tetrahedronInverted} />
            </div>
          </button>
        </div>
      </div>
    </div>
  );
}
