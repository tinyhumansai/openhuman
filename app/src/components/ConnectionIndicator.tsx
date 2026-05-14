import { selectBlockingState } from '../store/connectivitySelectors';
import { useAppSelector } from '../store/hooks';
import { selectSocketStatus } from '../store/socketSelectors';

interface ConnectionIndicatorProps {
  /**
   * Optional override — used by storybook fixtures and a couple of legacy
   * call sites that still drive a single 3-state pill from local state. New
   * code should NOT pass this; let the indicator read connectivitySlice.
   */
  status?: 'connected' | 'disconnected' | 'connecting';
  className?: string;
}

interface StatusConfig {
  color: string;
  textColor: string;
  text: string;
  pulse: boolean;
}

/**
 * 3-channel connectivity chip (#1527).
 *
 * Reads `selectBlockingState`, which encodes the user-visible precedence:
 * internet > core > backend. The legacy `status` prop and `selectSocketStatus`
 * fallback are retained so existing call sites that pre-date the split keep
 * rendering correctly during rollout.
 */
const ConnectionIndicator = ({
  status: overrideStatus,
  className = '',
}: ConnectionIndicatorProps) => {
  const blocking = useAppSelector(selectBlockingState);
  const legacyStatus = useAppSelector(selectSocketStatus);

  const config: StatusConfig = (() => {
    if (overrideStatus) {
      return legacyMap[overrideStatus];
    }
    switch (blocking) {
      case 'ok':
        return {
          color: 'bg-sage-500',
          textColor: 'text-sage-500',
          text: 'Connected to OpenHuman AI 🚀',
          pulse: true,
        };
      case 'internet-offline':
        return {
          color: 'bg-coral-500',
          textColor: 'text-coral-500',
          text: 'Offline',
          pulse: false,
        };
      case 'core-unreachable':
        return {
          color: 'bg-amber-500',
          textColor: 'text-amber-500',
          text: 'Core offline',
          pulse: false,
        };
      case 'backend-only':
        return {
          color: 'bg-amber-500',
          textColor: 'text-amber-500',
          text: legacyStatus === 'connecting' ? 'Connecting' : 'Reconnecting…',
          pulse: false,
        };
    }
  })();

  return (
    <div className={`${className}`}>
      <div className="inline-flex items-center gap-1.5 px-3 py-1 rounded-full bg-stone-50 border border-stone-200">
        <div
          className={`w-2 h-2 ${config.color} rounded-full ${config.pulse ? 'animate-pulse' : ''}`}
        />
        <span className={`text-xs font-medium ${config.textColor}`}>{config.text}</span>
      </div>
    </div>
  );
};

const legacyMap: Record<'connected' | 'disconnected' | 'connecting', StatusConfig> = {
  connected: {
    color: 'bg-sage-500',
    textColor: 'text-sage-500',
    text: 'Connected to OpenHuman AI 🚀',
    pulse: true,
  },
  disconnected: {
    color: 'bg-coral-500',
    textColor: 'text-coral-500',
    text: 'Disconnected',
    pulse: false,
  },
  connecting: {
    color: 'bg-amber-500',
    textColor: 'text-amber-500',
    text: 'Connecting',
    pulse: false,
  },
};

export default ConnectionIndicator;
