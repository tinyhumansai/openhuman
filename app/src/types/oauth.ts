/**
 * OAuth provider types and interfaces
 */

export type OAuthProvider = 'google' | 'twitter' | 'github' | 'discord';

export interface OAuthProviderConfig {
  id: OAuthProvider;
  name: string;
  icon: React.ComponentType<{ className?: string }>;
  color: string;
  hoverColor: string;
  textColor: string;
  showOnWelcome?: boolean;
}
