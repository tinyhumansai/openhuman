import {
  createContext,
  type FC,
  type ReactNode,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';

/**
 * Local replacements for Remotion's `useCurrentFrame` and `useVideoConfig`.
 *
 * `@remotion/player` was reliably starting only after the user blurred and
 * refocused the window in CEF — its internal play() races with audio-context /
 * focus-event scheduling on cold mount and the SVG paints frame 0 then sits
 * idle. Since the mascot compositions only use `useCurrentFrame` /
 * `useVideoConfig` from Remotion (everything else is pure utilities like
 * `interpolate` / `Easing`), we drive frame ticks ourselves via
 * requestAnimationFrame and feed both hooks via plain React context.
 */

interface FrameConfig {
  fps: number;
  width: number;
  height: number;
  durationInFrames: number;
}

const FrameContext = createContext<number>(0);
const FrameConfigContext = createContext<FrameConfig | null>(null);

export const useCurrentFrame = (): number => useContext(FrameContext);

export const useVideoConfig = (): FrameConfig => {
  const cfg = useContext(FrameConfigContext);
  if (!cfg) {
    throw new Error('useVideoConfig() must be used inside <FrameProvider>');
  }
  return cfg;
};

interface FrameProviderProps extends FrameConfig {
  children: ReactNode;
}

export const FrameProvider: FC<FrameProviderProps> = ({
  fps,
  width,
  height,
  durationInFrames,
  children,
}) => {
  const [frame, setFrame] = useState(0);
  const startRef = useRef<number | null>(null);

  useEffect(() => {
    let raf = 0;
    const tick = (now: number) => {
      if (startRef.current === null) startRef.current = now;
      const elapsed = now - startRef.current;
      const next = Math.floor((elapsed / 1000) * fps) % durationInFrames;
      setFrame(prev => (prev === next ? prev : next));
      raf = window.requestAnimationFrame(tick);
    };
    raf = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(raf);
  }, [fps, durationInFrames]);

  const config = useMemo<FrameConfig>(
    () => ({ fps, width, height, durationInFrames }),
    [fps, width, height, durationInFrames]
  );

  return (
    <FrameConfigContext.Provider value={config}>
      <FrameContext.Provider value={frame}>{children}</FrameContext.Provider>
    </FrameConfigContext.Provider>
  );
};
