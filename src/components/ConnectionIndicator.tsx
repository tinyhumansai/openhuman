import { useAppSelector } from '../store/hooks';
import { selectSocketStatus } from '../store/socketSelectors';

interface ConnectionIndicatorProps {
  status?: 'connected' | 'disconnected' | 'connecting';
  description?: string;
  className?: string;
}

const ConnectionIndicator = ({
  status: overrideStatus,
  description = 'Your browser is now connected to the AlphaHuman AI. Keep the app running to keep the connection alive. You can message your assistant with the button below.',
  className = '',
}: ConnectionIndicatorProps) => {
  // Use socket store status, but allow override via props
  const storeStatus = useAppSelector(selectSocketStatus);
  const status = overrideStatus || storeStatus;
  const statusConfig = {
    connected: {
      color: 'bg-sage-500',
      textColor: 'text-sage-500',
      text: 'Connected to AlphaHuman AI 🚀',
    },
    disconnected: { color: 'bg-coral-500', textColor: 'text-coral-500', text: 'Disconnected' },
    connecting: { color: 'bg-amber-500', textColor: 'text-amber-500', text: 'Connecting' },
  };

  const config = statusConfig[status];

  return (
    <div className={`mb-6 ${className}`}>
      <div className="flex items-center justify-center space-x-2 mb-3">
        <div
          className={`w-2 h-2 ${config.color} rounded-full ${status === 'connected' ? 'animate-pulse' : ''}`}></div>
        <span className={`text-sm ${config.textColor}`}>{config.text}</span>
      </div>
      {description && (
        <p className="text-xs opacity-60 text-center leading-relaxed">{description}</p>
      )}
    </div>
  );
};

export default ConnectionIndicator;
