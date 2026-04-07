/**
 * Skill runtime commands.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { isTauri } from './common';

export type IntegrationStatus = 'Available' | 'Active' | 'ComingSoon';
export type IntegrationCategory =
  | 'Chat'
  | 'AiModel'
  | 'Productivity'
  | 'MusicAudio'
  | 'SmartHome'
  | 'ToolsAutomation'
  | 'MediaCreative'
  | 'Social'
  | 'Platform';

export interface IntegrationInfo {
  name: string;
  description: string;
  category: IntegrationCategory;
  status: IntegrationStatus;
  setup_hints: string[];
}

export interface SkillSnapshot {
  skill_id: string;
  name: string;
  status: unknown;
  tools: Array<{ name: string; description: string; input_schema?: unknown }>;
  error?: string | null;
  state?: Record<string, unknown>;
}

export interface RuntimeDiscoveredSkill {
  id: string;
  name: string;
  runtime?: string;
  entry?: string;
  autoStart?: boolean;
  version?: string;
  ignoreInProduction?: boolean;
  description?: string;
  platforms?: string[];
  tickInterval?: number | null;
  setup?: {
    required?: boolean;
    label?: string;
    oauth?: { provider: string; scopes: string[]; apiBaseUrl: string };
  };
}

export interface RuntimeSkillOption {
  name: string;
  type: 'boolean' | 'text' | 'number' | 'select';
  label: string;
  description?: string | null;
  default?: string | number | boolean | null;
  options?: Array<{ label: string; value: string }> | null;
  value?: string | number | boolean | null;
}

export interface RuntimeSkillDataStats {
  exists: boolean;
  path: string;
  total_bytes: number;
  file_count: number;
}

export async function runtimeListSkills(): Promise<SkillSnapshot[]> {
  return await callCoreRpc<SkillSnapshot[]>({ method: 'openhuman.skills_list' });
}

export async function runtimeDiscoverSkills(): Promise<RuntimeDiscoveredSkill[]> {
  return await callCoreRpc<RuntimeDiscoveredSkill[]>({ method: 'openhuman.skills_discover' });
}

export async function runtimeStartSkill(skillId: string): Promise<SkillSnapshot> {
  return await callCoreRpc<SkillSnapshot>({
    method: 'openhuman.skills_start',
    params: { skill_id: skillId },
  });
}

export async function runtimeStopSkill(skillId: string): Promise<void> {
  await callCoreRpc({ method: 'openhuman.skills_stop', params: { skill_id: skillId } });
}

export async function runtimeRpc<T = unknown>(
  skillId: string,
  method: string,
  params: Record<string, unknown> = {}
): Promise<T> {
  return await callCoreRpc<T>({
    method: 'openhuman.skills_rpc',
    params: { skill_id: skillId, method, params },
  });
}

export async function runtimeSkillDataRead(skillId: string, filename: string): Promise<string> {
  const result = await callCoreRpc<{ content: string }>({
    method: 'openhuman.skills_data_read',
    params: { skill_id: skillId, filename },
  });
  return result.content;
}

export async function runtimeSkillDataWrite(
  skillId: string,
  filename: string,
  content: string
): Promise<void> {
  await callCoreRpc({
    method: 'openhuman.skills_data_write',
    params: { skill_id: skillId, filename, content },
  });
}

export async function runtimeSkillDataDir(skillId: string): Promise<string> {
  const result = await callCoreRpc<{ path: string }>({
    method: 'openhuman.skills_data_dir',
    params: { skill_id: skillId },
  });
  return result.path;
}

export async function runtimeListSkillOptions(skillId: string): Promise<RuntimeSkillOption[]> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  const response = await runtimeRpc<{ options?: RuntimeSkillOption[] }>(
    skillId,
    'options/list',
    {}
  );
  return response.options ?? [];
}

export async function runtimeSetSkillOption(
  skillId: string,
  name: string,
  value: unknown
): Promise<void> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  await runtimeRpc(skillId, 'options/set', { name, value });
}

export async function runtimeIsSkillEnabled(skillId: string): Promise<boolean> {
  const result = await callCoreRpc<{ enabled: boolean }>({
    method: 'openhuman.skills_is_enabled',
    params: { skill_id: skillId },
  });
  return result.enabled;
}

export async function runtimeEnableSkill(skillId: string): Promise<void> {
  await callCoreRpc({ method: 'openhuman.skills_enable', params: { skill_id: skillId } });
}

export async function runtimeDisableSkill(skillId: string): Promise<void> {
  await callCoreRpc({ method: 'openhuman.skills_disable', params: { skill_id: skillId } });
}

export async function runtimeSkillDataStats(skillId: string): Promise<RuntimeSkillDataStats> {
  return await callCoreRpc<RuntimeSkillDataStats>({
    method: 'openhuman.skills_data_stats',
    params: { skill_id: skillId },
  });
}
