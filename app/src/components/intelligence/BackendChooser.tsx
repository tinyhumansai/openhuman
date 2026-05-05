import { useState } from 'react';

import type { Backend } from '../../lib/intelligence/settingsApi';

interface BackendChooserProps {
  /** Currently selected backend. */
  value: Backend;
  /** Called when the user clicks a different card. */
  onChange: (next: Backend) => void;
  /** Optional cloud-cost estimate. Mock value until cost-tracker hook lands. */
  costEstimate?: string;
  /** Disabled while a backend switch is in flight. */
  busy?: boolean;
}

/**
 * Two large cards — Cloud (default, recommended) vs Local (advanced).
 *
 * Visual style intentionally matches the rest of the Intelligence page:
 * `bg-white` + `border-stone-200` + `rounded-2xl`, primary blue for the
 * selected accent. The inline tokens from the brief
 * (paper, hairline, ocean) map onto the existing stone/primary scale —
 * we keep the existing scale to avoid forking the design system.
 */
export default function BackendChooser({
  value,
  onChange,
  costEstimate = '$0.42 / mo est.',
  busy = false,
}: BackendChooserProps) {
  const [hoveredCloud, setHoveredCloud] = useState(false);

  const cardBase =
    'flex-1 min-h-[160px] px-6 py-5 rounded-2xl text-left transition-all duration-150 disabled:opacity-50 disabled:cursor-not-allowed';

  return (
    <div className="flex gap-4 flex-col sm:flex-row" role="radiogroup" aria-label="AI backend">
      {/* Cloud */}
      <button
        type="button"
        role="radio"
        aria-checked={value === 'cloud'}
        disabled={busy}
        onClick={() => onChange('cloud')}
        onMouseEnter={() => setHoveredCloud(true)}
        onMouseLeave={() => setHoveredCloud(false)}
        onFocus={() => setHoveredCloud(true)}
        onBlur={() => setHoveredCloud(false)}
        className={`${cardBase} border-2 ${
          value === 'cloud'
            ? 'border-primary-500 bg-white shadow-soft'
            : 'border-stone-200 bg-stone-50 hover:bg-white hover:border-stone-300'
        }`}>
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-2">
            <RadioDot active={value === 'cloud'} />
            <span className="text-sm font-semibold text-stone-900">Cloud</span>
            <span className="text-[10px] uppercase tracking-wider px-1.5 py-0.5 rounded-full bg-primary-50 text-primary-700 border border-primary-100">
              Recommended
            </span>
          </div>
        </div>
        <p className="text-xs text-stone-600 leading-relaxed mb-3">
          Runs on OpenHuman servers. Costs credits. No local CPU.
        </p>
        <div className="font-mono text-[11px] text-stone-500">{costEstimate}</div>
        {/* Privacy reassurance — appears on hover/focus of the Cloud card. */}
        <div
          className={`mt-3 text-[11px] text-stone-500 leading-snug transition-opacity ${
            hoveredCloud ? 'opacity-100' : 'opacity-0'
          }`}
          aria-live="polite">
          Your data still stays local. bge-m3 embedder runs on your machine regardless.
        </div>
      </button>

      {/* Local */}
      <button
        type="button"
        role="radio"
        aria-checked={value === 'local'}
        disabled={busy}
        onClick={() => onChange('local')}
        className={`${cardBase} border-2 ${
          value === 'local'
            ? 'border-primary-500 bg-white shadow-soft'
            : 'border-stone-200 bg-stone-50 hover:bg-white hover:border-stone-300'
        }`}>
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-2">
            <RadioDot active={value === 'local'} />
            <span className="text-sm font-semibold text-stone-900">Local</span>
            <span className="text-[10px] uppercase tracking-wider px-1.5 py-0.5 rounded-full bg-stone-100 text-stone-600 border border-stone-200">
              Advanced
            </span>
          </div>
        </div>
        <p className="text-xs text-stone-600 leading-relaxed mb-3">
          Runs on your machine. Free. Uses your CPU and battery.
        </p>
        <div className="flex items-center gap-1.5 text-[11px] text-amber-700">
          <svg
            className="w-3 h-3"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth={2}>
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              d="M12 9v3.75m0 3.75h.008v.008H12v-.008zM21 12a9 9 0 11-18 0 9 9 0 0118 0z"
            />
          </svg>
          <span>≥8 GB RAM recommended</span>
        </div>
      </button>
    </div>
  );
}

function RadioDot({ active }: { active: boolean }) {
  return (
    <span
      aria-hidden
      className={`w-3.5 h-3.5 rounded-full border-2 flex items-center justify-center ${
        active ? 'border-primary-500' : 'border-stone-300'
      }`}>
      <span
        className={`w-1.5 h-1.5 rounded-full ${active ? 'bg-primary-500' : 'bg-transparent'}`}
      />
    </span>
  );
}
