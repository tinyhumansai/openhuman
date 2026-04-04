import { STATUS_STYLES } from '../../lib/channels/definitions';
import type { ChannelConnectionStatus } from '../../types/channels';

interface ChannelStatusBadgeProps {
  status: ChannelConnectionStatus;
  className?: string;
}

const ChannelStatusBadge = ({ status, className = '' }: ChannelStatusBadgeProps) => {
  const style = STATUS_STYLES[status];
  return (
    <span
      className={`shrink-0 px-2 py-1 text-[11px] border rounded-full ${style.className} ${className}`}>
      {style.label}
    </span>
  );
};

export default ChannelStatusBadge;
