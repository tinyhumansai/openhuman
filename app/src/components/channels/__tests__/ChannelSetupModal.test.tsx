import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { FALLBACK_DEFINITIONS } from '../../../lib/channels/definitions';
import { renderWithProviders } from '../../../test/test-utils';
import type { ChannelDefinition } from '../../../types/channels';
import ChannelSetupModal from '../ChannelSetupModal';

// YuanbaoConfig pulls in API + Tauri helpers we don't need for the routing
// branches under test — stub it so we only assert ChannelSetupModal's own
// behavior (icon branch + yuanbao switch case).
vi.mock('../YuanbaoConfig', () => ({
  default: () => <div data-testid="yuanbao-config">Yuanbao Config</div>,
}));

vi.mock('../TelegramConfig', () => ({
  default: () => <div data-testid="telegram-config">Telegram Config</div>,
}));

vi.mock('../DiscordConfig', () => ({
  default: () => <div data-testid="discord-config">Discord Config</div>,
}));

const yuanbaoDef: ChannelDefinition = {
  id: 'yuanbao',
  display_name: '元宝',
  description: '通过元宝（Yuanbao）机器人收发消息。',
  icon: 'yuanbao',
  auth_modes: [
    {
      mode: 'api_key',
      description: '提供元宝开放平台的 AppID 和 AppSecret。',
      fields: [],
      auth_action: undefined,
    },
  ],
  capabilities: ['send_text', 'receive_text'],
};

describe('ChannelSetupModal', () => {
  it('renders the YuanbaoConfig body and brand SVG icon for the yuanbao channel', () => {
    renderWithProviders(<ChannelSetupModal definition={yuanbaoDef} onClose={() => {}} />);
    // Header title + body routing both exercised.
    expect(screen.getByText('元宝')).toBeInTheDocument();
    expect(screen.getByTestId('yuanbao-config')).toBeInTheDocument();
    // YuanbaoIcon emits an aria-hidden SVG in the header; the emoji-based
    // fallback should NOT also render for yuanbao.
    const dialog = screen.getByRole('dialog');
    expect(dialog.querySelector('svg[aria-hidden="true"]')).not.toBeNull();
  });

  it('renders the emoji icon and TelegramConfig body for the telegram channel', () => {
    const telegramDef = FALLBACK_DEFINITIONS.find(d => d.id === 'telegram')!;
    renderWithProviders(<ChannelSetupModal definition={telegramDef} onClose={() => {}} />);
    expect(screen.getByTestId('telegram-config')).toBeInTheDocument();
    // Emoji branch produces a span sibling to the title.
    expect(screen.getByText('\u2708\uFE0F')).toBeInTheDocument();
  });

  it('falls back to the unavailable-channel message for an unknown channel id', () => {
    const unknown: ChannelDefinition = { ...yuanbaoDef, id: 'unknown', display_name: 'Unknown' };
    renderWithProviders(<ChannelSetupModal definition={unknown} onClose={() => {}} />);
    expect(screen.getByText(/Configuration for/i)).toBeInTheDocument();
  });

  it('invokes onClose when the Escape key is pressed', () => {
    const onClose = vi.fn();
    renderWithProviders(<ChannelSetupModal definition={yuanbaoDef} onClose={onClose} />);
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
