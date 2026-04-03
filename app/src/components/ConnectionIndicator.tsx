import { useAppSelector } from '../store/hooks';
import { selectSocketStatus } from '../store/socketSelectors';

interface ConnectionIndicatorProps {
  status?: 'connected' | 'disconnected' | 'connecting';
  description?: string;
  className?: string;
}

const ConnectionIndicator = ({
  status: overrideStatus,
  className = '',
}: ConnectionIndicatorProps) => {
  // Use socket store status, but allow override via props
  const storeStatus = useAppSelector(selectSocketStatus);
  const status = overrideStatus || storeStatus;
  const statusConfig = {
    connected: {
      color: 'bg-sage-500',
      textColor: 'text-sage-500',
      text: 'Connected to OpenHuman AI 🚀',
    },
    disconnected: { color: 'bg-coral-500', textColor: 'text-coral-500', text: 'Disconnected' },
    connecting: { color: 'bg-amber-500', textColor: 'text-amber-500', text: 'Connecting' },
  };

  const config = statusConfig[status];

  return (
    <div className={`${className}`}>
      <div className="inline-flex items-center gap-1.5 px-3 py-1 rounded-full bg-stone-50 border border-stone-200">
        <div
          className={`w-2 h-2 ${config.color} rounded-full ${status === 'connected' ? 'animate-pulse' : ''}`}
        />
        <span className={`text-xs font-medium ${config.textColor}`}>{config.text}</span>
      </div>
    </div>
  );
};

export default ConnectionIndicator;
