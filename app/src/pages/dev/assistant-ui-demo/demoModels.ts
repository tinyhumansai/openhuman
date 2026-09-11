/**
 * Model list for the vendored assistant-ui `base` demo.
 *
 * Upstream this comes from the docs site's live model catalogue
 * (`docsModelOptions()` / `DEFAULT_MODEL_ID`). Here it is a fixed table: the
 * demo is mock-only, so nothing behind these ids is ever dialled and the list
 * exists purely to give `ModelSelector` something to render.
 */
import type { ModelOption } from '@/components/assistant-ui/model-selector';

export const DEFAULT_MODEL_ID = 'demo-sonnet';

export function demoModelOptions(): readonly ModelOption[] {
  return [
    { id: 'demo-sonnet', name: 'Demo Sonnet', description: 'Balanced mock model', efforts: true },
    {
      id: 'demo-opus',
      name: 'Demo Opus',
      description: 'Highest quality mock model',
      efforts: true,
    },
    { id: 'demo-haiku', name: 'Demo Haiku', description: 'Fastest mock model' },
  ];
}
