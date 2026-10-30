/** Advisory, no-egress model feasibility over SAVED catalog values only.
 * This neither routes inference nor checks credentials or actual availability.
 * Hard constraints precede cost ranking; missing or zero prices are unknown.
 */
export interface FeasibilityModel {
  id: string;
  provider: string;
  context_window?: number | null;
  vision?: boolean | null;
  cost_per_1m_input?: number | null;
  cost_per_1m_cached_input?: number | null;
  cost_per_1m_output?: number | null;
}

export interface ModelRequirements {
  inputTokens: number;
  cachedInputTokens?: number;
  outputTokens: number;
  requiresVision: boolean;
  maxEstimatedCostUSD: number | null;
}

export type ModelFitStatus = 'metadata_fit' | 'unknown' | 'excluded';
export type ModelFitReason =
  | 'context_too_small'
  | 'vision_unavailable'
  | 'estimated_cost_exceeds_budget'
  | 'unknown_context'
  | 'unknown_vision'
  | 'unknown_price';

export interface ModelFitRow<T extends FeasibilityModel> {
  model: T;
  status: ModelFitStatus;
  reasons: ModelFitReason[];
  estimatedCostUSD: number | null;
}

export interface ModelFeasibilityReport<T extends FeasibilityModel> {
  rows: ModelFitRow<T>[];
  counts: Record<ModelFitStatus, number>;
  availabilityVerified: false;
  livePricesVerified: false;
  selectionIsAdvisory: true;
}

const knownNonnegative = (value: unknown): value is number =>
  typeof value === 'number' && Number.isFinite(value) && value >= 0;

/** Explainable metadata-only ranking; never claims provider eligibility.
 * OpenHuman's ModelRegistryEntry contract uses zero/missing prices as unknown,
 * including entries labeled "free" until authoritative pricing is checked.
 */
export function assessModelFeasibility<T extends FeasibilityModel>(
  models: readonly T[],
  requirements: ModelRequirements
): ModelFeasibilityReport<T> {
  const { inputTokens, outputTokens, requiresVision, maxEstimatedCostUSD } = requirements;
  const cachedInputTokens =
    requirements.cachedInputTokens === undefined ? 0 : requirements.cachedInputTokens;
  if (
    !Number.isSafeInteger(cachedInputTokens) ||
    cachedInputTokens < 0 ||
    cachedInputTokens > inputTokens ||
    !Number.isSafeInteger(inputTokens) ||
    inputTokens < 0 ||
    !Number.isSafeInteger(outputTokens) ||
    outputTokens < 0 ||
    typeof requiresVision !== 'boolean' ||
    (maxEstimatedCostUSD !== null && !knownNonnegative(maxEstimatedCostUSD))
  ) {
    throw new Error('Invalid model feasibility requirements.');
  }
  const demand = inputTokens + outputTokens;
  if (!Number.isSafeInteger(demand)) {
    throw new Error('Invalid combined token demand.');
  }

  const rows: ModelFitRow<T>[] = models.map(model => {
    const reasons: ModelFitReason[] = [];
    const context = model.context_window;
    if (!knownNonnegative(context) || !Number.isSafeInteger(context) || context === 0) {
      reasons.push('unknown_context');
    } else if (context < demand) {
      reasons.push('context_too_small');
    }
    if (requiresVision) {
      if (model.vision === false) {
        reasons.push('vision_unavailable');
      } else if (model.vision !== true) {
        reasons.push('unknown_vision');
      }
    }

    const inputPrice = model.cost_per_1m_input;
    const cachedInputPrice = model.cost_per_1m_cached_input;
    const outputPrice = model.cost_per_1m_output;
    const uncachedInputTokens = inputTokens - cachedInputTokens;
    // Each consumed component needs a positive saved price; zero-price
    // metadata with actual usage remains unknown, not verified free.
    const uncachedKnown =
      uncachedInputTokens === 0 || (knownNonnegative(inputPrice) && inputPrice > 0);
    const cachedKnown =
      cachedInputTokens === 0 || (knownNonnegative(cachedInputPrice) && cachedInputPrice > 0);
    const outputKnown = outputTokens === 0 || (knownNonnegative(outputPrice) && outputPrice > 0);
    const estimated =
      uncachedKnown && cachedKnown && outputKnown
        ? ((uncachedInputTokens === 0 ? 0 : uncachedInputTokens * (inputPrice as number)) +
            (cachedInputTokens === 0 ? 0 : cachedInputTokens * (cachedInputPrice as number)) +
            (outputTokens === 0 ? 0 : outputTokens * (outputPrice as number))) /
          1_000_000
        : null;
    const estimatedCostUSD = estimated !== null && Number.isFinite(estimated) ? estimated : null;
    if (estimatedCostUSD === null) {
      reasons.push('unknown_price');
    } else if (maxEstimatedCostUSD !== null && estimatedCostUSD > maxEstimatedCostUSD) {
      reasons.push('estimated_cost_exceeds_budget');
    }

    const excluded = reasons.some(
      reason =>
        reason === 'context_too_small' ||
        reason === 'vision_unavailable' ||
        reason === 'estimated_cost_exceeds_budget'
    );
    const status: ModelFitStatus = excluded
      ? 'excluded'
      : reasons.length > 0
        ? 'unknown'
        : 'metadata_fit';
    return { model, status, reasons, estimatedCostUSD };
  });

  const rank: Record<ModelFitStatus, number> = { metadata_fit: 0, unknown: 1, excluded: 2 };
  rows.sort((a, b) => {
    const statusDiff = rank[a.status] - rank[b.status];
    if (statusDiff !== 0) return statusDiff;
    if (a.estimatedCostUSD !== null && b.estimatedCostUSD !== null) {
      const difference = a.estimatedCostUSD - b.estimatedCostUSD;
      if (difference !== 0) return difference;
    } else if (a.estimatedCostUSD !== null || b.estimatedCostUSD !== null) {
      return a.estimatedCostUSD === null ? 1 : -1;
    }
    return a.model.provider.localeCompare(b.model.provider) || a.model.id.localeCompare(b.model.id);
  });

  const counts: Record<ModelFitStatus, number> = { metadata_fit: 0, unknown: 0, excluded: 0 };
  for (const row of rows) counts[row.status]++;

  return {
    rows,
    counts,
    availabilityVerified: false,
    livePricesVerified: false,
    selectionIsAdvisory: true,
  };
}

/** Present microcosts conservatively: positive usage must not look free. */
export function formatEstimatedModelCost(cost: number | null): string {
  if (cost === null || !Number.isFinite(cost) || cost < 0) return 'Cost unknown';
  if (cost > 0 && cost < 0.0001) return '<$0.0001';
  return 'Est. $' + cost.toFixed(4);
}
