import TelegramIcon from '../assets/icons/telegram.svg';
import { useSkillConnectionStatus } from '../lib/skills/hooks';
import type { SkillConnectionStatus } from '../lib/skills/types';

interface TelegramConnectionIndicatorProps {
  className?: string;
}

const STATUS_CONFIG: Record<SkillConnectionStatus, { dot: string; text: string; label: string }> = {
  connected: { dot: 'bg-sage-500', text: 'text-sage-500', label: 'Telegram Connected' },
  connecting: { dot: 'bg-amber-500', text: 'text-amber-500', label: 'Telegram Connecting...' },
  not_authenticated: {
    dot: 'bg-amber-500',
    text: 'text-amber-500',
    label: 'Telegram Not Authenticated',
  },
  disconnected: { dot: 'bg-gray-500', text: 'text-gray-500', label: 'Telegram Disconnected' },
  error: { dot: 'bg-coral-500', text: 'text-coral-500', label: 'Telegram Error' },
  offline: { dot: 'bg-gray-500', text: 'text-gray-500', label: 'Telegram Offline' },
  setup_required: { dot: 'bg-gray-500', text: 'text-gray-500', label: 'Telegram Not Set Up' },
};

const TelegramConnectionIndicator = ({ className = '' }: TelegramConnectionIndicatorProps) => {
  const status = useSkillConnectionStatus('telegram');
  const config = STATUS_CONFIG[status];
  const isActive = status === 'connected';

  return (
    <div className={className}>
      <div className="flex items-center justify-center space-x-2">
        <div className={`w-2 h-2 ${config.dot} rounded-full ${isActive ? 'animate-pulse' : ''}`} />
        <div className="flex items-center space-x-1.5">
          <img
            src={TelegramIcon}
            alt="Telegram"
            className={`w-4 h-4 ${isActive ? 'opacity-100' : 'opacity-50'}`}
          />
          <span className={`text-sm ${config.text}`}>{config.label}</span>
        </div>
      </div>
    </div>
  );
};

export default TelegramConnectionIndicator;
