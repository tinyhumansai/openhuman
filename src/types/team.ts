export type TeamRole = 'ADMIN' | 'BILLING_MANAGER' | 'MEMBER';
export type TeamPlan = 'FREE' | 'BASIC' | 'PRO';

export interface TeamSubscription {
  plan: TeamPlan;
  hasActiveSubscription: boolean;
  planExpiry?: string;
  stripeCustomerId?: string;
}

export interface TeamUsage {
  dailyTokenLimit: number;
  remainingTokens: number;
  activeSessionCount: number;
  lastTokenResetAt?: string;
}

export interface Team {
  _id: string;
  name: string;
  slug: string;
  createdBy: string;
  isPersonal: boolean;
  maxMembers: number;
  inviteCode?: string;
  subscription: TeamSubscription;
  usage: TeamUsage;
  createdAt: string;
  updatedAt: string;
}

export interface TeamWithRole {
  team: Team;
  role: TeamRole;
}

export interface TeamMember {
  _id: string;
  user: {
    _id: string;
    firstName?: string;
    lastName?: string;
    username?: string;
    telegramId?: number;
  };
  role: TeamRole;
  joinedAt: string;
  invitedBy?: string;
}

export interface TeamInvite {
  _id: string;
  code: string;
  createdBy: string;
  expiresAt: string;
  maxUses: number;
  currentUses: number;
  usageHistory: Array<{ userId: string; usedAt: string }>;
}
