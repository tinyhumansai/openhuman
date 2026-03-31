export const CORE_RPC_URL =
  import.meta.env.VITE_OPENHUMAN_CORE_RPC_URL || 'http://127.0.0.1:7788/rpc';

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

/** Dev only: auto-inject JWT token to skip login flow. */
export const DEV_JWT_TOKEN = import.meta.env.DEV
  ? (import.meta.env.VITE_DEV_JWT_TOKEN as string | undefined)
  : undefined;
