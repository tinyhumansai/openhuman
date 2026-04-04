/**
 * Card-based selector for skill authentication modes.
 * Displays available auth modes (managed, self_hosted, text) as clickable cards.
 * For managed mode, clicking immediately triggers the OAuth flow.
 */

import type { ReactElement } from "react";
import type { AuthMode } from "../../lib/skills/types.ts";

interface AuthModeSelectorProps {
  modes: AuthMode[];
  onSelect: (mode: AuthMode) => void;
  disabled?: boolean;
}

const MODE_ICONS: Record<string, (props: { className: string }) => ReactElement> = {
  managed: ({ className }) => (
    <svg className={className} fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={1.5}
        d="M9 12.75L11.25 15 15 9.75m-3-7.036A11.959 11.959 0 013.598 6 11.99 11.99 0 003 9.749c0 5.592 3.824 10.29 9 11.623 5.176-1.332 9-6.03 9-11.622 0-1.31-.21-2.571-.598-3.751h-.152c-3.196 0-6.1-1.248-8.25-3.285z"
      />
    </svg>
  ),
  self_hosted: ({ className }) => (
    <svg className={className} fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={1.5}
        d="M5.25 14.25h13.5m-13.5 0a3 3 0 01-3-3m3 3a3 3 0 100 6h13.5a3 3 0 100-6m-16.5-3a3 3 0 013-3h13.5a3 3 0 013 3m-19.5 0a4.5 4.5 0 01.9-2.7L5.737 5.1a3.375 3.375 0 012.7-1.35h7.126c1.062 0 2.062.5 2.7 1.35l2.587 3.45a4.5 4.5 0 01.9 2.7m0 0a3 3 0 01-3 3m0 3h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008zm-3 6h.008v.008h-.008v-.008zm0-6h.008v.008h-.008v-.008z"
      />
    </svg>
  ),
  text: ({ className }) => (
    <svg className={className} fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={1.5}
        d="M19.5 14.25v-2.625a3.375 3.375 0 00-3.375-3.375h-1.5A1.125 1.125 0 0113.5 7.125v-1.5a3.375 3.375 0 00-3.375-3.375H8.25m0 12.75h7.5m-7.5 3H12M10.5 2.25H5.625c-.621 0-1.125.504-1.125 1.125v17.25c0 .621.504 1.125 1.125 1.125h12.75c.621 0 1.125-.504 1.125-1.125V11.25a9 9 0 00-9-9z"
      />
    </svg>
  ),
};

const DEFAULT_LABELS: Record<string, string> = {
  managed: "OpenHuman Managed",
  self_hosted: "Self-hosted",
  text: "Credential Text",
};

const DEFAULT_DESCRIPTIONS: Record<string, string> = {
  managed: "One-click setup through OpenHuman",
  self_hosted: "Enter your own API credentials",
  text: "Paste credential content directly",
};

function formatProviderName(provider: string): string {
  const names: Record<string, string> = {
    notion: "Notion",
    google: "Google",
    github: "GitHub",
    slack: "Slack",
    discord: "Discord",
    twitter: "Twitter",
    linear: "Linear",
    gitlab: "GitLab",
  };
  return names[provider] ?? provider.charAt(0).toUpperCase() + provider.slice(1);
}

export default function AuthModeSelector({
  modes,
  onSelect,
  disabled,
}: AuthModeSelectorProps) {
  return (
    <div className="space-y-3">
      <div className="text-center mb-4">
        <h3 className="text-lg font-semibold text-stone-900">
          Choose how to connect
        </h3>
        <p className="text-sm text-stone-500 mt-1">
          Select an authentication method
        </p>
      </div>

      {modes.map((mode) => {
        const IconComponent = MODE_ICONS[mode.type] ?? MODE_ICONS.self_hosted;
        const label =
          mode.type === "managed" && mode.provider
            ? mode.label ?? `Connect with ${formatProviderName(mode.provider)}`
            : mode.label ?? DEFAULT_LABELS[mode.type] ?? mode.type;
        const description =
          mode.description ?? DEFAULT_DESCRIPTIONS[mode.type] ?? "";

        return (
          <button
            key={mode.type}
            type="button"
            onClick={() => onSelect(mode)}
            disabled={disabled}
            className="w-full flex items-center gap-4 p-4 bg-stone-50 border border-stone-200 rounded-xl hover:bg-stone-100 hover:border-stone-300 transition-colors text-left disabled:opacity-50 disabled:cursor-not-allowed group"
          >
            <div className="flex-shrink-0 w-10 h-10 rounded-lg bg-white border border-stone-200 flex items-center justify-center group-hover:border-primary-300 transition-colors">
              <IconComponent className="w-5 h-5 text-stone-500 group-hover:text-primary-500 transition-colors" />
            </div>
            <div className="flex-1 min-w-0">
              <div className="text-sm font-medium text-stone-900">
                {label}
              </div>
              {description && (
                <div className="text-xs text-stone-500 mt-0.5">
                  {description}
                </div>
              )}
            </div>
            <svg
              className="w-4 h-4 text-stone-400 group-hover:text-stone-600 transition-colors flex-shrink-0"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M9 5l7 7-7 7"
              />
            </svg>
          </button>
        );
      })}
    </div>
  );
}
