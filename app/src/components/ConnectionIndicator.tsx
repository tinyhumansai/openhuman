import { useT } from '../lib/i18n/I18nContext';
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
  const { t } = useT();
  const blocking = useAppSelector(selectBlockingState);
  const legacyStatus = useAppSelector(selectSocketStatus);

  const config: StatusConfig = (() => {
    if (overrideStatus) {
      return legacyMap(t)[overrideStatus];
    }
    switch (blocking) {
      case 'ok':
        return {
          color: 'bg-sage-500',
          textColor: 'text-sage-500',
          text: t('app.connectionIndicator.connected'),
          pulse: true,
        };
      case 'internet-offline':
        return {
          color: 'bg-coral-500',
          textColor: 'text-coral-500',
          text: t('app.connectionIndicator.offline'),
          pulse: false,
        };
      case 'core-unreachable':
        return {
          color: 'bg-amber-500',
          textColor: 'text-amber-500',
          text: t('app.connectionIndicator.coreOffline'),
          pulse: false,
        };
      case 'backend-only':
        return {
          color: 'bg-amber-500',
          textColor: 'text-amber-500',
          text:
            legacyStatus === 'connecting'
              ? t('app.connectionIndicator.connecting')
              : t('app.connectionIndicator.reconnecting'),
          pulse: false,
        };
    }
  })();

  // Simplified two-state label (the dot colour still reflects the nuanced
  // connecting/reconnecting states).
  const isConnected = overrideStatus ? overrideStatus === 'connected' : blocking === 'ok';
  const label = isConnected
    ? t('app.connectionIndicator.connected')
    : t('app.connectionIndicator.disconnected');

  return (
    <div className={`inline-flex items-center gap-1.5 ${className}`}>
      <div
        className={`w-2 h-2 ${config.color} rounded-full ${config.pulse ? 'animate-pulse' : ''}`}
      />
      <span className={`text-[10px] font-medium ${config.textColor}`}>{label}</span>
    </div>
  );
};

const legacyMap = (
  t: (k: string) => string
): Record<'connected' | 'disconnected' | 'connecting', StatusConfig> => ({
  connected: {
    color: 'bg-sage-500',
    textColor: 'text-sage-500',
    text: t('app.connectionIndicator.connected'),
    pulse: true,
  },
  disconnected: {
    color: 'bg-coral-500',
    textColor: 'text-coral-500',
    text: t('app.connectionIndicator.disconnected'),
    pulse: false,
  },
  connecting: {
    color: 'bg-amber-500',
    textColor: 'text-amber-500',
    text: t('app.connectionIndicator.connecting'),
    pulse: false,
  },
});

export default ConnectionIndicator;
