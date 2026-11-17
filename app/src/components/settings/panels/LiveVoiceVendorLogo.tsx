import { Mic } from 'lucide-react';
import type { IconType } from 'react-icons';
import { SiElevenlabs, SiGooglegemini } from 'react-icons/si';

import sarvamLogo from '../../../assets/provider-icons/sarvam.svg';
import { cn } from '../../../lib/cn';

/** Simple Icons marks for vendors in the installed `react-icons` set. */
const VENDOR_ICONS: Record<string, IconType> = { gemini: SiGooglegemini, elevenlabs: SiElevenlabs };

/**
 * Bundled single-colour marks for vendors Simple Icons lacks, drawn as a CSS
 * mask so they take the surrounding text colour in both themes. Sarvam's is
 * its published brand mark (assets.sarvam.ai/assets/brand/logos).
 */
const VENDOR_MASKS: Record<string, string> = { sarvam: sarvamLogo };

/** A voice vendor's brand mark, falling back to a microphone for unknown vendors. */
const LiveVoiceVendorLogo = ({ vendorId, className }: { vendorId: string; className?: string }) => {
  const Icon = VENDOR_ICONS[vendorId];
  if (Icon) return <Icon className={className} aria-hidden />;
  const mask = VENDOR_MASKS[vendorId];
  if (mask) {
    return (
      <span
        aria-hidden
        data-testid={`live-voice-logo-${vendorId}`}
        className={cn('inline-block bg-current', className)}
        style={{
          maskImage: `url("${mask}")`,
          WebkitMaskImage: `url("${mask}")`,
          maskSize: 'contain',
          WebkitMaskSize: 'contain',
          maskRepeat: 'no-repeat',
          WebkitMaskRepeat: 'no-repeat',
          maskPosition: 'center',
          WebkitMaskPosition: 'center',
        }}
      />
    );
  }
  return <Mic className={className} aria-hidden />;
};

export default LiveVoiceVendorLogo;
