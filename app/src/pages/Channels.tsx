import { useState } from 'react';

import ChannelConfigPanel from '../components/channels/ChannelConfigPanel';
import ChannelSelector from '../components/channels/ChannelSelector';
import { useChannelDefinitions } from '../hooks/useChannelDefinitions';
import { useT } from '../lib/i18n/I18nContext';
import type { ChannelType } from '../types/channels';

const Channels = () => {
  const { t } = useT();
  const { definitions, loading, error } = useChannelDefinitions();
  const [selectedChannel, setSelectedChannel] = useState<ChannelType>('telegram');

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="flex-1 overflow-y-auto p-6 space-y-6">
        {error && (
          <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300">
            {error}
          </div>
        )}

        {loading ? (
          <div className="rounded-xl border border-line bg-surface p-6 text-sm text-content-faint">
            {t('common.loading')}
          </div>
        ) : (
          <>
            <ChannelSelector
              definitions={definitions}
              selectedChannel={selectedChannel}
              onSelectChannel={setSelectedChannel}
            />
            <ChannelConfigPanel selectedChannel={selectedChannel} definitions={definitions} />
          </>
        )}
      </div>
    </div>
  );
};

export default Channels;
