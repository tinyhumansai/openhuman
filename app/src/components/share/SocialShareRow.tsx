import debugFactory from 'debug';
import { useCallback, useState } from 'react';

import { copyToClipboard, shareOn, type SharePlatform, tryNativeShare } from '../../utils/share';

const log = debugFactory('share:row');

interface SocialShareRowProps {
  /** Share URL (typically the invite or referral link). */
  url: string;
  /** Text blurb to prefill each social intent. */
  message: string;
  /** Dense (icon-only) vs spacious (icon + label) rendering. */
  variant?: 'dense' | 'spacious';
  /** Platforms to render, in the order shown. Defaults to the full set. */
  platforms?: SharePlatform[];
  /** Called after a share intent opens (analytics hook). */
  onShare?: (platform: SharePlatform | 'native' | 'copy') => void;
}

interface PlatformMeta {
  id: SharePlatform;
  label: string;
  bg: string;
  hover: string;
  icon: React.ReactNode;
}

const PLATFORMS: Record<SharePlatform, PlatformMeta> = {
  twitter: {
    id: 'twitter',
    label: 'X',
    bg: 'bg-stone-900 text-white',
    hover: 'hover:bg-black',
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true" className="h-4 w-4 fill-current">
        <path d="M17.53 3H20.5l-6.48 7.4L22 21h-6.03l-4.71-6.16L5.86 21H2.88l6.94-7.93L2 3h6.17l4.27 5.64L17.53 3Zm-1.06 16.2h1.67L7.62 4.7H5.83l10.64 14.5Z" />
      </svg>
    ),
  },
  telegram: {
    id: 'telegram',
    label: 'Telegram',
    bg: 'bg-[#229ED9] text-white',
    hover: 'hover:bg-[#1B8BC0]',
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true" className="h-4 w-4 fill-current">
        <path d="M9.78 18.65l.28-4.23 7.68-6.92c.34-.31-.07-.46-.52-.19L7.74 13.3 3.64 12c-.88-.25-.89-.86.2-1.3l15.97-6.16c.73-.33 1.43.18 1.15 1.3l-2.72 12.81c-.19.91-.74 1.13-1.5.71L12.6 16.3l-1.99 1.93c-.23.23-.42.42-.83.42z" />
      </svg>
    ),
  },
  whatsapp: {
    id: 'whatsapp',
    label: 'WhatsApp',
    bg: 'bg-[#25D366] text-white',
    hover: 'hover:bg-[#1DB954]',
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true" className="h-4 w-4 fill-current">
        <path d="M12.04 2C6.58 2 2.14 6.44 2.14 11.89c0 2.09.64 4.03 1.76 5.64L2 22l4.66-1.83c1.55.85 3.33 1.34 5.22 1.34h.01c5.46 0 9.89-4.44 9.89-9.9 0-2.64-1.03-5.13-2.9-7-1.88-1.87-4.37-2.9-7-2.9zm0 1.88c2.13 0 4.13.83 5.64 2.34 1.51 1.51 2.34 3.51 2.34 5.64 0 4.42-3.59 8.01-8.02 8.01-1.58 0-3.11-.47-4.42-1.36l-.32-.19-2.77 1.09.94-2.72-.21-.34c-.98-1.46-1.5-3.17-1.5-4.94 0-4.42 3.59-8.02 8.02-8.02zm4.64 9.19c-.25-.13-1.48-.73-1.71-.81-.23-.09-.4-.13-.57.13-.17.25-.65.81-.8.98-.15.17-.3.19-.55.06-.25-.13-1.07-.39-2.04-1.25-.75-.67-1.26-1.5-1.41-1.76-.15-.25-.02-.39.11-.52.12-.12.25-.3.38-.46.13-.15.17-.25.25-.42.09-.17.04-.32-.02-.45-.06-.13-.57-1.37-.78-1.88-.2-.49-.42-.43-.57-.43H8.8c-.17 0-.45.06-.68.32-.23.25-.89.87-.89 2.12 0 1.25.91 2.45 1.04 2.62.13.17 1.79 2.74 4.34 3.85.61.26 1.08.42 1.44.54.61.19 1.16.17 1.6.1.49-.07 1.48-.6 1.69-1.18.21-.58.21-1.08.15-1.18-.06-.1-.23-.17-.48-.3z" />
      </svg>
    ),
  },
  linkedin: {
    id: 'linkedin',
    label: 'LinkedIn',
    bg: 'bg-[#0A66C2] text-white',
    hover: 'hover:bg-[#08528B]',
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true" className="h-4 w-4 fill-current">
        <path d="M4.98 3.5a2.5 2.5 0 1 1 .01 5 2.5 2.5 0 0 1-.01-5zM3 9.25h4V21H3V9.25zM9.25 9.25h3.84v1.6h.05c.53-1 1.83-2.05 3.77-2.05 4.03 0 4.78 2.65 4.78 6.1V21h-4v-5.28c0-1.26-.02-2.88-1.75-2.88-1.75 0-2.02 1.36-2.02 2.78V21h-4V9.25z" />
      </svg>
    ),
  },
  facebook: {
    id: 'facebook',
    label: 'Facebook',
    bg: 'bg-[#1877F2] text-white',
    hover: 'hover:bg-[#1463C9]',
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true" className="h-4 w-4 fill-current">
        <path d="M13.5 21v-7.5h2.53l.38-2.93H13.5V8.87c0-.85.24-1.43 1.46-1.43H16.5V4.83c-.27-.04-1.2-.12-2.29-.12-2.27 0-3.82 1.38-3.82 3.91v2.18H8v2.93h2.39V21h3.11z" />
      </svg>
    ),
  },
  reddit: {
    id: 'reddit',
    label: 'Reddit',
    bg: 'bg-[#FF4500] text-white',
    hover: 'hover:bg-[#E03E00]',
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true" className="h-4 w-4 fill-current">
        <path d="M22 12.1a2.11 2.11 0 0 0-3.57-1.52c-1.35-.94-3.16-1.54-5.16-1.61l.98-3.54 2.83.6a1.44 1.44 0 1 0 .15-1.42l-3.52-.75-1.16 4.2c-2.08.04-3.97.65-5.37 1.62a2.11 2.11 0 1 0-2.37 3.44 4.06 4.06 0 0 0-.06.7C4.75 16.43 8.03 19 12.02 19c3.98 0 7.25-2.57 7.25-5.77 0-.24-.02-.47-.06-.7A2.11 2.11 0 0 0 22 12.1zM8.38 13.67a1.18 1.18 0 1 1 0-2.36 1.18 1.18 0 0 1 0 2.36zm7.24 0a1.18 1.18 0 1 1 0-2.36 1.18 1.18 0 0 1 0 2.36zm-.66 3.05c-.74.74-2.15 1.08-3.54 1.08s-2.8-.34-3.54-1.08a.35.35 0 0 1 .5-.5c.59.6 1.78.88 3.04.88s2.45-.29 3.04-.88a.35.35 0 0 1 .5.5z" />
      </svg>
    ),
  },
  email: {
    id: 'email',
    label: 'Email',
    bg: 'bg-stone-200 text-stone-900',
    hover: 'hover:bg-stone-300',
    icon: (
      <svg
        viewBox="0 0 24 24"
        aria-hidden="true"
        className="h-4 w-4"
        fill="none"
        stroke="currentColor"
        strokeWidth="2">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          d="M3 8l8.5 5.5a2 2 0 0 0 2 0L22 8M5 20h14a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2z"
        />
      </svg>
    ),
  },
  sms: {
    id: 'sms',
    label: 'SMS',
    bg: 'bg-sage-500 text-white',
    hover: 'hover:bg-sage-600',
    icon: (
      <svg
        viewBox="0 0 24 24"
        aria-hidden="true"
        className="h-4 w-4"
        fill="none"
        stroke="currentColor"
        strokeWidth="2">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          d="M8 10h.01M12 10h.01M16 10h.01M21 12a8 8 0 0 1-11.7 7.1L3 21l1.9-6.3A8 8 0 1 1 21 12z"
        />
      </svg>
    ),
  },
};

const DEFAULT_PLATFORMS: SharePlatform[] = [
  'twitter',
  'telegram',
  'whatsapp',
  'linkedin',
  'reddit',
  'email',
];

/**
 * A row of one-tap share buttons plus "Copy" and (when supported) a native
 * share sheet trigger. Drop this in anywhere you want users to spread the
 * word.
 */
export default function SocialShareRow({
  url,
  message,
  variant = 'dense',
  platforms = DEFAULT_PLATFORMS,
  onShare,
}: SocialShareRowProps) {
  const [copied, setCopied] = useState(false);
  const [nativeShareAvailable] = useState(
    () => typeof navigator !== 'undefined' && typeof navigator.share === 'function'
  );

  const handleCopy = useCallback(async () => {
    const ok = await copyToClipboard(url);
    if (ok) {
      setCopied(true);
      onShare?.('copy');
      setTimeout(() => setCopied(false), 1800);
      log('copied link');
    }
  }, [url, onShare]);

  const handleNativeShare = useCallback(async () => {
    const ok = await tryNativeShare({ title: 'OpenHuman', text: message, url });
    if (ok) {
      onShare?.('native');
    } else {
      await handleCopy();
    }
  }, [message, url, onShare, handleCopy]);

  const handlePlatform = useCallback(
    async (platform: SharePlatform) => {
      await shareOn(platform, message, url);
      onShare?.(platform);
    },
    [message, url, onShare]
  );

  const dense = variant === 'dense';

  return (
    <div className="flex flex-wrap items-center gap-2">
      {platforms.map(id => {
        const meta = PLATFORMS[id];
        if (!meta) return null;
        return (
          <button
            key={id}
            type="button"
            onClick={() => void handlePlatform(id)}
            title={`Share on ${meta.label}`}
            aria-label={`Share on ${meta.label}`}
            className={`inline-flex items-center justify-center gap-1.5 rounded-full transition-colors ${meta.bg} ${meta.hover} ${
              dense ? 'h-9 w-9' : 'h-9 px-3 text-xs font-medium'
            }`}>
            {meta.icon}
            {!dense ? <span>{meta.label}</span> : null}
          </button>
        );
      })}

      {nativeShareAvailable ? (
        <button
          type="button"
          onClick={() => void handleNativeShare()}
          title="Share via your device"
          aria-label="Share via your device"
          className={`inline-flex items-center justify-center gap-1.5 rounded-full border border-stone-200 bg-white text-stone-700 transition-colors hover:bg-stone-50 ${
            dense ? 'h-9 w-9' : 'h-9 px-3 text-xs font-medium'
          }`}>
          <svg
            viewBox="0 0 24 24"
            aria-hidden="true"
            className="h-4 w-4"
            fill="none"
            stroke="currentColor"
            strokeWidth="2">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              d="M4 12v7a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-7M16 6l-4-4m0 0L8 6m4-4v13"
            />
          </svg>
          {!dense ? <span>More</span> : null}
        </button>
      ) : null}

      <button
        type="button"
        onClick={() => void handleCopy()}
        className={`inline-flex items-center justify-center gap-1.5 rounded-full transition-colors ${
          copied ? 'bg-sage-500 text-white' : 'bg-stone-900 text-white hover:bg-stone-800'
        } ${dense ? 'h-9 px-3 text-xs font-medium' : 'h-9 px-3 text-xs font-medium'}`}>
        {copied ? (
          <>
            <svg
              viewBox="0 0 24 24"
              aria-hidden="true"
              className="h-4 w-4"
              fill="none"
              stroke="currentColor"
              strokeWidth="2">
              <path strokeLinecap="round" strokeLinejoin="round" d="M5 13l4 4L19 7" />
            </svg>
            <span>Copied!</span>
          </>
        ) : (
          <>
            <svg
              viewBox="0 0 24 24"
              aria-hidden="true"
              className="h-4 w-4"
              fill="none"
              stroke="currentColor"
              strokeWidth="2">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                d="M8 16H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2v2m-6 12h8a2 2 0 0 0 2-2v-8a2 2 0 0 0-2-2h-8a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2z"
              />
            </svg>
            <span>Copy link</span>
          </>
        )}
      </button>
    </div>
  );
}
