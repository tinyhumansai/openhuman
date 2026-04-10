/**
 * Modal wrapper for the skill setup wizard or management panel.
 * Shows management view for connected skills, setup wizard otherwise.
 * Uses createPortal like the settings modal system.
 */

import { useState, useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { useSkillSnapshot } from "../../lib/skills/hooks";
import SkillSetupWizard from "./SkillSetupWizard";
import SkillManagementPanel from "./SkillManagementPanel";

interface SkillSetupModalProps {
  skillId: string;
  skillName: string;
  skillDescription: string;
  /** Whether this skill has interactive setup hooks. */
  hasSetup?: boolean;
  skillType?: 'openhuman' | 'openclaw';
  onClose: () => void;
}

export default function SkillSetupModal({
  skillId,
  skillName,
  skillDescription,
  hasSetup = true,
  skillType = 'openhuman',
  onClose,
}: SkillSetupModalProps) {
  const modalRef = useRef<HTMLDivElement>(null);
  const snap = useSkillSnapshot(skillId);
  const setupComplete = snap?.setup_complete ?? false;
  // Lock the mode in once we have a concrete snapshot — `useSkillSnapshot`
  // returns `null` on the first render while it fetches, so reading
  // `setup_complete` at mount time would always see `false` and wrongly
  // default an already-connected skill into the setup wizard.
  //
  // We keep `sessionMode` stable after the first resolution so that an
  // OAuth flow that flips `setup_complete` to true mid-wizard does not
  // yank the user out of the wizard's own "complete" success screen.
  // The user can still switch modes explicitly via `setMode` below
  // (e.g. SkillManagementPanel's "Reconfigure" button).
  const [sessionMode, setSessionMode] = useState<"manage" | "setup" | null>(
    () => (!hasSetup ? "manage" : null),
  );

  useEffect(() => {
    if (sessionMode !== null) return;
    // Wait for the first concrete snapshot before deciding.
    if (snap === null) return;
    setSessionMode(setupComplete ? "manage" : "setup");
  }, [sessionMode, snap, setupComplete]);

  const setMode = (m: "manage" | "setup") => setSessionMode(m);
  const mode = sessionMode;

  // Handle escape key
  useEffect(() => {
    const handleEscape = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        onClose();
      }
    };

    document.addEventListener("keydown", handleEscape);
    return () => document.removeEventListener("keydown", handleEscape);
  }, [onClose]);

  // Focus management
  useEffect(() => {
    const previousFocus = document.activeElement as HTMLElement;
    if (modalRef.current) {
      modalRef.current.focus();
    }
    return () => {
      if (previousFocus?.focus) {
        previousFocus.focus();
      }
    };
  }, []);

  const handleBackdropClick = (e: React.MouseEvent) => {
    if (e.target === e.currentTarget) {
      onClose();
    }
  };

  const headerTitle =
    mode === null
      ? skillName
      : mode === "manage"
        ? `Manage ${skillName}`
        : `Connect ${skillName}`;

  const modalContent = (
    <div
      className="fixed inset-0 z-[9999] bg-black/30 backdrop-blur-sm flex items-center justify-center p-4"
      onClick={handleBackdropClick}
      role="dialog"
      aria-modal="true"
      aria-labelledby="skill-setup-title"
    >
      <div
        ref={modalRef}
        className="bg-white border border-stone-200 rounded-3xl shadow-large w-full max-w-[460px] overflow-hidden animate-fade-up focus:outline-none focus:ring-0"
        style={{
          animationDuration: "200ms",
          animationTimingFunction: "cubic-bezier(0.25, 0.46, 0.45, 0.94)",
          animationFillMode: "both",
        }}
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="p-4 border-b border-stone-200">
          <div className="flex items-start justify-between">
            <div className="flex-1 min-w-0 pr-2">
              <div className="flex items-center gap-2">
                <h2
                  id="skill-setup-title"
                  className="text-base font-semibold text-stone-900"
                >
                  {headerTitle}
                </h2>
                <span
                  className={`px-1.5 py-0.5 text-[10px] font-medium rounded-md ${
                    skillType === 'openclaw'
                      ? 'bg-violet-500/15 text-violet-400'
                      : 'bg-sage-500/15 text-sage-400'
                  }`}
                >
                  {skillType}
                </span>
              </div>
              {skillDescription && (
                <p className="text-xs text-stone-400 mt-1.5 line-clamp-2">
                  {skillDescription}
                </p>
              )}
            </div>
            <button
              onClick={onClose}
              className="p-1 text-stone-400 hover:text-stone-900 transition-colors rounded-lg hover:bg-stone-100 flex-shrink-0"
            >
              <svg
                className="w-5 h-5"
                fill="none"
                stroke="currentColor"
                viewBox="0 0 24 24"
              >
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
        <div className="p-4">
          {mode === null ? (
            <div className="flex items-center justify-center py-8 text-sm text-stone-400">
              Loading…
            </div>
          ) : mode === "manage" ? (
            <SkillManagementPanel
              skillId={skillId}
              onClose={onClose}
              onReconfigure={hasSetup ? () => setMode("setup") : undefined}
            />
          ) : (
            <SkillSetupWizard
              skillId={skillId}
              onComplete={onClose}
              onCancel={setupComplete ? () => setMode("manage") : onClose}
            />
          )}
        </div>
      </div>
    </div>
  );

  return createPortal(modalContent, document.body);
}
