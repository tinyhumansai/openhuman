import { apiClient } from '../apiClient';

/**
 * Response shape from GET /auth/me when coreToken is included.
 * The backend adds `coreToken` (select: false on User model) only in this
 * endpoint so the desktop can authenticate against the user's core instance.
 */
export interface AuthMeWithCoreToken {
  coreToken: string;
}

export type DeploymentStatus =
  | 'pending'
  | 'provisioning'
  | 'deploying'
  | 'starting'
  | 'active'
  | 'unhealthy'
  | 'terminating'
  | 'terminated'
  | 'failed';

export interface DeploymentInstance {
  deploymentId: string;
  status: DeploymentStatus;
  url: string | null; // the RPC URL (https://.../rpc)
  healthUrl: string | null;
  region: string;
  imageTag: string;
  createdAt: string;
  activatedAt: string | null;
  failureReason: string | null;
}

export interface ProvisionParams {
  awsAccessKeyId: string;
  awsSecretAccessKey: string;
  awsRegion: string;
  imageTag?: string;
  domain?: string;
}

export interface ProvisionResponse {
  deploymentId: string;
  status: 'pending';
  estimatedReadySeconds: number;
}

export interface HealthCheckResponse {
  instanceReachable: boolean;
  instanceStatus: 'ok' | 'error' | 'unreachable';
  latencyMs: number;
  checkedAt: string;
}

export const deploymentsApi = {
  /**
   * Fetch the user's coreToken from the backend.
   * Returns null if the backend does not yet include coreToken (graceful degradation).
   */
  getCoreToken: async (): Promise<string | null> => {
    try {
      const res = await apiClient.get<{ success: boolean; data: AuthMeWithCoreToken }>(
        '/auth/me/core-token'
      );
      return res.data?.coreToken ?? null;
    } catch {
      return null;
    }
  },


  provision: (params: ProvisionParams) =>
    apiClient.post<{ success: boolean; data: ProvisionResponse }>('/deployments/provision', params),

  getStatus: () =>
    apiClient.get<{ success: boolean; data: DeploymentInstance | null }>('/deployments/status'),

  getHealth: () =>
    apiClient.get<{ success: boolean; data: HealthCheckResponse }>('/deployments/health'),

  terminate: (creds?: { awsAccessKeyId: string; awsSecretAccessKey: string }) =>
    apiClient.post<{ success: boolean; data: { deploymentId: string; status: string } }>(
      '/deployments/terminate',
      creds ?? {}
    ),
};
