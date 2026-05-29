export interface AgentProfile {
  id: string;
  name: string;
  description: string;
  agentId: string;
  modelOverride?: string | null;
  temperature?: number | null;
  systemPromptSuffix?: string | null;
  allowedTools?: string[] | null;
  builtIn: boolean;
  avatarUrl?: string | null;
  voiceId?: string | null;
  soulMd?: string | null;
  soulMdPath?: string | null;
  composioIntegrations?: string[] | null;
  memoryDirSuffix?: string | null;
  isMaster?: boolean | null;
  sortOrder?: number | null;
}

export interface AgentProfilesResponse {
  profiles: AgentProfile[];
  activeProfileId: string;
}
