// Meeting bots entry point on the Skills "Integrations" section.
//
// Surfaces as a compact, fun banner: clicking opens a modal that wraps
// the backend mascot bot (PR tinyhumansai/backend#773). Joining a
// Google Meet kicks off the Camoufox-driven mascot in the backend,
// which streams the mascot's WebRTC video into the call as an
// anonymous guest. Zoom and Teams are shown as "coming soon" — the
// backend already routes them but returns 400 "not yet supported".

import { useEffect, useState } from 'react';

import {
  joinMeetingViaMascotBot,
  SERVER_OVERLOADED_MESSAGE,
  type MascotJoinMeetingError,
  type MascotMeetPlatform,
} from '../../services/meetCallService';

type Toast = { type: 'success' | 'error' | 'info'; title: string; message?: string };

interface Props {
  onToast?: (toast: Toast) => void;
}

interface PlatformDef {
  platform: MascotMeetPlatform;
  label: string;
  domainHint: string;
  comingSoon?: boolean;
}

const PLATFORMS: PlatformDef[] = [
  { platform: 'gmeet', label: 'Google Meet', domainHint: 'meet.google.com/abc-defg-hij' },
  { platform: 'zoom', label: 'Zoom', domainHint: 'zoom.us/j/…', comingSoon: true },
  {
    platform: 'teams',
    label: 'Microsoft Teams',
    domainHint: 'teams.microsoft.com/…',
    comingSoon: true,
  },
];

function isMascotJoinMeetingError(err: unknown): err is MascotJoinMeetingError {
  return !!err && typeof err === 'object' && 'isCapacityGated' in err && 'message' in err;
}

export default function MeetingBotsCard({ onToast }: Props) {
  const [open, setOpen] = useState(false);

  return (
    <>
      <MeetingBotsBanner onClick={() => setOpen(true)} />
      {open && <MeetingBotsModal onClose={() => setOpen(false)} onToast={onToast} />}
    </>
  );
}

function MeetingBotsBanner({ onClick }: { onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      data-testid="meeting-bots-banner"
      className="group relative w-full overflow-hidden rounded-2xl border border-primary-200/60 bg-gradient-to-br from-primary-50 via-white to-amber-50 p-4 text-left shadow-soft transition hover:-translate-y-0.5 hover:shadow-md focus:outline-none focus-visible:ring-2 focus-visible:ring-primary-400 animate-fade-up">
      {/* Decorative gradient orbs — purely cosmetic, hidden from a11y. */}
      <span
        aria-hidden="true"
        className="pointer-events-none absolute -right-8 -top-8 h-32 w-32 rounded-full bg-primary-300/30 blur-2xl transition group-hover:bg-primary-300/40"
      />
      <span
        aria-hidden="true"
        className="pointer-events-none absolute -bottom-10 -left-6 h-24 w-24 rounded-full bg-amber-300/30 blur-2xl"
      />

      <div className="relative flex items-center gap-3">
        <span
          aria-hidden="true"
          className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-white text-base font-bold text-primary-600 shadow-soft ring-1 ring-primary-200/70">
          {/* Tiny "wave" mark — three dots that animate on hover. */}
          <span className="flex items-end gap-0.5">
            <span className="h-2 w-1 rounded-full bg-primary-500 transition group-hover:h-3" />
            <span className="h-3 w-1 rounded-full bg-primary-500 transition group-hover:h-4" />
            <span className="h-2 w-1 rounded-full bg-primary-500 transition group-hover:h-3" />
          </span>
        </span>

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <h2 className="text-sm font-semibold text-stone-900">
              Send OpenHuman to a meeting
            </h2>
            <span className="rounded-full bg-primary-100 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-primary-700">
              New
            </span>
          </div>
          <p className="mt-0.5 line-clamp-1 text-[11px] leading-relaxed text-stone-600">
            Drop a Google Meet link and OpenHuman joins as a guest, talks, listens, and waves
            back.
          </p>
        </div>

        <span
          aria-hidden="true"
          className="ml-2 hidden text-stone-400 transition group-hover:text-stone-600 sm:inline">
          →
        </span>
      </div>
    </button>
  );
}

interface ModalProps {
  onClose: () => void;
  onToast?: (toast: Toast) => void;
}

function MeetingBotsModal({ onClose, onToast }: ModalProps) {
  const [platform, setPlatform] = useState<MascotMeetPlatform>('gmeet');
  const [meetUrl, setMeetUrl] = useState('');
  const [displayName, setDisplayName] = useState('OpenHuman');
  const [submitting, setSubmitting] = useState(false);
  const [capacityGated, setCapacityGated] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const selected = PLATFORMS.find(p => p.platform === platform) ?? PLATFORMS[0];
  const isComingSoon = !!selected.comingSoon;

  // Esc closes the modal — matches the OpenhumanLinkModal pattern.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose]);

  const handleSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    setCapacityGated(false);
    if (isComingSoon) {
      setError(`${selected.label} support is coming soon.`);
      return;
    }
    setSubmitting(true);
    try {
      await joinMeetingViaMascotBot({ platform, meetUrl, displayName });
      onToast?.({
        type: 'success',
        title: 'OpenHuman is joining the meeting',
        message: 'It should appear as a participant in a few seconds.',
      });
      setMeetUrl('');
      onClose();
    } catch (err) {
      if (isMascotJoinMeetingError(err)) {
        setCapacityGated(err.isCapacityGated);
        const message = err.isCapacityGated ? SERVER_OVERLOADED_MESSAGE : err.message;
        setError(message);
        onToast?.({
          type: 'error',
          title: err.isCapacityGated ? 'OpenHuman is busy' : 'Could not start OpenHuman',
          message,
        });
      } else {
        const message = err instanceof Error ? err.message : 'Failed to start OpenHuman.';
        setError(message);
        onToast?.({ type: 'error', title: 'Could not start OpenHuman', message });
      }
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Send OpenHuman to a meeting"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      onClick={onClose}>
      <div
        className="w-full max-w-md overflow-hidden rounded-2xl bg-white shadow-xl"
        onClick={e => e.stopPropagation()}>
        {/* Header band — same fun gradient as the banner so the modal feels like
            a continuation of the click, not a context switch. */}
        <div className="relative bg-gradient-to-br from-primary-50 via-white to-amber-50 px-5 py-4">
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="absolute right-3 top-3 rounded-full p-1 text-stone-500 hover:bg-white/80 hover:text-stone-800">
            ✕
          </button>
          <h2 className="text-base font-semibold text-stone-900">Send OpenHuman to a meeting</h2>
          <p className="mt-1 text-xs leading-relaxed text-stone-600">
            OpenHuman joins as an anonymous guest, streams its video into the call, and replies
            via the agent.
          </p>
        </div>

        <div className="space-y-4 p-5">
          <div className="flex flex-wrap gap-1.5">
            {PLATFORMS.map(p => {
              const active = p.platform === platform;
              return (
                <button
                  key={p.platform}
                  type="button"
                  onClick={() => {
                    setPlatform(p.platform);
                    setError(null);
                  }}
                  className={`rounded-full px-3 py-1 text-[11px] font-medium transition ${
                    active
                      ? 'bg-primary-500 text-white'
                      : 'bg-stone-100 text-stone-600 hover:bg-stone-200'
                  }`}>
                  {p.label}
                  {p.comingSoon && <span className="ml-1 opacity-70">· soon</span>}
                </button>
              );
            })}
          </div>

          <form onSubmit={handleSubmit} className="space-y-3">
            <label className="block">
              <span className="text-[10px] font-medium uppercase tracking-wide text-stone-500">
                Meeting link
              </span>
              <input
                type="url"
                inputMode="url"
                autoComplete="off"
                spellCheck={false}
                value={meetUrl}
                onChange={e => setMeetUrl(e.target.value)}
                placeholder={selected.domainHint}
                disabled={isComingSoon || submitting}
                autoFocus
                className="mt-1 w-full rounded-xl border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 placeholder:text-stone-400 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100 disabled:cursor-not-allowed disabled:bg-stone-50"
                required
              />
            </label>

            <label className="block">
              <span className="text-[10px] font-medium uppercase tracking-wide text-stone-500">
                Display name
              </span>
              <input
                type="text"
                value={displayName}
                onChange={e => setDisplayName(e.target.value)}
                maxLength={64}
                disabled={isComingSoon || submitting}
                className="mt-1 w-full rounded-xl border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100 disabled:cursor-not-allowed disabled:bg-stone-50"
              />
            </label>

            {error && (
              <div
                role="alert"
                className={`rounded-xl border px-3 py-2 text-xs ${
                  capacityGated
                    ? 'border-amber-200 bg-amber-50 text-amber-800'
                    : 'border-coral-200 bg-coral-50 text-coral-700'
                }`}>
                {error}
              </div>
            )}

            <div className="flex items-center justify-end gap-2 pt-1">
              <button
                type="button"
                onClick={onClose}
                className="rounded-xl px-3 py-2 text-sm font-medium text-stone-600 hover:bg-stone-100">
                Cancel
              </button>
              <button
                type="submit"
                disabled={submitting || isComingSoon || !meetUrl.trim()}
                className="rounded-xl bg-primary-500 px-4 py-2 text-sm font-semibold text-white hover:bg-primary-600 disabled:cursor-not-allowed disabled:bg-stone-200 disabled:text-stone-400">
                {isComingSoon
                  ? `${selected.label} coming soon`
                  : submitting
                    ? 'Starting…'
                    : `Send to ${selected.label}`}
              </button>
            </div>
          </form>
        </div>
      </div>
    </div>
  );
}
