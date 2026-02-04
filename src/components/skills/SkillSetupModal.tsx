/**
 * Modal wrapper for the skill setup wizard or management panel.
 * Shows management view for connected skills, setup wizard otherwise.
 * Uses createPortal like the settings modal system.
 */

import { useState, useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { useAppSelector } from "../../store/hooks";
import SkillSetupWizard from "./SkillSetupWizard";
import SkillManagementPanel from "./SkillManagementPanel";

interface SkillSetupModalProps {
  skillId: string;
  skillName: string;
  skillDescription: string;
  /** Whether this skill has interactive setup hooks. */
  hasSetup?: boolean;
  onClose: () => void;
}

export default function SkillSetupModal({
  skillId,
  skillName,
  skillDescription,
  hasSetup = true,
  onClose,
}: SkillSetupModalProps) {
  const modalRef = useRef<HTMLDivElement>(null);
  const setupComplete = useAppSelector(
    (state) => state.skills.skills[skillId]?.setupComplete,
  );
  // Skills without setup hooks always go straight to manage mode.
  const [mode, setMode] = useState<"manage" | "setup">(
    !hasSetup || setupComplete ? "manage" : "setup",
  );

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
    mode === "manage" ? `Manage ${skillName}` : `Connect ${skillName}`;

  const modalContent = (
    <div
      className="fixed inset-0 z-[9999] bg-black/50 backdrop-blur-sm flex items-center justify-center p-4"
      onClick={handleBackdropClick}
      role="dialog"
      aria-modal="true"
      aria-labelledby="skill-setup-title"
    >
      <div
        ref={modalRef}
        className="bg-stone-900 border border-stone-600 rounded-3xl shadow-large w-full max-w-[460px] overflow-hidden animate-fade-up focus:outline-none focus:ring-0"
        style={{
          animationDuration: "200ms",
          animationTimingFunction: "cubic-bezier(0.25, 0.46, 0.45, 0.94)",
          animationFillMode: "both",
        }}
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="p-4 border-b border-stone-700/50">
          <div className="flex items-start justify-between">
            <div className="flex-1 min-w-0 pr-2">
              <h2
                id="skill-setup-title"
                className="text-base font-semibold text-white"
              >
                {headerTitle}
              </h2>
              {skillDescription && (
                <p className="text-xs text-stone-400 mt-1.5 line-clamp-2">
                  {skillDescription}
                </p>
              )}
            </div>
            <button
              onClick={onClose}
              className="p-1 text-stone-400 hover:text-white transition-colors rounded-lg hover:bg-stone-700/50 flex-shrink-0"
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
          {mode === "manage" ? (
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
