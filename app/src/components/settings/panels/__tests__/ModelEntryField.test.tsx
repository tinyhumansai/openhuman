import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../test/test-utils';
import { ModelEntryField, useModelEntryMode } from '../ai/ModelEntryField';

/**
 * Direct coverage for the model / deployment-name field shared by both AI-panel
 * pickers (#5213). The panel-level regressions in `AIPanel.test.tsx` prove the
 * Azure wiring end to end; these exercise the branches that are awkward to
 * reach from there — a still-loading catalog and a rejected `/models` probe.
 */

/** Thin harness so the hook can drive the component the way the panel does. */
const Harness = ({
  endpoint,
  model = '',
  catalog = [],
  catalogLoading = false,
  catalogError = null,
  onRetry,
}: {
  endpoint?: string;
  model?: string;
  catalog?: { id: string }[];
  catalogLoading?: boolean;
  catalogError?: string | null;
  onRetry?: () => void;
}) => {
  const mode = useModelEntryMode({ endpoint, model, catalogIds: catalog.map(m => m.id) });
  return (
    <ModelEntryField
      mode={mode}
      model={model}
      onModelChange={() => {}}
      catalog={catalog}
      catalogLoading={catalogLoading}
      catalogError={catalogError}
      onRetry={onRetry}
      label="Model"
      placeholder="model-id"
      analyticsId="test-toggle"
    />
  );
};

describe('ModelEntryField', () => {
  it('relabels the field for an Azure endpoint and opens on free text', () => {
    renderWithProviders(
      <Harness
        endpoint="https://my-resource.openai.azure.com/openai/v1"
        catalog={[{ id: 'gpt-4o' }]}
      />
    );

    expect(screen.getByRole('textbox', { name: /Deployment name/i })).toBeInTheDocument();
    expect(screen.getByText(/This is not the model ID/i)).toBeInTheDocument();
  });

  it('keeps a non-Azure provider on the catalog dropdown', () => {
    renderWithProviders(
      <Harness endpoint="https://api.openai.com/v1" catalog={[{ id: 'gpt-4o' }]} />
    );

    expect(screen.getByRole('combobox', { name: /Model/i })).toBeInTheDocument();
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
    expect(screen.queryByText(/This is not the model ID/i)).not.toBeInTheDocument();
    // The escape hatch is offered to every provider, not just Azure.
    expect(screen.getByRole('button', { name: /Enter model ID manually/i })).toBeInTheDocument();
  });

  it('does not stall an Azure field behind a still-loading catalog', () => {
    // The listing only fills the dropdown, and its values are the wrong ones for
    // Azure anyway, so waiting on it would be pure delay.
    renderWithProviders(
      <Harness endpoint="https://my-resource.openai.azure.com/openai/v1" catalogLoading />
    );

    expect(screen.getByRole('textbox', { name: /Deployment name/i })).toBeInTheDocument();
    expect(screen.queryByText(/Loading models/i)).not.toBeInTheDocument();
  });

  it('shows the loading placeholder while a catalog-mode provider probes', () => {
    // The realistic shape: the panel clears the catalog before fetching, so a
    // provider in dropdown mode is loading with an *empty* catalog. Gating the
    // placeholder on the effective mode would lose it in exactly this window.
    renderWithProviders(<Harness endpoint="https://api.openai.com/v1" catalogLoading />);

    expect(screen.getByText(/Loading models/i)).toBeInTheDocument();
    expect(screen.queryByRole('textbox')).not.toBeInTheDocument();
  });

  it('omits the model-id retry copy for an Azure provider', () => {
    // The retry hint names a model id, which is the wrong thing to ask an
    // Azure user for; they get the deployment-name help under the field.
    renderWithProviders(
      <Harness
        endpoint="https://my-resource.openai.azure.com/openai/v1"
        catalogError="provider returned 404"
        onRetry={() => {}}
      />
    );

    expect(screen.queryByText(/enter model id manually:/i)).not.toBeInTheDocument();
    expect(screen.getByText(/This is not the model ID/i)).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: /Deployment name/i })).toBeInTheDocument();
  });

  it('surfaces a probe error with a retry and still accepts a typed value', () => {
    const onRetry = vi.fn();
    renderWithProviders(
      <Harness
        endpoint="https://api.openai.com/v1"
        catalogError="provider returned 404: no /models endpoint"
        onRetry={onRetry}
      />
    );

    expect(screen.getByText(/provider returned 404/i)).toBeInTheDocument();
    // A failed listing must never lock the user out of naming a model.
    expect(screen.getByRole('textbox', { name: /Model/i })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /Try again/i }));
    expect(onRetry).toHaveBeenCalledTimes(1);
  });

  it('flags a stored Azure value that is verbatim a catalog model id', () => {
    renderWithProviders(
      <Harness
        endpoint="https://my-resource.openai.azure.com/openai/v1"
        model="gpt-5.6-terra-2026-07-09"
        catalog={[{ id: 'gpt-5.6-terra-2026-07-09' }]}
      />
    );

    expect(
      screen.getByText(/confirm this is the name you gave your deployment/i)
    ).toBeInTheDocument();
  });

  it('leaves an off-catalog Azure value unflagged', () => {
    renderWithProviders(
      <Harness
        endpoint="https://my-resource.openai.azure.com/openai/v1"
        model="gpt-5.6-terra"
        catalog={[{ id: 'gpt-5.6-terra-2026-07-09' }]}
      />
    );

    expect(
      screen.queryByText(/confirm this is the name you gave your deployment/i)
    ).not.toBeInTheDocument();
  });
});
