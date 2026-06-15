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
  /** Composio toolkit slugs this profile can use. null/undefined = all. */
  composioIntegrations?: string[] | null;
  /** Memory-source entry ids this profile recalls from. null/undefined = all. */
  memorySources?: string[] | null;
  /** Whether this profile recalls prior agent conversations. Default true. */
  includeAgentConversations?: boolean;
  /** Skill/workflow ids this profile can list and run. null/undefined = all. */
  allowedSkills?: string[] | null;
  /** MCP server names this profile can reach. null/undefined = all. */
  allowedMcpServers?: string[] | null;
  memoryDirSuffix?: string | null;
  isMaster?: boolean | null;
  sortOrder?: number | null;
}

export interface AgentProfilesResponse {
  profiles: AgentProfile[];
  activeProfileId: string;
}
