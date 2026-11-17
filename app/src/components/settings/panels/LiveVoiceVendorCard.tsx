import { Settings, Sparkles } from 'lucide-react';

import { cn } from '../../../lib/cn';
import { useT } from '../../../lib/i18n/I18nContext';
import Badge from '../../ui/Badge';
import Button from '../../ui/Button';
import LiveVoiceVendorLogo from './LiveVoiceVendorLogo';
import { type LiveVoiceVendor, vendorDescKey } from './liveVoiceVendors';

/** The highlighted "Included with TinyHumans" tag on managed voice agents. */
export const IncludedTag = ({ className, label }: { className?: string; label?: string }) => {
  const { t } = useT();
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1 rounded-full bg-primary-500/10 px-2 py-0.5 text-[11px] font-medium text-primary-600 ring-1 ring-primary-500/20 ring-inset dark:text-primary-300',
        className
      )}>
      <Sparkles className="h-3 w-3" aria-hidden />
      {label ?? t('connections.voiceAgents.includedTag')}
    </span>
  );
};

export interface LiveVoiceVendorCardProps {
  vendor: LiveVoiceVendor;
  /** The provider id the live voice agent currently talks through. */
  defaultProvider: string;
  saving: boolean;
  /** Make this vendor's preferred ready provider the one in use. */
  onUse: (providerId: string) => void;
  onOpenSettings: () => void;
}

/**
 * One voice service on Connections → Voice agents: what it is, whether it
 * comes with TinyHumans, whether it is the one in use, and the way into its
 * settings.
 */
const LiveVoiceVendorCard = ({
  vendor,
  defaultProvider,
  saving,
  onUse,
  onOpenSettings,
}: LiveVoiceVendorCardProps) => {
  const { t } = useT();
  const descKey = vendorDescKey(vendor.id);
  const inUse = vendor.providers.some(p => p.id === defaultProvider);
  const hasHosted = vendor.providers.some(p => p.kind === 'hosted');
  const ready = vendor.providers.find(p => p.configured);

  const status = inUse ? (
    <Badge variant="primary">{t('connections.voiceAgents.badgeInUse')}</Badge>
  ) : ready ? (
    <Badge variant="success">{t('connections.voiceAgents.badgeReady')}</Badge>
  ) : (
    <Badge variant="warning">{t('connections.voiceAgents.badgeNeedsKey')}</Badge>
  );

  return (
    <div
      data-testid={`live-voice-vendor-${vendor.id}`}
      data-in-use={inUse || undefined}
      className={cn(
        'flex h-full flex-col gap-3 rounded-xl border p-4 transition-colors',
        inUse
          ? 'border-primary-500 bg-primary-50 ring-1 ring-primary-500 dark:bg-primary-500/10'
          : 'border-line bg-surface'
      )}>
      <div className="flex items-start gap-3">
        <span
          className={cn(
            'flex h-11 w-11 shrink-0 items-center justify-center rounded-xl',
            inUse ? 'bg-primary-500 text-content-inverted' : 'bg-surface-strong text-content'
          )}>
          <LiveVoiceVendorLogo vendorId={vendor.id} className="h-5.5 w-5.5" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1">
            <h4 className="text-sm font-semibold text-content">{vendor.name}</h4>
            {status}
          </div>
          {descKey && (
            <p className="mt-0.5 text-xs leading-relaxed text-content-muted">{t(descKey)}</p>
          )}
        </div>
      </div>

      <div className="mt-auto flex flex-wrap items-center justify-between gap-2">
        {hasHosted ? <IncludedTag /> : <Badge>{t('connections.voiceAgents.ownKeyTag')}</Badge>}
        <div className="ml-auto flex items-center gap-1.5">
          {!inUse && ready && (
            <Button
              size="sm"
              analyticsId="live-voice-use-vendor"
              data-testid={`live-voice-use-vendor-${vendor.id}`}
              disabled={saving}
              onClick={() => onUse(ready.id)}>
              {t('connections.voiceAgents.use')}
            </Button>
          )}
          <Button
            size="sm"
            variant="secondary"
            iconOnly
            analyticsId="live-voice-open-settings"
            data-testid={`live-voice-settings-${vendor.id}`}
            aria-label={t('connections.voiceAgents.settingsAria').replace('{name}', vendor.name)}
            title={t('connections.voiceAgents.settings')}
            onClick={onOpenSettings}>
            <Settings className="h-4 w-4" aria-hidden />
          </Button>
        </div>
      </div>
    </div>
  );
};

export default LiveVoiceVendorCard;
