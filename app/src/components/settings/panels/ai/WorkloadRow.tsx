/*
 * One workload's row in the "Advanced" routing table.
 *
 * This is a `<tr>`, not a stack of `<div>`s. The advanced view is a matrix —
 * eight workloads against the provider and model each resolves to — and the
 * old markup rendered every cell as another line inside a flex column, so
 * nothing lined up between rows and the values could only be read by scanning
 * each block in turn. A real table gives the columns, the header association,
 * and a row a screen reader can announce as a row.
 *
 * The row keeps `data-slot="workload-row"` so tests can reach it without
 * matching a class string that layout work is free to change. `TableRow`
 * writes its own `data-slot` before spreading props, so ours wins.
 */
import { useT } from '../../../../lib/i18n/I18nContext';
import Button from '../../../ui/Button';
import {
  type CloudProvider,
  type ProviderRef,
  slugTone,
  type Workload,
  WORKLOAD_MODEL_HINT_KEYS,
} from './aiPanelTypes';
import { ProviderSwatch } from './ProviderListRow';

export type WorkloadRowProps = {
  workload: Workload;
  ref_: ProviderRef;
  cloudProviders: CloudProvider[];
};

/**
 * Split a `ProviderRef` into the two columns the table shows.
 *
 * The old row joined these into one `"OpenAI · gpt-5.6"` string. A table needs
 * them apart, and keeping the split here means the joining rule lives in one
 * place rather than being re-derived per cell.
 */
function resolveTarget(
  ref_: ProviderRef,
  cloudProviders: CloudProvider[],
  t: (key: string) => string
): { provider: string | null; providerSlug: string | null; model: string | null } {
  if (ref_.kind === 'cloud') {
    const cloud = cloudProviders.find(c => c.slug === ref_.providerSlug);
    return {
      provider: cloud?.label ?? ref_.providerSlug,
      providerSlug: ref_.providerSlug,
      model: ref_.model,
    };
  }
  if (ref_.kind === 'local') {
    return { provider: t('settings.ai.localOllama'), providerSlug: 'ollama', model: ref_.model };
  }
  if (ref_.kind === 'claude-code') {
    return {
      provider: t('settings.ai.claudeCode.modalTitle'),
      providerSlug: 'claude-code',
      model: ref_.model,
    };
  }
  // `openhuman` and `default` are the SAME state to the user: this workload
  // has no explicit pin and inherits whatever Managed decides. `inferRoutingMode`
  // already treats them as one (`refs.every(kind === 'openhuman' || 'default')`
  // is what makes the mode "managed"), so rendering them differently is a bug.
  //
  // It is a PRE-EXISTING one, not a consequence of the table: the row before
  // this rewrite had branches for `cloud`/`local`/`openhuman` only, so a
  // `default` ref fell through to an empty string and rendered "No model
  // selected" for a workload that does route somewhere. A stacked list of
  // sub-labels hid it; a Provider column full of "No model selected" does not.
  if (ref_.kind === 'openhuman' || ref_.kind === 'default') {
    return { provider: t('settings.ai.openhumanDefault'), providerSlug: 'openhuman', model: null };
  }
  return { provider: null, providerSlug: null, model: null };
}

export const WorkloadRow = ({
  workload,
  ref_,
  cloudProviders,
  onCustomClick,
}: WorkloadRowProps & { onCustomClick: () => void }) => {
  const { t } = useT();
  // "Pinned" means the user chose this route explicitly, rather than inheriting
  // whatever Managed decides. Keep it as row state for styling and tests.
  const isPinned = ref_.kind === 'cloud' || ref_.kind === 'local' || ref_.kind === 'claude-code';
  const { provider, providerSlug, model } = resolveTarget(ref_, cloudProviders, t);

  return (
    <li
      data-slot="workload-row"
      data-pinned={isPinned}
      className="flex flex-wrap items-center gap-3 px-4 py-3">
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 flex-col gap-0.5">
          <span className="text-sm font-medium text-content">{t(workload.labelKey)}</span>
          <span className="text-xs leading-5 text-content-muted">{t(workload.descriptionKey)}</span>
          <span className="text-[11px] leading-5 text-content-faint">
            {t(WORKLOAD_MODEL_HINT_KEYS[workload.id])}
          </span>
        </div>
      </div>

      <Button
        type="button"
        variant="secondary"
        size="xs"
        onClick={onCustomClick}
        className="h-auto min-w-52 max-w-60 justify-start gap-2 px-3 py-2 text-left">
        {provider && providerSlug ? (
          <ProviderSwatch slug={providerSlug} label={provider} tone={slugTone(providerSlug)} />
        ) : null}
        <span className="flex min-w-0 flex-col gap-0.5">
          <span className="truncate text-[10px] font-medium text-content-muted">
            {provider ?? 'Select provider and model'}
          </span>
          <span className="max-w-full truncate font-mono text-xs text-content">
            {model || 'Choose a model'}
          </span>
        </span>
      </Button>
    </li>
  );
};

export default WorkloadRow;
