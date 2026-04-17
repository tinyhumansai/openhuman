import {
  openhumanLocalAiApplyPreset,
  openhumanLocalAiDownloadAllAssets,
  openhumanLocalAiPresets,
  type PresetsResponse,
} from './tauriCommands';

const MAX_RETRIES = 5;
const RETRY_BASE_DELAY_MS = 250;

const wait = (ms: number) =>
  new Promise<void>(resolve => {
    setTimeout(resolve, ms);
  });

const normalizeSelectedTier = (tier: string | null | undefined): string | null => {
  if (typeof tier !== 'string') return null;
  const normalized = tier.trim().toLowerCase();
  return normalized.length > 0 ? normalized : null;
};

const retryLocalAiCommand = async <T>(
  label: string,
  run: () => Promise<T>,
  logPrefix: string
): Promise<T> => {
  let lastError: unknown;
  for (let attempt = 1; attempt <= MAX_RETRIES; attempt += 1) {
    try {
      return await run();
    } catch (error) {
      lastError = error;
      if (attempt === MAX_RETRIES) {
        break;
      }
      console.debug(
        `${logPrefix} ${label} failed on attempt ${attempt}/${MAX_RETRIES}; retrying after core warm-up`,
        error
      );
      await wait(RETRY_BASE_DELAY_MS * attempt);
    }
  }
  throw lastError instanceof Error ? lastError : new Error(`Failed to ${label}`);
};

export interface LocalAiPresetResolution {
  presets: PresetsResponse;
  recommendedTier: string;
  selectedTier: string | null;
  hadSelectedTier: boolean;
  appliedTier: string | null;
}

export const ensureRecommendedLocalAiPresetIfNeeded = async (
  logPrefix = '[local-ai-bootstrap]'
): Promise<LocalAiPresetResolution> => {
  const presets = await retryLocalAiCommand(
    'load local AI presets',
    () => openhumanLocalAiPresets(),
    logPrefix
  );
  const selectedTier = normalizeSelectedTier(presets.selected_tier);
  const recommendedTier = presets.recommended_tier;

  if (selectedTier) {
    console.debug(
      `${logPrefix} keeping existing local AI preset`,
      JSON.stringify({ selectedTier, currentTier: presets.current_tier, recommendedTier })
    );
    return { presets, recommendedTier, selectedTier, hadSelectedTier: true, appliedTier: null };
  }

  // No selected tier yet: persist the recommended tier so the Rust-side
  // `config_with_recommended_tier_if_unselected()` honors the user's
  // opt-in instead of defaulting a low-RAM device back to disabled.
  // The mount-time probe in LocalAIStep uses `openhumanLocalAiPresets()`
  // directly, so this apply only runs when the user has explicitly
  // chosen to proceed with local AI (consent flow).
  console.debug(
    `${logPrefix} applying recommended local AI preset`,
    JSON.stringify({ recommendedTier, recommendDisabled: presets.recommend_disabled ?? false })
  );
  await retryLocalAiCommand(
    'apply recommended local AI preset',
    () => openhumanLocalAiApplyPreset(recommendedTier),
    logPrefix
  );

  return {
    presets: { ...presets, current_tier: recommendedTier, selected_tier: recommendedTier },
    recommendedTier,
    selectedTier: null,
    hadSelectedTier: false,
    appliedTier: recommendedTier,
  };
};

export const triggerLocalAiAssetBootstrap = async (
  force = false,
  logPrefix = '[local-ai-bootstrap]'
) => {
  console.debug(`${logPrefix} triggering local AI background bootstrap`, JSON.stringify({ force }));
  return await retryLocalAiCommand(
    force ? 're-bootstrap local AI assets' : 'bootstrap local AI assets',
    () => openhumanLocalAiDownloadAllAssets(force),
    logPrefix
  );
};

export const bootstrapLocalAiWithRecommendedPreset = async (
  force = false,
  logPrefix = '[local-ai-bootstrap]'
) => {
  const preset = await ensureRecommendedLocalAiPresetIfNeeded(logPrefix);
  const download = await triggerLocalAiAssetBootstrap(force, logPrefix);
  return { preset, download };
};
