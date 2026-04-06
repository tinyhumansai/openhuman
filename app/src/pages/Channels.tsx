import { useState } from 'react';

import ChannelConfigPanel from '../components/channels/ChannelConfigPanel';
import ChannelSelector from '../components/channels/ChannelSelector';
import { useChannelDefinitions } from '../hooks/useChannelDefinitions';
import type { ChannelType } from '../types/channels';

const Channels = () => {
  const { definitions, loading, error } = useChannelDefinitions();
  const [selectedChannel, setSelectedChannel] = useState<ChannelType>('telegram');

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="flex-1 overflow-y-auto p-6 space-y-6">
        {error && (
          <div className="rounded-lg border border-coral-200 bg-coral-50 px-4 py-3 text-sm text-coral-700">
            {error}
          </div>
        )}

        {loading ? (
          <div className="rounded-xl border border-stone-200 bg-white p-6 text-sm text-stone-400">
            Loading channel definitions...
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
