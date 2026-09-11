/**
 * Model + dimensions section for the Embeddings panel (active provider with
 * catalog models). Extracted out of `EmbeddingsPanel.tsx` to keep that file
 * under the repo's file-size convention.
 */
import { useT } from '../../../lib/i18n/I18nContext';
import type { EmbeddingModelPreset } from '../../../services/api/embeddingsApi';
import { Button } from '../../ui';
import { SettingsRow, SettingsSection, SettingsSelect } from '../controls';

export interface EmbeddingsModelSectionProps {
  currentModels: readonly EmbeddingModelPreset[];
  allowedDims: readonly number[];
  model: string;
  dimensions: number;
  onModelChange: (modelId: string) => void;
  onDimsChange: (dims: number) => void;
  canClearKey: boolean;
  onClearKey: () => void;
  onTestConnection: () => void;
  testConnectionDisabled: boolean;
}

const EmbeddingsModelSection = ({
  currentModels,
  allowedDims,
  model,
  dimensions,
  onModelChange,
  onDimsChange,
  canClearKey,
  onClearKey,
  onTestConnection,
  testConnectionDisabled,
}: EmbeddingsModelSectionProps) => {
  const { t } = useT();

  return (
    <SettingsSection>
      {currentModels.length > 1 && (
        <SettingsRow
          htmlFor="embeddings-model"
          label={t('settings.embeddings.model')}
          stacked
          control={
            <SettingsSelect
              id="embeddings-model"
              value={model}
              onChange={e => onModelChange(e.target.value)}
              className="w-full">
              {currentModels.map(m => (
                <option key={m.id} value={m.id}>
                  {m.label} ({m.id})
                </option>
              ))}
            </SettingsSelect>
          }
        />
      )}

      {allowedDims.length > 1 && (
        <SettingsRow
          htmlFor="embeddings-dims"
          label={t('settings.embeddings.dimensions')}
          stacked
          control={
            <SettingsSelect
              id="embeddings-dims"
              value={dimensions}
              onChange={e => onDimsChange(Number(e.target.value))}
              className="w-full">
              {allowedDims.map(d => (
                <option key={d} value={d}>
                  {d}
                </option>
              ))}
            </SettingsSelect>
          }
        />
      )}

      {/* Active provider info + actions */}
      <div className="flex items-center gap-2 px-4 py-3">
        {canClearKey && (
          <Button variant="secondary" tone="danger" size="xs" onClick={onClearKey}>
            {t('settings.embeddings.clearKey')}
          </Button>
        )}
        <Button
          variant="secondary"
          size="xs"
          onClick={onTestConnection}
          disabled={testConnectionDisabled}>
          {t('settings.embeddings.testConnection')}
        </Button>
      </div>
    </SettingsSection>
  );
};

export default EmbeddingsModelSection;
