import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { channelConnectionsApi } from '../../services/api/channelConnectionsApi';
import type { BotPermissionCheck, DiscordGuild, DiscordTextChannel } from '../../types/channels';

const log = debug('channels:discord:picker');

interface DiscordServerChannelPickerProps {
  selectedGuildId?: string;
  selectedChannelId?: string;
  onGuildSelected?: (guildId: string) => void;
  onChannelSelected?: (channelId: string) => void;
}

type PickerState =
  | 'idle'
  | 'loading_guilds'
  | 'guilds_loaded'
  | 'loading_channels'
  | 'channels_loaded'
  | 'checking_permissions'
  | 'ready'
  | 'error';

const DiscordServerChannelPicker = ({
  selectedGuildId: selectedGuildIdProp,
  selectedChannelId: selectedChannelIdProp,
  onGuildSelected,
  onChannelSelected,
}: DiscordServerChannelPickerProps) => {
  const [state, setState] = useState<PickerState>('idle');
  const [guilds, setGuilds] = useState<DiscordGuild[]>([]);
  const [channels, setChannels] = useState<DiscordTextChannel[]>([]);
  const [selectedGuildId, setSelectedGuildId] = useState<string>(selectedGuildIdProp ?? '');
  const [selectedChannelId, setSelectedChannelId] = useState<string>(selectedChannelIdProp ?? '');
  const [permissions, setPermissions] = useState<BotPermissionCheck | null>(null);
  const [error, setError] = useState<string | null>(null);
  const channelsRequestIdRef = useRef(0);
  const permissionsRequestIdRef = useRef(0);

  useEffect(() => {
    setSelectedGuildId(selectedGuildIdProp ?? '');
  }, [selectedGuildIdProp]);

  useEffect(() => {
    setSelectedChannelId(selectedChannelIdProp ?? '');
  }, [selectedChannelIdProp]);

  // Load guilds on mount
  useEffect(() => {
    const loadGuilds = async () => {
      setState('loading_guilds');
      setError(null);
      try {
        const result = await channelConnectionsApi.listDiscordGuilds();
        setGuilds(result);
        setState('guilds_loaded');
        log('loaded %d guilds', result.length);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        setError(msg);
        setState('error');
        log('failed to load guilds: %s', msg);
      }
    };
    void loadGuilds();
  }, []);

  const handleGuildChange = useCallback(
    (guildId: string) => {
      channelsRequestIdRef.current += 1;
      permissionsRequestIdRef.current += 1;
      setSelectedGuildId(guildId);
      setSelectedChannelId('');
      setChannels([]);
      setPermissions(null);
      onGuildSelected?.(guildId);
      onChannelSelected?.('');

      if (!guildId) {
        setState('guilds_loaded');
        return;
      }

      const loadChannels = async () => {
        const requestId = channelsRequestIdRef.current;
        setState('loading_channels');
        setError(null);
        try {
          const result = await channelConnectionsApi.listDiscordChannels(guildId);
          if (requestId !== channelsRequestIdRef.current) {
            return;
          }
          setChannels(result);
          setState('channels_loaded');
          log('loaded %d channels for guild %s', result.length, guildId);
        } catch (e) {
          if (requestId !== channelsRequestIdRef.current) {
            return;
          }
          const msg = e instanceof Error ? e.message : String(e);
          setError(msg);
          setState('error');
        }
      };
      void loadChannels();
    },
    [onChannelSelected, onGuildSelected]
  );

  const handleChannelChange = useCallback(
    (channelId: string) => {
      permissionsRequestIdRef.current += 1;
      setSelectedChannelId(channelId);
      setPermissions(null);
      onChannelSelected?.(channelId);

      if (!channelId || !selectedGuildId) {
        setState('channels_loaded');
        return;
      }

      const checkPerms = async () => {
        const requestId = permissionsRequestIdRef.current;
        setState('checking_permissions');
        setError(null);
        try {
          const result = await channelConnectionsApi.checkDiscordPermissions(
            selectedGuildId,
            channelId
          );
          if (requestId !== permissionsRequestIdRef.current) {
            return;
          }
          setPermissions(result);
          setState('ready');
          log('permissions for channel %s: %o', channelId, result);
        } catch (e) {
          if (requestId !== permissionsRequestIdRef.current) {
            return;
          }
          const msg = e instanceof Error ? e.message : String(e);
          setError(msg);
          setState('error');
        }
      };
      void checkPerms();
    },
    [selectedGuildId, onChannelSelected]
  );

  // Group channels by category
  const groupedChannels = channels.reduce<Record<string, DiscordTextChannel[]>>((acc, ch) => {
    const key = ch.parent_id ?? '__uncategorized';
    if (!acc[key]) acc[key] = [];
    acc[key].push(ch);
    return acc;
  }, {});

  const isLoading =
    state === 'loading_guilds' || state === 'loading_channels' || state === 'checking_permissions';

  return (
    <div className="mt-3 space-y-3">
      <p className="text-xs font-medium text-stone-600">Server &amp; Channel Selection</p>

      {/* Error banner */}
      {error && (
        <div className="rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-xs text-coral-700">
          {error}
        </div>
      )}

      {/* Guild selector */}
      <div>
        <label htmlFor="discord-guild-select" className="block text-xs text-stone-500 mb-1">
          Server
        </label>
        <select
          id="discord-guild-select"
          value={selectedGuildId}
          onChange={e => handleGuildChange(e.target.value)}
          disabled={isLoading || guilds.length === 0}
          className="w-full rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 focus:border-primary-500 focus:outline-none disabled:opacity-50">
          <option value="">
            {state === 'loading_guilds'
              ? 'Loading servers...'
              : guilds.length === 0
                ? 'No servers found'
                : 'Select a server'}
          </option>
          {guilds.map(g => (
            <option key={g.id} value={g.id}>
              {g.name}
            </option>
          ))}
        </select>
        {guilds.length === 0 && state === 'guilds_loaded' && (
          <p className="mt-1 text-xs text-stone-400">
            The bot is not in any servers. Invite it using the Discord Developer Portal.
          </p>
        )}
      </div>

      {/* Channel selector */}
      {selectedGuildId && (
        <div>
          <label htmlFor="discord-channel-select" className="block text-xs text-stone-500 mb-1">
            Channel
          </label>
          <select
            id="discord-channel-select"
            value={selectedChannelId}
            onChange={e => handleChannelChange(e.target.value)}
            disabled={isLoading || channels.length === 0}
            className="w-full rounded-lg border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 focus:border-primary-500 focus:outline-none disabled:opacity-50">
            <option value="">
              {state === 'loading_channels'
                ? 'Loading channels...'
                : channels.length === 0
                  ? 'No text channels found'
                  : 'Select a channel'}
            </option>
            {Object.entries(groupedChannels).map(([categoryId, chs]) => {
              if (categoryId === '__uncategorized') {
                return chs.map(ch => (
                  <option key={ch.id} value={ch.id}>
                    # {ch.name}
                  </option>
                ));
              }
              return (
                <optgroup key={categoryId} label={`Category ${categoryId}`}>
                  {chs.map(ch => (
                    <option key={ch.id} value={ch.id}>
                      # {ch.name}
                    </option>
                  ))}
                </optgroup>
              );
            })}
          </select>
        </div>
      )}

      {/* Permission check result */}
      {state === 'checking_permissions' && (
        <div className="flex items-center gap-2 text-xs text-stone-500">
          <span className="inline-block h-3 w-3 animate-spin rounded-full border-2 border-stone-300 border-t-primary-500" />
          Checking permissions...
        </div>
      )}

      {permissions && state === 'ready' && (
        <div
          className={`rounded-lg border px-3 py-2 text-xs ${
            permissions.missing_permissions.length === 0
              ? 'border-sage-200 bg-sage-50 text-sage-700'
              : 'border-amber-200 bg-amber-50 text-amber-700'
          }`}>
          {permissions.missing_permissions.length === 0 ? (
            <span>Bot has all required permissions in this channel.</span>
          ) : (
            <div>
              <span className="font-medium">Missing permissions: </span>
              {permissions.missing_permissions.join(', ')}
            </div>
          )}
        </div>
      )}
    </div>
  );
};

export default DiscordServerChannelPicker;
