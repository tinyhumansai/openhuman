/**
 * Reusable modal for configuring a channel integration (Telegram, Discord, etc.).
 * Uses createPortal like SkillSetupModal. Can be opened from the Skills page or Settings.
 */
import { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';

import type { ChannelDefinition, ChannelType } from '../../types/channels';
import DiscordConfig from './DiscordConfig';
import TelegramConfig from './TelegramConfig';

const CHANNEL_ICONS: Record<string, string> = {
  telegram: '\u2708\uFE0F',
  discord: '\uD83C\uDFAE',
  web: '\uD83C\uDF10',
};

interface ChannelSetupModalProps {
  definition: ChannelDefinition;
  onClose: () => void;
}

function ChannelConfigContent({ definition }: { definition: ChannelDefinition }) {
  const channelId = definition.id as ChannelType;
  switch (channelId) {
    case 'telegram':
      return <TelegramConfig definition={definition} />;
    case 'discord':
      return <DiscordConfig definition={definition} />;
    default:
      return (
        <p className="text-sm text-stone-400 py-4">
          Configuration for {definition.display_name} is not available yet.
        </p>
      );
  }
}

export default function ChannelSetupModal({ definition, onClose }: ChannelSetupModalProps) {
  const modalRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    document.addEventListener('keydown', handleEscape);
    return () => document.removeEventListener('keydown', handleEscape);
  }, [onClose]);

  useEffect(() => {
    const previousFocus = document.activeElement as HTMLElement;
    modalRef.current?.focus();
    return () => {
      previousFocus?.focus?.();
    };
  }, []);

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) onClose();
  };

  const icon = CHANNEL_ICONS[definition.icon] ?? '';

  const modalContent = (
    <div
      className="fixed inset-0 z-[9999] bg-black/30 backdrop-blur-sm flex items-center justify-center p-4"
      onClick={handleBackdropClick}
      role="dialog"
      aria-modal="true"
      aria-labelledby="channel-setup-title">
      <div
        ref={modalRef}
        className="bg-white border border-stone-200 rounded-3xl shadow-large w-full max-w-[500px] overflow-hidden animate-fade-up focus:outline-none focus:ring-0"
        style={{
          animationDuration: '200ms',
          animationTimingFunction: 'cubic-bezier(0.25, 0.46, 0.45, 0.94)',
          animationFillMode: 'both',
        }}
        tabIndex={-1}
        onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className="px-5 pt-5 pb-4 border-b border-stone-200">
          <div className="flex items-start justify-between">
            <div className="flex-1 min-w-0 pr-2">
              <div className="flex items-center gap-2">
                {icon && <span className="text-base">{icon}</span>}
                <h2 id="channel-setup-title" className="text-base font-semibold text-stone-900">
                  {definition.display_name}
                </h2>
                <span className="px-1.5 py-0.5 text-[10px] font-medium rounded-md bg-primary-500/15 text-primary-600">
                  channel
                </span>
              </div>
              <p className="text-xs text-stone-500 mt-1.5">{definition.description}</p>
            </div>
            <button
              onClick={onClose}
              className="p-1 text-stone-400 hover:text-stone-900 transition-colors rounded-lg hover:bg-stone-100 flex-shrink-0">
              <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={2}
                  d="M6 18L18 6M6 6l12 12"
                />
              </svg>
            </button>
          </div>
        </div>

        {/* Content */}
        <div className="p-4 max-h-[70vh] overflow-y-auto">
          <ChannelConfigContent definition={definition} />
        </div>
      </div>
    </div>
  );

  return createPortal(modalContent, document.body);
}
