import { STATUS_STYLES } from '../../lib/channels/definitions';
import { useT } from '../../lib/i18n/I18nContext';
import type { ChannelConnectionStatus } from '../../types/channels';

interface ChannelStatusBadgeProps {
  status: ChannelConnectionStatus;
  className?: string;
}

const ChannelStatusBadge = ({ status, className = '' }: ChannelStatusBadgeProps) => {
  const { t } = useT();
  const style = STATUS_STYLES[status];
  return (
    <span
      className={`shrink-0 px-2 py-1 text-[11px] border rounded-full ${style.className} ${className}`}>
      {t(`channels.status.${status}`)}
    </span>
  );
};

export default ChannelStatusBadge;
