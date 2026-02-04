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
 * In dev, skills are in the submodule `skills/skills/` dir.
 * In production, paths are resolved by the Rust engine.
 */
export function getSkillsBaseDir(): string {
  if (IS_DEV) {
    return "skills";
  }
  return "";
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
  if (IS_DEV) {
    return `skills/skills/${skillId}/manifest.json`;
  }
  return `skills/${skillId}/manifest.json`;
}
