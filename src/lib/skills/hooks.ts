/**
 * React hooks for consuming skill state from Redux.
 */

import { useMemo } from "react";
import { useAppSelector } from "../../store/hooks";
import type {
  SkillConnectionStatus,
  SkillHostConnectionState,
} from "./types";

/**
 * Derive a unified connection status from the skill's lifecycle status
 * and its self-reported connection/auth state.
 */
function deriveConnectionStatus(
  lifecycleStatus: string | undefined,
  setupComplete: boolean | undefined,
  skillState: Record<string, unknown> | undefined,
): SkillConnectionStatus {
  // Skill not registered or not started
  if (!lifecycleStatus || lifecycleStatus === "installed") {
    return "offline";
  }

  // Process-level errors (failed to spawn, etc.)
  if (lifecycleStatus === "error") {
    return "error";
  }

  // Setup required
  if (
    lifecycleStatus === "setup_required" ||
    lifecycleStatus === "setup_in_progress"
  ) {
    return "setup_required";
  }

  // Still starting up
  if (lifecycleStatus === "starting") {
    return "connecting";
  }

  // Process is running or ready — use the skill's self-reported state
  const hostState = skillState as SkillHostConnectionState | undefined;
  if (!hostState) {
    // No state pushed yet. Skills that don't maintain an external connection
    // (e.g. cron-based skills) may never push host state. If setup is complete
    // and the lifecycle says "ready", treat it as connected.
    if (setupComplete && lifecycleStatus === "ready") {
      return "connected";
    }
    return "connecting";
  }

  const connStatus = hostState.connection_status;
  const authStatus = hostState.auth_status;

  // Check for errors first
  if (connStatus === "error" || authStatus === "error") {
    return "error";
  }

  // Fully connected and authenticated
  if (connStatus === "connected" && authStatus === "authenticated") {
    return "connected";
  }

  // Connecting or authenticating
  if (connStatus === "connecting" || authStatus === "authenticating") {
    return "connecting";
  }

  // Connected but not authenticated
  if (connStatus === "connected" && authStatus === "not_authenticated") {
    return "not_authenticated";
  }

  // Disconnected from service
  if (connStatus === "disconnected") {
    // If setup is complete but we're disconnected, it might be a reconnecting state
    if (setupComplete) {
      return "disconnected";
    }
    return "setup_required";
  }

  // Fallback
  return "connecting";
}

/**
 * Returns the unified connection status for a skill.
 *
 * Combines the skill's lifecycle status (process running, setup needed, etc.)
 * with its self-reported connection/auth state (pushed via state/set reverse RPC).
 */
export function useSkillConnectionStatus(
  skillId: string,
): SkillConnectionStatus {
  const skill = useAppSelector((state) => state.skills.skills[skillId]);
  const skillState = useAppSelector(
    (state) => state.skills.skillStates[skillId],
  );

  return useMemo(
    () =>
      deriveConnectionStatus(skill?.status, skill?.setupComplete, skillState),
    [skill?.status, skill?.setupComplete, skillState],
  );
}

/**
 * Returns the raw skill-pushed state (from reverse RPC state/set).
 */
export function useSkillState<T = Record<string, unknown>>(
  skillId: string,
): T | undefined {
  return useAppSelector(
    (state) => state.skills.skillStates[skillId] as T | undefined,
  );
}

/**
 * Returns connection status info including error messages.
 */
export function useSkillConnectionInfo(skillId: string): {
  status: SkillConnectionStatus;
  error?: string | null;
  isInitialized: boolean;
} {
  const skill = useAppSelector((state) => state.skills.skills[skillId]);
  const skillState = useAppSelector(
    (state) => state.skills.skillStates[skillId],
  );

  return useMemo(() => {
    const status = deriveConnectionStatus(
      skill?.status,
      skill?.setupComplete,
      skillState,
    );
    const hostState = skillState as SkillHostConnectionState | undefined;

    let error: string | null | undefined;
    if (status === "error") {
      error =
        hostState?.connection_error ??
        hostState?.auth_error ??
        skill?.error ??
        null;
    }

    return {
      status,
      error,
      isInitialized: !!hostState?.is_initialized,
    };
  }, [skill?.status, skill?.setupComplete, skill?.error, skillState]);
}

