import packageJson from '../../package.json';

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
export const BACKEND_URL = import.meta.env.VITE_BACKEND_URL as string | undefined;

/** Telegram bot username used for managed DM linking when backend does not return a launch URL. */
export const TELEGRAM_BOT_USERNAME =
  (import.meta.env.VITE_TELEGRAM_BOT_USERNAME as string | undefined) || 'openhuman_bot';

/** Dev only: auto-inject JWT token to skip login flow. */
export const DEV_JWT_TOKEN = import.meta.env.DEV
  ? (import.meta.env.VITE_DEV_JWT_TOKEN as string | undefined)
  : undefined;

export const APP_VERSION = packageJson.version;
