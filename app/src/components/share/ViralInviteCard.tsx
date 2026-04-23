import { useCallback, useMemo, useState } from 'react';

import { copyToClipboard } from '../../utils/share';

interface ViralInviteCardProps {
  /** The user-facing invite or referral code (e.g. "ABC123"). */
  code: string;
  /** Shareable URL for the code (deep-link-friendly). */
  url: string;
  /** Headline copy override. */
  headline?: string;
  /** Subheadline / reward description override. */
  subheadline?: string;
  /** Friendly first name for personalization. */
  firstName?: string;
}

/**
 * Beautiful, high-contrast "hero" card that showcases the user's invite
 * code and link. Designed to be screenshotted — the gradient mesh, star
 * motif, and personalized headline make it feel worth sharing.
 */
export default function ViralInviteCard({
  code,
  url,
  headline,
  subheadline = 'Both of you earn credits when your friend joins.',
  firstName,
}: ViralInviteCardProps) {
  const [copiedTarget, setCopiedTarget] = useState<'code' | 'url' | null>(null);

  const displayHeadline =
    headline ??
    (firstName ? `${firstName}, pass the torch.` : 'Invite your people. Earn together.');

  const copy = useCallback(async (target: 'code' | 'url', text: string) => {
    const ok = await copyToClipboard(text);
    if (ok) {
      setCopiedTarget(target);
      setTimeout(() => setCopiedTarget(null), 1800);
    }
  }, []);

  const shortUrl = useMemo(() => url.replace(/^https?:\/\//, ''), [url]);

  return (
    <div className="relative overflow-hidden rounded-3xl bg-stone-900 p-6 text-white shadow-strong sm:p-8">
      {/* Decorative gradient + soft noise */}
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-0 opacity-80"
        style={{
          background:
            'radial-gradient(circle at 20% 0%, #4A83DD 0%, transparent 45%), radial-gradient(circle at 100% 100%, #9B8AFB 0%, transparent 45%), radial-gradient(circle at 70% 30%, #4DC46F 0%, transparent 40%)',
        }}
      />
      <div
        aria-hidden="true"
        className="pointer-events-none absolute inset-0"
        style={{
          background: 'linear-gradient(180deg, rgba(28,25,23,0.15) 0%, rgba(28,25,23,0.65) 100%)',
        }}
      />

      <div className="relative z-10 space-y-5">
        <div className="flex items-center justify-between">
          <div className="inline-flex items-center gap-2 rounded-full border border-white/20 bg-white/10 px-3 py-1 text-[11px] font-medium uppercase tracking-wider backdrop-blur-sm">
            <span className="h-1.5 w-1.5 rounded-full bg-sage-500 animate-pulse" />
            OpenHuman · Invite
          </div>
          <div className="hidden sm:flex items-center gap-1 text-[11px] text-white/60">
            <svg className="h-3 w-3" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
              <path d="M12 2L15 9L22 9.25L16.5 14L18.5 21L12 17.25L5.5 21L7.5 14L2 9.25L9 9L12 2Z" />
            </svg>
            Limited codes
          </div>
        </div>

        <div className="space-y-2">
          <h2 className="text-2xl font-bold leading-tight tracking-tight sm:text-3xl">
            {displayHeadline}
          </h2>
          <p className="max-w-md text-sm text-white/70">{subheadline}</p>
        </div>

        <div className="grid gap-3 sm:grid-cols-[1fr_auto]">
          <button
            type="button"
            onClick={() => void copy('code', code)}
            className="group flex items-center justify-between rounded-2xl border border-white/15 bg-white/10 px-4 py-3 text-left backdrop-blur-md transition-colors hover:bg-white/15"
            aria-label="Copy invite code">
            <div>
              <div className="text-[10px] font-medium uppercase tracking-widest text-white/60">
                Your code
              </div>
              <div className="font-mono text-xl font-semibold tracking-[0.25em] text-white">
                {code || '—'}
              </div>
            </div>
            <span className="ml-3 rounded-full bg-white/10 px-3 py-1 text-[11px] font-medium">
              {copiedTarget === 'code' ? 'Copied!' : 'Tap to copy'}
            </span>
          </button>
          <button
            type="button"
            onClick={() => void copy('url', url)}
            className="group flex items-center justify-between rounded-2xl border border-white/15 bg-white/10 px-4 py-3 text-left backdrop-blur-md transition-colors hover:bg-white/15 sm:w-64"
            aria-label="Copy invite link">
            <div className="min-w-0">
              <div className="text-[10px] font-medium uppercase tracking-widest text-white/60">
                Share link
              </div>
              <div className="truncate font-mono text-sm text-white/90">{shortUrl}</div>
            </div>
            <span className="ml-3 rounded-full bg-white/10 px-3 py-1 text-[11px] font-medium">
              {copiedTarget === 'url' ? 'Copied!' : 'Copy'}
            </span>
          </button>
        </div>

        <div className="flex items-center gap-2 text-[11px] text-white/60">
          <svg
            className="h-3.5 w-3.5"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            viewBox="0 0 24 24"
            aria-hidden="true">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0z"
            />
          </svg>
          Both you and your friend get credit when they join. No limit on earnings.
        </div>
      </div>
    </div>
  );
}
