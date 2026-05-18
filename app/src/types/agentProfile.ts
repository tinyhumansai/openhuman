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
}

export interface AgentProfilesResponse {
  profiles: AgentProfile[];
  activeProfileId: string;
}
