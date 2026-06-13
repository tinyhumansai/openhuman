import { useT } from '../../lib/i18n/I18nContext';
import type { ChannelDefinition } from '../../types/channels';
import ChannelStatusBadge from './ChannelStatusBadge';

interface WebChannelConfigProps {
  definition: ChannelDefinition;
}

const WebChannelConfig = ({ definition: _definition }: WebChannelConfigProps) => {
  const { t } = useT();
  return (
    <div className="space-y-3">
      <div className="flex items-start justify-between">
        <div>
          <ChannelStatusBadge status="connected" />
        </div>
      </div>
      <p className="text-sm text-stone-500 dark:text-neutral-400">
        {t('channels.web.alwaysAvailable')}
      </p>
    </div>
  );
};

export default WebChannelConfig;
