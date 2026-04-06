import type { ChannelDefinition } from '../../types/channels';
import ChannelStatusBadge from './ChannelStatusBadge';

interface WebChannelConfigProps {
  definition: ChannelDefinition;
}

const WebChannelConfig = ({ definition: _definition }: WebChannelConfigProps) => {
  return (
    <div className="space-y-3">
      <div className="flex items-start justify-between">
        <div>
          <ChannelStatusBadge status="connected" />
        </div>
      </div>
      <p className="text-sm text-stone-500">
        The web channel is always available — no setup required.
      </p>
    </div>
  );
};

export default WebChannelConfig;
