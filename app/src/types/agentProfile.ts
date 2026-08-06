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
  /** Give this profile its own memory subtree instead of the shared one. Default false. */
  dedicatedMemory?: boolean;
  /** Give this profile its own working directory under action_dir. Default false. */
  dedicatedWorkspace?: boolean;
  /**
   * Read-only, resolved by the core on read (never sent on upsert): absolute
   * path of `personalities/<id>/SOUL.md` when it exists.
   */
  soulMdFile?: string;
  /**
   * Read-only, resolved by the core on read (never sent on upsert): absolute
   * path of the dedicated workspace directory when `dedicatedWorkspace` is set.
   */
  workspaceDir?: string;
  /**
   * Read-only, resolved by the core on read (never sent on upsert): absolute
   * path of the profile's private `skills/` directory when it exists on disk.
   * SKILL.md workflows placed there are scoped to this profile only.
   */
  skillsDir?: string;
}

export interface AgentProfilesResponse {
  profiles: AgentProfile[];
  activeProfileId: string;
}
