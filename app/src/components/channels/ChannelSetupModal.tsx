/**
 * Reusable modal for configuring a channel integration (Telegram, Discord, etc.).
 * Uses createPortal like SkillSetupModal. Can be opened from the Skills page or Settings.
 */
import { useEffect, useRef } from 'react';
import { createPortal } from 'react-dom';

import { useEscapeKey } from '../../hooks/useEscapeKey';
import { useT } from '../../lib/i18n/I18nContext';
import type { ChannelDefinition, ChannelType } from '../../types/channels';
import { CloseIcon } from '../ui';
import { renderChannelIcon } from './channelIcon';
import DiscordConfig from './DiscordConfig';
import TelegramConfig from './TelegramConfig';
import YuanbaoConfig from './YuanbaoConfig';

interface ChannelSetupModalProps {
  definition: ChannelDefinition;
  onClose: () => void;
}

function ChannelConfigContent({ definition }: { definition: ChannelDefinition }) {
  const { t } = useT();
  const channelId = definition.id as ChannelType;
  switch (channelId) {
    case 'telegram':
      return <TelegramConfig definition={definition} />;
    case 'discord':
      return <DiscordConfig definition={definition} />;
    case 'yuanbao':
      return <YuanbaoConfig definition={definition} />;
    default:
      return (
        <p className="text-sm text-stone-400 dark:text-neutral-500 py-4">
          {t('channels.configNotAvailable')} {definition.display_name}
        </p>
      );
  }
}

export default function ChannelSetupModal({ definition, onClose }: ChannelSetupModalProps) {
  const { t } = useT();
  const modalRef = useRef<HTMLDivElement>(null);

  useEscapeKey(onClose);

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

  const modalContent = (
    <div
      className="fixed inset-0 z-[9999] bg-black/30 backdrop-blur-sm flex items-center justify-center p-4"
      onClick={handleBackdropClick}
      role="dialog"
      aria-modal="true"
      aria-labelledby="channel-setup-title">
      <div
        ref={modalRef}
        className="bg-white dark:bg-neutral-900 border border-stone-200 dark:border-neutral-800 rounded-3xl shadow-large w-full max-w-[500px] overflow-hidden animate-fade-up focus:outline-none focus:ring-0"
        style={{
          animationDuration: '200ms',
          animationTimingFunction: 'cubic-bezier(0.25, 0.46, 0.45, 0.94)',
          animationFillMode: 'both',
        }}
        tabIndex={-1}
        onClick={e => e.stopPropagation()}>
        {/* Header */}
        <div className="px-5 pt-5 pb-4 border-b border-stone-200 dark:border-neutral-800">
          <div className="flex items-start justify-between">
            <div className="flex-1 min-w-0 pr-2">
              <div className="flex items-center gap-2">
                {renderChannelIcon(definition.icon)}
                <h2
                  id="channel-setup-title"
                  className="text-base font-semibold text-stone-900 dark:text-neutral-100">
                  {definition.display_name}
                </h2>
                <span className="px-1.5 py-0.5 text-[10px] font-medium rounded-md bg-primary-500/15 text-primary-600 dark:text-primary-300">
                  {t('channels.channel')}
                </span>
              </div>
              <p className="text-xs text-stone-500 dark:text-neutral-400 mt-1.5">
                {definition.description}
              </p>
            </div>
            <button
              onClick={onClose}
              className="p-1 text-stone-400 dark:text-neutral-500 hover:text-stone-900 dark:hover:text-neutral-100 dark:text-neutral-100 dark:hover:text-neutral-100 transition-colors rounded-lg hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 dark:hover:bg-neutral-800/60 flex-shrink-0">
              <CloseIcon className="w-5 h-5" />
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
