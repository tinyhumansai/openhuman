export const CORE_RPC_URL =
  import.meta.env.OPENHUMAN_CORE_RPC_URL ||
  import.meta.env.VITE_OPENHUMAN_CORE_RPC_URL ||
  'http://127.0.0.1:7788/rpc';

export const IS_DEV = import.meta.env.DEV;

/** Dev only: skip `.skip_onboarding` workspace check and ignore onboarded state so `/onboarding` always shows. Set `VITE_DEV_FORCE_ONBOARDING=true` in `.env.local`. */
export const DEV_FORCE_ONBOARDING = true;
// import.meta.env.DEV && import.meta.env.VITE_DEV_FORCE_ONBOARDING === 'true';

export const SKILLS_GITHUB_REPO =
  import.meta.env.VITE_SKILLS_GITHUB_REPO || 'tinyhumansai/openhuman-skills';
