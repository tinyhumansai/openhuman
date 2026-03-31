/**
 * Multi-step setup wizard for a single skill.
 * Manages the state machine: start -> render form -> submit -> next/error/complete.
 * For OAuth skills, shows a login button instead of form steps.
 * Ensures the skill is running (starts it if needed) before starting the setup flow.
 */

import { useState, useEffect, useCallback } from "react";
import { useSkillSnapshot } from "../../lib/skills/hooks.ts";
import { skillManager } from "../../lib/skills/manager.ts";
import { listAvailable, setSetupComplete, startSkill } from "../../lib/skills/skillsApi.ts";
import { apiClient } from "../../services/apiClient.ts";
import { openUrl } from "../../utils/openUrl.ts";
import type { SetupStep, SetupFieldError } from "../../lib/skills/types.ts";
import SetupFormRenderer from "./SetupFormRenderer.tsx";
import {IS_DEV} from "../../utils/config.ts";

interface SkillSetupWizardProps {
  skillId: string;
  onComplete: () => void;
  onCancel: () => void;
}

interface OAuthConfig {
  provider: string;
  scopes: string[];
}

type WizardState =
  | { phase: "loading" }
  | { phase: "oauth"; oauth: OAuthConfig }
  | { phase: "oauth_waiting"; oauth: OAuthConfig }
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

  // Watch skill snapshot for OAuth completion via RPC-backed hook
  const snap = useSkillSnapshot(skillId);
  const isConnected = snap?.connection_status === "connected" || snap?.setup_complete === true;

  // When skill state changes to connected during OAuth waiting, mark complete
  useEffect(() => {
    if (
      (state.phase === "oauth" || state.phase === "oauth_waiting") &&
      isConnected
    ) {
      setSetupComplete(skillId, true).catch(() => {});
      // Schedule state update to avoid synchronous setState inside an effect
      setTimeout(() => {
        setState({ phase: "complete", message: "Successfully connected!" });
      }, 0);
    }
  }, [isConnected, state.phase, skillId]);

  // Start the setup flow on mount
  useEffect(() => {
    let cancelled = false;

    async function initSetup() {
      try {
        console.log("[SkillSetupWizard] initSetup", skillId);

        // Find the available skill entry from the registry for OAuth config
        const available = await listAvailable();
        const entry = available.find(e => e.id === skillId);

        if (!entry) {
          if (!cancelled) {
            setState({
              phase: "error",
              message: "Skill not found in registry. Try refreshing the page.",
            });
          }
          return;
        }

        const setup = entry.setup as { required?: boolean; oauth?: OAuthConfig } | null | undefined;

        // If the skill has OAuth config, show OAuth login directly
        if (setup?.oauth) {
          if (!cancelled) {
            setState({
              phase: "oauth",
              oauth: {
                provider: setup.oauth.provider,
                scopes: setup.oauth.scopes,
              },
            });
          }
          return;
        }

        // Non-OAuth skills need the runtime running for setup steps
        try {
          await startSkill(skillId);
          console.log("[SkillSetupWizard] skill started via RPC", skillId);
        } catch (startErr) {
          console.warn("[SkillSetupWizard] runtime start failed:", startErr);
          if (!cancelled) {
            const msg = startErr instanceof Error ? startErr.message : String(startErr);
            setState({ phase: "error", message: msg });
          }
          return;
        }

        if (cancelled) return;

        console.log("[SkillSetupWizard] starting setup", skillId);
        const firstStep = await skillManager.startSetup(skillId);
        console.log("[SkillSetupWizard] setup started", skillId, firstStep);
        if (!cancelled) {
          if (!firstStep) {
            setState({
              phase: "error",
              message: "This skill requires OAuth setup but no setup steps were returned. Try restarting the app.",
            });
          } else {
            setState({ phase: "step", step: firstStep });
          }
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

  const handleOAuthLogin = useCallback(async () => {
    if (state.phase !== "oauth") return;

    const { oauth } = state;

    try {
      const shouldShowJson = IS_DEV ? 'responseType=json&' : ''
      // Call backend to get the real OAuth authorization URL
      const data = await apiClient.get<{ oauthUrl?: string }>(
        `/auth/${oauth.provider}/connect?${shouldShowJson}skillId=${skillId}`,
      );

      if (!data.oauthUrl) {
        console.error("[SkillSetupWizard] Backend did not return oauthUrl:", data);
        setState({ phase: "error", message: "Failed to get OAuth URL from backend." });
        return;
      }

      await openUrl(data.oauthUrl);
      setState({ phase: "oauth_waiting", oauth });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error("[SkillSetupWizard] OAuth connect error:", err);
      setState({ phase: "error", message: `OAuth connection failed: ${msg}` });
    }
  }, [state, skillId]);

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
    if (state.phase !== "oauth" && state.phase !== "oauth_waiting") {
      try {
        await skillManager.cancelSetup(skillId);
      } catch {
        // Ignore cancel errors
      }
    }
    onCancel();
  }, [skillId, onCancel, state.phase]);

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

    case "oauth":
      return (
        <OAuthLoginView
          provider={state.oauth.provider}
          onLogin={handleOAuthLogin}
          onCancel={handleCancel}
          waiting={false}
        />
      );

    case "oauth_waiting":
      return (
        <OAuthLoginView
          provider={state.oauth.provider}
          onLogin={handleOAuthLogin}
          onCancel={handleCancel}
          waiting={true}
        />
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

// ---------------------------------------------------------------------------
// OAuth Login View
// ---------------------------------------------------------------------------

function formatProviderName(provider: string): string {
  const names: Record<string, string> = {
    notion: "Notion",
    google: "Google",
    github: "GitHub",
    slack: "Slack",
    discord: "Discord",
    twitter: "Twitter",
    linear: "Linear",
  };
  return names[provider] ?? provider.charAt(0).toUpperCase() + provider.slice(1);
}

interface OAuthLoginViewProps {
  provider: string;
  onLogin: () => void;
  onCancel: () => void;
  waiting: boolean;
}

function OAuthLoginView({
  provider,
  onLogin,
  onCancel,
  waiting,
}: OAuthLoginViewProps) {
  const providerName = formatProviderName(provider);

  return (
    <div className="py-6">
      {/* Provider icon */}
      <div className="flex justify-center mb-5">
        <div className="w-14 h-14 rounded-2xl bg-stone-800 border border-stone-700 flex items-center justify-center">
          <ProviderIcon provider={provider} />
        </div>
      </div>

      {/* Title and description */}
      <div className="text-center mb-6">
        <h3 className="text-lg font-semibold text-white mb-2">
          Connect to {providerName}
        </h3>
        <p className="text-sm text-stone-400">
          {waiting
            ? "Waiting for authorization. Complete the login in your browser..."
            : `Sign in with your ${providerName} account to connect this skill.`}
        </p>
      </div>

      {/* Login button or waiting state */}
      {waiting ? (
        <div className="flex flex-col items-center gap-4">
          <div className="flex items-center gap-3 px-4 py-3 bg-stone-800/50 border border-stone-700 rounded-xl">
            <svg
              className="animate-spin h-4 w-4 text-primary-400"
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
            <span className="text-sm text-stone-300">
              Waiting for {providerName} authorization...
            </span>
          </div>

          <button
            onClick={onLogin}
            className="text-xs text-primary-400 hover:text-primary-300 transition-colors"
          >
            Open login page again
          </button>
        </div>
      ) : (
        <button
          onClick={onLogin}
          className="w-full py-3 text-sm font-medium text-white bg-primary-500 rounded-xl hover:bg-primary-600 transition-colors flex items-center justify-center gap-2"
        >
          <svg
            className="w-4 h-4"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M10 6H6a2 2 0 00-2 2v10a2 2 0 002 2h10a2 2 0 002-2v-4M14 4h6m0 0v6m0-6L10 14"
            />
          </svg>
          Sign in with {providerName}
        </button>
      )}

      {/* Cancel */}
      <div className="mt-4">
        <button
          onClick={onCancel}
          className="w-full py-2.5 text-sm font-medium text-stone-400 bg-stone-800/50 border border-stone-700 rounded-xl hover:bg-stone-800 transition-colors"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Provider Icons
// ---------------------------------------------------------------------------

function ProviderIcon({ provider }: { provider: string }) {
  switch (provider) {
    case "notion":
      return (
        <svg className="w-7 h-7" viewBox="0 0 100 100" fill="none">
          <path
            d="M6.017 4.313l55.333-4.087c6.797-.583 8.543-.19 12.817 2.917l17.663 12.443c2.913 2.14 3.883 2.723 3.883 5.053v68.243c0 4.277-1.553 6.807-6.99 7.193L24.467 99.967c-4.08.193-6.023-.39-8.16-3.113L3.3 79.94c-2.333-3.113-3.3-5.443-3.3-8.167V11.113c0-3.497 1.553-6.413 6.017-6.8z"
            fill="#fff"
          />
          <path
            fillRule="evenodd"
            clipRule="evenodd"
            d="M61.35.227L6.017 4.313C1.553 4.7 0 7.617 0 11.113v60.66c0 2.723.967 5.053 3.3 8.167l12.993 16.913c2.137 2.723 4.08 3.307 8.16 3.113l64.257-3.89c5.433-.387 6.99-2.917 6.99-7.193V20.64c0-2.21-.873-2.847-3.443-4.733L75.34 3.57l-.027-.02C71.587.527 69.087-.36 61.35.227zM25.723 19.01c-5.167.39-6.337.477-9.277-1.84l-6.44-5.057c-.773-.78-.39-1.75 1.553-1.947l52.893-3.89c4.473-.39 6.797 1.167 8.543 2.527l7.41 5.443c.39.193.97 1.357 0 1.357l-54.88 3.213-.003.003v-.01zm-6.6 73.793V28.883c0-2.53.777-3.697 3.107-3.89L81.44 21.78c2.14-.193 3.107 1.167 3.107 3.693v63.537c0 2.53-1.16 4.667-4.467 4.86L27.45 97.077c-3.113.193-4.337-.97-4.337-4.273h.01zm51.57-62.177c.39 1.75 0 3.5-1.75 3.7l-2.527.48v47.013c-2.14 1.167-4.273 1.75-5.637 1.75-2.53 0-3.303-.78-5.247-3.107l-16.08-25.283v24.477l5.25 1.17s0 3.5-4.857 3.5L28.337 79.4c-.39-.78 0-2.723 1.357-3.11l3.497-.97V40.763l-4.857-.39c-.39-1.75.583-4.277 3.3-4.473L42.63 35.32l16.667 25.477V37.853l-4.473-.58c-.39-2.14 1.163-3.5 3.303-3.697l12.567-.75z"
            fill="#000"
          />
        </svg>
      );
    case "google":
      return (
        <svg className="w-6 h-6" viewBox="0 0 24 24">
          <path
            d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92a5.06 5.06 0 01-2.2 3.32v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.1z"
            fill="#4285F4"
          />
          <path
            d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"
            fill="#34A853"
          />
          <path
            d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z"
            fill="#FBBC05"
          />
          <path
            d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"
            fill="#EA4335"
          />
        </svg>
      );
    case "github":
      return (
        <svg className="w-7 h-7 text-white" fill="currentColor" viewBox="0 0 24 24">
          <path
            fillRule="evenodd"
            clipRule="evenodd"
            d="M12 2C6.477 2 2 6.484 2 12.017c0 4.425 2.865 8.18 6.839 9.504.5.092.682-.217.682-.483 0-.237-.008-.868-.013-1.703-2.782.605-3.369-1.343-3.369-1.343-.454-1.158-1.11-1.466-1.11-1.466-.908-.62.069-.608.069-.608 1.003.07 1.531 1.032 1.531 1.032.892 1.53 2.341 1.088 2.91.832.092-.647.35-1.088.636-1.338-2.22-.253-4.555-1.113-4.555-4.951 0-1.093.39-1.988 1.029-2.688-.103-.253-.446-1.272.098-2.65 0 0 .84-.27 2.75 1.026A9.564 9.564 0 0112 6.844c.85.004 1.705.115 2.504.337 1.909-1.296 2.747-1.027 2.747-1.027.546 1.379.202 2.398.1 2.651.64.7 1.028 1.595 1.028 2.688 0 3.848-2.339 4.695-4.566 4.943.359.309.678.92.678 1.855 0 1.338-.012 2.419-.012 2.747 0 .268.18.58.688.482A10.019 10.019 0 0022 12.017C22 6.484 17.522 2 12 2z"
          />
        </svg>
      );
    default:
      return (
        <svg
          className="w-7 h-7 text-stone-400"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.5}
            d="M13.828 10.172a4 4 0 00-5.656 0l-4 4a4 4 0 105.656 5.656l1.102-1.101m-.758-4.899a4 4 0 005.656 0l4-4a4 4 0 00-5.656-5.656l-1.1 1.1"
          />
        </svg>
      );
  }
}
