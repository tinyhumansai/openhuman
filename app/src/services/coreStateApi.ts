import type { User } from '../types/api';
import type { TeamInvite, TeamMember, TeamWithRole } from '../types/team';
import type { LocalAiStatus } from '../utils/tauriCommands/localAi';
import type { ServiceStatus } from '../utils/tauriCommands/service';
import { callCoreRpc } from './coreRpcClient';

interface OnboardingTasks {
  accessibilityPermissionGranted: boolean;
  localModelConsentGiven: boolean;
  localModelDownloadStarted: boolean;
  enabledTools: string[];
  connectedSources: string[];
  updatedAtMs?: number;
}

export interface KeyringConsentPreference {
  storageMode: string;
  consentedAtMs?: number;
}

export interface KeyringStatus {
  available: boolean;
  failureReason?: string | null;
  activeMode: string;
  backendName: string;
}

interface UpdateCoreLocalStateParams {
  encryptionKey?: string | null;
  onboardingTasks?: OnboardingTasks | null;
  keyringConsent?: KeyringConsentPreference | null;
}

interface AppStateSnapshotResult {
  auth: {
    isAuthenticated: boolean;
    userId: string | null;
    user: unknown | null;
    profileId: string | null;
  };
  sessionToken: string | null;
  currentUser: User | null;
  onboardingCompleted: boolean;
  chatOnboardingCompleted: boolean;
  analyticsEnabled: boolean;
  /**
   * Mirror of `Config::meet.auto_orchestrator_handoff` (#1299). Older
   * core builds may omit the field on the wire — `fetchCoreAppSnapshot`
   * normalises the missing case to `false` before returning so callers
   * never observe `undefined` here.
   */
  localState: {
    encryptionKey?: string | null;
    onboardingTasks?: OnboardingTasks | null;
    keyringConsent?: KeyringConsentPreference | null;
  };
  keyringStatus?: KeyringStatus;
  runtime: { localAi: LocalAiStatus; service: ServiceStatus };
  /**
   * Process + component health, folded into this snapshot (#daemon-poll-fold)
   * so the daemon-health store hydrates from the same poll instead of a second
   * `health_snapshot` poller. Fields are snake_case on the wire (the core type
   * has no camelCase rename). Optional so older cores that omit it degrade
   * gracefully — the daemon store simply isn't refreshed from those.
   */
  health?: RawHealthSnapshot;
  /**
   * `true` when the core recovered a corrupted `config.toml` this session — the
   * on-disk settings were unreadable/unparseable, so the file was renamed to
   * `.corrupted.<ts>` and reset to defaults (#5167). Latched at boot so it stays
   * reported after the file is healed. Optional so older cores that omit it
   * degrade to "no recovery". `CoreStateProvider` raises a one-shot notice.
   */
  configRecovered?: boolean;
  /**
   * `true` when `currentUser` came from the core's stored snapshot because the
   * backend could not be refreshed, so its plan tier, credit balance and
   * feature flags may be out of date (#5930). Optional so older cores that
   * omit it degrade to "not stale".
   */
  currentUserStale?: boolean;
  /**
   * Seconds since the core last got a successful `auth_get_me` answer this
   * process. Absent when it never has — the stored snapshot then came off
   * disk and its real age is unknown, which is a different statement from
   * "zero seconds old".
   */
  currentUserStaleSeconds?: number;
}

/** Raw (snake_case) health payload embedded in the app-state snapshot. */
interface RawHealthSnapshot {
  pid: number;
  updated_at: string;
  uptime_seconds: number;
  components: Record<
    string,
    {
      status: string;
      updated_at: string;
      // Rust serializes absent `Option<String>` as `null` (no skip attribute),
      // so match `src/openhuman/platform/health/core.rs` — not `string | undefined`.
      last_ok?: string | null;
      last_error?: string | null;
      restart_count: number;
    }
  >;
}

/**
 * First-launch `app_state_snapshot` can take 30–40s on M-series Macs while
 * memory tree init, Composio registry warmup, and other boot work compete
 * for the snapshot critical path (#2156). The global `CORE_RPC_TIMEOUT_MS`
 * default of 30s caused users with merely slow-but-alive cores to be parked
 * on the post-login fallback. Use a longer-but-still-bounded budget here so
 * legitimate slow-success completes inline, while real failures still abort
 * within `SNAPSHOT_TIMEOUT_MS` rather than hanging forever.
 */
export const SNAPSHOT_TIMEOUT_MS = 90_000;

export const fetchCoreAppSnapshot = async (): Promise<AppStateSnapshotResult> => {
  const response = await callCoreRpc<{ result: AppStateSnapshotResult }>({
    method: 'openhuman.app_state_snapshot',
    timeoutMs: SNAPSHOT_TIMEOUT_MS,
  });
  // Normalise the optional #1299 field at the API boundary so older core
  // privacy-conservative `false` to callers (e.g. CoreStateProvider).
  return { ...response.result };
};

export const updateCoreLocalState = async (params: UpdateCoreLocalStateParams): Promise<void> => {
  await callCoreRpc({ method: 'openhuman.app_state_update_local_state', params });
};

export const listTeams = async (): Promise<TeamWithRole[]> => {
  const response = await callCoreRpc<{ result: TeamWithRole[] }>({
    method: 'openhuman.team_list_teams',
  });
  return response.result;
};

export const getTeamMembers = async (teamId: string): Promise<TeamMember[]> => {
  const response = await callCoreRpc<{ result: TeamMember[] }>({
    method: 'openhuman.team_list_members',
    params: { teamId },
  });
  return response.result;
};

export const getTeamInvites = async (teamId: string): Promise<TeamInvite[]> => {
  const response = await callCoreRpc<{ result: TeamInvite[] }>({
    method: 'openhuman.team_list_invites',
    params: { teamId },
  });
  return response.result;
};
