/**
 * Path resolution for skills directories.
 *
 * With the V8 runtime, paths are primarily resolved by the Rust backend.
 * These helpers provide client-side path computation for UI display and
 * manifest file resolution.
 */

import { IS_DEV } from "../../utils/config";

/**
 * Get the root directory for discovering skills.
 * Skills are installed at runtime by the Rust registry service.
 * Client-side path helpers are informational only.
 */
export function getSkillsBaseDir(): string {
  if (IS_DEV) {
    return "skills/installed";
  }
  return "skills/installed";
}

/**
 * Get the module path for a skill given its ID.
 * For V8 skills, this is the skill ID itself.
 */
export function getSkillModulePath(skillId: string): string {
  return skillId;
}

/**
 * Get the manifest path for a skill.
 */
export function getSkillManifestPath(skillId: string): string {
  return `skills/installed/${skillId}/manifest.json`;
}
