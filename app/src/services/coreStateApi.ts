import type { User } from '../types/api';
import type { TeamInvite, TeamMember, TeamWithRole } from '../types/team';
import type { CoreCredentialKind } from '../utils/localSession';
import type { LocalAiStatus } from '../utils/tauriCommands/localAi';
import type { ServiceStatus } from '../utils/tauriCommands/service';
import { callCoreRpc } from './coreRpcClient';
import { fetchCurrentUser } from './session/sessionOwner';

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
    /**
     * Which credential the core holds: `session` (TinyHumans app session),
     * `api-key` (TinyHumans API key) or `local` (offline local profile, no
     * hosted account). Absent when signed out or on older cores.
     */
    credential?: CoreCredentialKind | null;
  };
  sessionToken: string | null;
  /**
   * The live user: the session owner's `/auth/me` answer on the desktop, else
   * the payload the core was handed at login. Older cores also populate it
   * themselves.
   */
  currentUser?: User | null;
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
   * Seconds since the session owner last got a successful `/auth/me` answer
   * this process. Absent when it never has — the stored snapshot then came
   * off disk and its real age is unknown, which is a different statement from
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
      // so match `crates/openhuman-core/src/platform/health/core.rs` — not `string | undefined`.
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
  const result: AppStateSnapshotResult = { ...response.result };
  // The core reports the credential it holds and the user payload it was
  // handed at login (`auth.user`); the *live* current user comes from the
  // session owner's `/auth/me` cache — the Tauri shell's on the desktop, a
  // small in-page one in the browser build and cloud mode — so this
  // poll-frequency call stays cheap.
  if (result.auth?.isAuthenticated) {
    try {
      const current = await fetchCurrentUser(false);
      if (current.user) {
        // A logout, or an A→B login, can land while the session owner's
        // `/auth/me` request above is in flight: `current.user` would then
        // belong to a different identity than the `auth`/`sessionToken`
        // already captured from `app_state_snapshot`. `CoreStateProvider`
        // scopes identity off `auth.userId`, so merging here could pair A's
        // auth/token with B's live user (or A's after a logout). Re-read the
        // active credential and retry the whole snapshot rather than merge a
        // stale pairing (#6318).
        const latestAuth = await callCoreRpc<{
          result: { isAuthenticated: boolean; userId: string | null };
        }>({ method: 'openhuman.auth_get_state' });
        if (
          latestAuth.result.isAuthenticated !== result.auth.isAuthenticated ||
          latestAuth.result.userId !== result.auth.userId
        ) {
          return fetchCoreAppSnapshot();
        }
        result.currentUser = current.user as User;
        result.currentUserStale = current.stale;
        result.currentUserStaleSeconds = current.staleSeconds ?? undefined;
      }
    } catch (error) {
      // A `REJECTED:` here means the owner already cleared the credential and
      // emitted `auth://expired`; CoreStateProvider reacts to that event. Any
      // other failure leaves the stored payload in place.
      console.debug('[core-state] current user unavailable from the session owner:', error);
    }
  }
  if (!result.currentUser && result.auth?.user) {
    result.currentUser = result.auth.user as User;
    result.currentUserStale = true;
  }
  return result;
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
