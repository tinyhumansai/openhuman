import packageJson from '../../package.json';

const APP_ENV = (import.meta.env.VITE_OPENHUMAN_APP_ENV as string | undefined)
  ?.trim()
  .toLowerCase();

const DEFAULT_BACKEND_URL =
  APP_ENV === 'staging' ? 'https://staging-api.tinyhumans.ai' : 'https://api.tinyhumans.ai';

export const CORE_RPC_URL =
  import.meta.env.VITE_OPENHUMAN_CORE_RPC_URL || 'http://127.0.0.1:7788/rpc';

/** Matches core `OPENHUMAN_TOOL_TIMEOUT_SECS` (default 120s, max 3600s). */
const DEFAULT_TOOL_TIMEOUT_SECS = 120;
const MAX_TOOL_TIMEOUT_SECS = 3600;

function parseToolTimeoutSecs(): number {
  const raw = import.meta.env.VITE_TOOL_TIMEOUT_SECS as string | undefined;
  if (raw === undefined || raw === '') return DEFAULT_TOOL_TIMEOUT_SECS;
  const n = Number(raw);
  if (!Number.isFinite(n) || n <= 0 || n > MAX_TOOL_TIMEOUT_SECS) {
    return DEFAULT_TOOL_TIMEOUT_SECS;
  }
  return Math.round(n);
}

export const TOOL_TIMEOUT_SECS = parseToolTimeoutSecs();

export const IS_DEV = import.meta.env.DEV;

/** Dev only: skip `.skip_onboarding` workspace check and ignore onboarded state so `/onboarding` always shows. Set `VITE_DEV_FORCE_ONBOARDING=true` in `.env.local`. */
export const DEV_FORCE_ONBOARDING =
  import.meta.env.DEV && import.meta.env.VITE_DEV_FORCE_ONBOARDING === 'true';

export const SKILLS_GITHUB_REPO =
  import.meta.env.VITE_SKILLS_GITHUB_REPO || 'tinyhumansai/openhuman-skills';

/** Sentry DSN for error reporting. Leave blank to disable. */
export const SENTRY_DSN = import.meta.env.VITE_SENTRY_DSN as string | undefined;

/** Backend API URL (web fallback when core RPC is unavailable). */
export const BACKEND_URL =
  (import.meta.env.VITE_BACKEND_URL as string | undefined)?.trim() || DEFAULT_BACKEND_URL;

/** Telegram bot username used for managed DM linking when backend does not return a launch URL. */
export const TELEGRAM_BOT_USERNAME =
  (import.meta.env.VITE_TELEGRAM_BOT_USERNAME as string | undefined) || 'openhuman_bot';

/** Dev only: auto-inject JWT token to skip login flow. */
export const DEV_JWT_TOKEN = import.meta.env.DEV
  ? (import.meta.env.VITE_DEV_JWT_TOKEN as string | undefined)
  : undefined;

export const APP_VERSION = packageJson.version;

/**
 * Deployment environment reported to Sentry and other observability surfaces.
 *
 * Derived from `VITE_OPENHUMAN_APP_ENV` (set by CI for production / staging
 * bundles). Falls back to `development` in non-production builds so local
 * debugging never mingles with real user events.
 */
export const APP_ENVIRONMENT: 'production' | 'staging' | 'development' = IS_DEV
  ? 'development'
  : APP_ENV === 'staging'
    ? 'staging'
    : 'production';

/** Short git SHA baked in at build time (`VITE_BUILD_SHA`). Empty locally. */
export const BUILD_SHA = ((import.meta.env.VITE_BUILD_SHA as string | undefined) ?? '')
  .trim()
  .slice(0, 12);

/**
 * Canonical Sentry release identifier: `openhuman@<version>[+<short_sha>]`.
 *
 * Matches the tag the Rust core sidecar reports (see `src/main.rs`) so events
 * from the frontend, the core, and source-map uploads all group under the
 * same release in the Sentry dashboard.
 */
export const SENTRY_RELEASE = BUILD_SHA
  ? `openhuman@${APP_VERSION}+${BUILD_SHA}`
  : `openhuman@${APP_VERSION}`;

/**
 * Minimum **desktop app** semver required for OAuth deep-link completion (`openhuman://oauth/success`).
 *
 * **Build-time embedding:** This value is baked into each shipped installer. Raising the floor for
 * users already on an older build requires them to install a **new** release (or use in-app update
 * when available)—changing CI vars alone does not retrofit existing binaries. For a fleet-wide
 * minimum that can move without a new app build, add a runtime policy endpoint later and consult it
 * here with this constant as fallback only.
 *
 * Set in production builds (e.g. GitHub Actions `vars`). Empty = no gate (default for local dev).
 */
export const MINIMUM_SUPPORTED_APP_VERSION =
  (import.meta.env.VITE_MINIMUM_SUPPORTED_APP_VERSION as string | undefined)?.trim() ?? '';

/** URL for the latest app release download page. Used for OAuth version-gate recovery and crash-recovery prompts. Override via VITE_LATEST_APP_DOWNLOAD_URL for deployment-specific download pages. */
export const LATEST_APP_DOWNLOAD_URL =
  (import.meta.env.VITE_LATEST_APP_DOWNLOAD_URL as string | undefined)?.trim() ||
  'https://github.com/tinyhumansai/openhuman/releases/latest';

/**
 * Public base URL used to build shareable invite/referral links.
 *
 * Friends who click the link from a social post land on a web page that
 * routes them into the app (or the download page). Override with
 * `VITE_SHARE_BASE_URL` to point at a staging/marketing host.
 */
export const SHARE_BASE_URL = (
  (import.meta.env.VITE_SHARE_BASE_URL as string | undefined)?.trim() || 'https://openhuman.ai'
).replace(/\/+$/, '');
