import { useEffect, useState } from 'react';

export type AnimationState = 'entering' | 'entered' | 'exiting' | 'exited';

interface SettingsAnimationHook {
  isVisible: boolean;
  animationState: AnimationState;
  startEntry: () => void;
  startExit: () => void;
}

export const useSettingsAnimation = (duration = 300): SettingsAnimationHook => {
  const [animationState, setAnimationState] = useState<AnimationState>('exited');

  const isVisible = animationState === 'entering' || animationState === 'entered';

  const startEntry = () => {
    setAnimationState('entering');
    setTimeout(() => {
      setAnimationState('entered');
    }, duration);
  };

  const startExit = () => {
    setAnimationState('exiting');
    setTimeout(() => {
      setAnimationState('exited');
    }, duration);
  };

  return { isVisible, animationState, startEntry, startExit };
};

// Hook for panel slide animations (slide from right)
export const usePanelAnimation = (isActive: boolean, duration = 300) => {
  const [mounted, setMounted] = useState(isActive);

  useEffect(() => {
    if (isActive) {
      setMounted(true);
    } else {
      const timer = setTimeout(() => setMounted(false), duration);
      return () => clearTimeout(timer);
    }
  }, [isActive, duration]);

  const getPanelClasses = () => {
    const baseClasses = 'transition-all duration-300 ease-[cubic-bezier(0.25,0.46,0.45,0.94)]';
    if (!mounted) return `${baseClasses} opacity-0`;

    return isActive
      ? `${baseClasses} opacity-100 translate-x-0`
      : `${baseClasses} opacity-0 translate-x-full`;
  };

  return { mounted, panelClasses: getPanelClasses() };
};
