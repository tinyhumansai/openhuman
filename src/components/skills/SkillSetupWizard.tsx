/**
 * Multi-step setup wizard for a single skill.
 * Manages the state machine: start → render form → submit → next/error/complete.
 * Ensures the skill is running (starts it if needed) before starting the setup flow.
 */

import { useState, useEffect, useCallback } from "react";
import { store } from "../../store";
import { skillManager } from "../../lib/skills/manager";
import type { SetupStep, SetupFieldError } from "../../lib/skills/types";
import SetupFormRenderer from "./SetupFormRenderer";

interface SkillSetupWizardProps {
  skillId: string;
  onComplete: () => void;
  onCancel: () => void;
}

type WizardState =
  | { phase: "loading" }
  | { phase: "step"; step: SetupStep; errors?: SetupFieldError[] | null }
  | { phase: "submitting"; step: SetupStep }
  | { phase: "complete"; message?: string }
  | { phase: "error"; message: string };

export default function SkillSetupWizard({
  skillId,
  onComplete,
  onCancel,
}: SkillSetupWizardProps) {
  const [state, setState] = useState<WizardState>({ phase: "loading" });

  // Start the skill (if not running) then start the setup flow on mount
  useEffect(() => {
    let cancelled = false;

    async function initSetup() {
      try {
        console.log("[SkillSetupWizard] initSetup", skillId);
        const manifest = store.getState().skills.skills[skillId]?.manifest;
        console.log("[SkillSetupWizard] manifest", manifest);
        if (!manifest) {
          if (!cancelled) {
            setState({
              phase: "error",
              message: "Skill not found. Try refreshing the page.",
            });
          }
          return;
        }

        if (!skillManager.isSkillRunning(skillId)) {
          console.log("[SkillSetupWizard] starting skill", skillId);
          await skillManager.startSkill(manifest);
          console.log("[SkillSetupWizard] skill started", skillId);
        }

        if (cancelled) return;

        if (!skillManager.isSkillRunning(skillId)) {
          console.log("[SkillSetupWizard] skill not running", skillId);
          const status = skillManager.getSkillStatus(skillId);
          console.log("[SkillSetupWizard] status", status);
          const errMsg =
            status === "error"
              ? store.getState().skills.skills[skillId]?.error ?? "Skill failed to start"
              : "Skill failed to start. Check the console for errors.";
          throw new Error(errMsg);
        }

        console.log("[SkillSetupWizard] starting setup", skillId);
        const firstStep = await skillManager.startSetup(skillId);
        console.log("[SkillSetupWizard] setup started", skillId);
        if (!cancelled) {
          setState({ phase: "step", step: firstStep });
        }
      } catch (err) {
        if (!cancelled) {
          const msg = err instanceof Error ? err.message : String(err);
          setState({ phase: "error", message: msg });
        }
      }
    }

    initSetup();

    return () => {
      cancelled = true;
    };
  }, [skillId]);

  const handleSubmit = useCallback(
    async (values: Record<string, unknown>) => {
      if (state.phase !== "step") return;

      const currentStep = state.step;
      setState({ phase: "submitting", step: currentStep });

      try {
        const result = await skillManager.submitSetup(
          skillId,
          currentStep.id,
          values,
        );

        switch (result.status) {
          case "next":
            if (result.nextStep) {
              setState({ phase: "step", step: result.nextStep });
            }
            break;
          case "error":
            setState({
              phase: "step",
              step: currentStep,
              errors: result.errors,
            });
            break;
          case "complete":
            setState({
              phase: "complete",
              message: result.message ?? "Setup complete!",
            });
            break;
        }
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        setState({ phase: "error", message: msg });
      }
    },
    [state, skillId],
  );

  const handleCancel = useCallback(async () => {
    try {
      await skillManager.cancelSetup(skillId);
    } catch {
      // Ignore cancel errors
    }
    onCancel();
  }, [skillId, onCancel]);

  // Render based on current wizard state
  switch (state.phase) {
    case "loading":
      return (
        <div className="flex items-center justify-center py-12">
          <svg
            className="animate-spin h-6 w-6 text-primary-500"
            xmlns="http://www.w3.org/2000/svg"
            fill="none"
            viewBox="0 0 24 24"
          >
            <circle
              className="opacity-25"
              cx="12"
              cy="12"
              r="10"
              stroke="currentColor"
              strokeWidth="4"
            />
            <path
              className="opacity-75"
              fill="currentColor"
              d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z"
            />
          </svg>
          <span className="ml-3 text-sm text-stone-400">
            Starting setup...
          </span>
        </div>
      );

    case "step":
      return (
        <SetupFormRenderer
          step={state.step}
          errors={state.errors}
          loading={false}
          onSubmit={handleSubmit}
          onCancel={handleCancel}
        />
      );

    case "submitting":
      return (
        <SetupFormRenderer
          step={state.step}
          loading={true}
          onSubmit={() => { }}
          onCancel={() => { }}
        />
      );

    case "complete":
      return (
        <div className="text-center py-8">
          <div className="w-12 h-12 mx-auto mb-4 rounded-full bg-sage-500/20 flex items-center justify-center">
            <svg
              className="w-6 h-6 text-sage-500"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M5 13l4 4L19 7"
              />
            </svg>
          </div>
          <h3 className="text-lg font-semibold text-white mb-2">
            Connected!
          </h3>
          {state.message && (
            <p className="text-sm text-stone-400 mb-6">{state.message}</p>
          )}
          <button
            onClick={onComplete}
            className="px-6 py-2.5 text-sm font-medium text-white bg-primary-500 rounded-xl hover:bg-primary-600 transition-colors"
          >
            Done
          </button>
        </div>
      );

    case "error":
      return (
        <div className="text-center py-8">
          <div className="w-12 h-12 mx-auto mb-4 rounded-full bg-coral-500/20 flex items-center justify-center">
            <svg
              className="w-6 h-6 text-coral-500"
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
          </div>
          <h3 className="text-lg font-semibold text-white mb-2">
            Setup Failed
          </h3>
          <p className="text-sm text-stone-400 mb-6">{state.message}</p>
          <div className="flex space-x-3 justify-center">
            <button
              onClick={handleCancel}
              className="px-6 py-2.5 text-sm font-medium text-stone-400 bg-stone-800/50 border border-stone-700 rounded-xl hover:bg-stone-800 transition-colors"
            >
              Close
            </button>
          </div>
        </div>
      );
  }
}
