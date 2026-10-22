import { fireEvent, screen, waitFor } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../../../test/test-utils';
import { OciSettingsDialog } from '../OciSettingsDialog';

const renderDialog = (props: Partial<ComponentProps<typeof OciSettingsDialog>> = {}) => {
  const onSubmit = vi.fn().mockResolvedValue(undefined);
  const onCancel = vi.fn();
  renderWithProviders(
    <OciSettingsDialog
      label="OCI Generative AI"
      hasStoredKey={false}
      onCancel={onCancel}
      onSubmit={onSubmit}
      {...props}
    />
  );
  return { onSubmit, onCancel };
};

const type = (testId: string, value: string) =>
  fireEvent.change(screen.getByTestId(testId), { target: { value } });

describe('OciSettingsDialog', () => {
  it('starts in api_key mode with the key field and no profile fields', async () => {
    renderDialog();
    expect(await screen.findByTestId('oci-api-key')).toBeInTheDocument();
    expect(screen.queryByTestId('oci-profile')).not.toBeInTheDocument();
    expect(screen.getByTestId('oci-auth-mode')).toHaveValue('api_key');
  });

  it('refuses to save without a region and reports it inline', async () => {
    const { onSubmit } = renderDialog();
    fireEvent.click(await screen.findByTestId('oci-save'));
    expect(await screen.findByText(/region is required/i)).toBeInTheDocument();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it('submits normalised api_key settings together with the key', async () => {
    const { onSubmit } = renderDialog();
    type('oci-api-key', ' genai-key ');
    type('oci-region', ' us-chicago-1 ');
    // `/openai/v1` serves requests in a Generative AI project.
    fireEvent.click(screen.getByTestId('oci-save'));
    expect(await screen.findByText(/project ocid is required/i)).toBeInTheDocument();
    expect(onSubmit).not.toHaveBeenCalled();

    type('oci-project', ' ocid1.generativeaiproject.oc1..example ');
    fireEvent.click(screen.getByTestId('oci-save'));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(onSubmit).toHaveBeenCalledWith(
      {
        auth_mode: 'api_key',
        transport: 'openai',
        region: 'us-chicago-1',
        project_id: 'ocid1.generativeaiproject.oc1..example',
      },
      'genai-key'
    );
  });

  it('keeps a stored key when the field is left blank on an edit', async () => {
    const { onSubmit } = renderDialog({
      hasStoredKey: true,
      initial: {
        auth_mode: 'api_key',
        transport: 'openai',
        region: 'us-chicago-1',
        project_id: 'ocid1.generativeaiproject.oc1..example',
      },
    });
    fireEvent.click(await screen.findByTestId('oci-save'));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(onSubmit.mock.calls[0][1]).toBe('');
  });

  it('swaps the key field for profile fields in session_token mode and sends no key', async () => {
    const { onSubmit } = renderDialog();
    fireEvent.change(await screen.findByTestId('oci-auth-mode'), {
      target: { value: 'session_token' },
    });
    expect(screen.queryByTestId('oci-api-key')).not.toBeInTheDocument();
    type('oci-profile', 'SESSION');
    // A signed call names its compartment even on the openai transport.
    fireEvent.click(screen.getByTestId('oci-save'));
    expect(await screen.findByText(/compartment ocid is required/i)).toBeInTheDocument();
    expect(onSubmit).not.toHaveBeenCalled();

    type('oci-compartment', 'ocid1.compartment.oc1..example');
    type('oci-project', 'ocid1.generativeaiproject.oc1..example');
    fireEvent.click(screen.getByTestId('oci-save'));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(onSubmit).toHaveBeenCalledWith(
      {
        auth_mode: 'session_token',
        transport: 'openai',
        profile: 'SESSION',
        compartment_id: 'ocid1.compartment.oc1..example',
        project_id: 'ocid1.generativeaiproject.oc1..example',
      },
      ''
    );
  });

  it('explains that principal modes store no secret', async () => {
    renderDialog();
    fireEvent.change(await screen.findByTestId('oci-auth-mode'), {
      target: { value: 'instance_principal' },
    });
    const hint = screen.getByTestId('oci-principal-hint');
    expect(hint).toHaveTextContent(/no secret is stored/i);
    expect(hint).toHaveTextContent(/oci principals support/i);
    expect(screen.queryByTestId('oci-api-key')).not.toBeInTheDocument();
  });

  it('requires a compartment for the native transport and hides the project field', async () => {
    const { onSubmit } = renderDialog();
    type('oci-api-key', 'genai-key');
    type('oci-region', 'us-chicago-1');
    expect(screen.getByTestId('oci-project')).toBeInTheDocument();
    fireEvent.change(screen.getByTestId('oci-transport'), { target: { value: 'native_cohere' } });
    expect(screen.queryByTestId('oci-project')).not.toBeInTheDocument();

    fireEvent.click(screen.getByTestId('oci-save'));
    expect(await screen.findByText(/compartment ocid is required/i)).toBeInTheDocument();
    expect(onSubmit).not.toHaveBeenCalled();

    type('oci-compartment', 'ocid1.compartment.oc1..example');
    fireEvent.click(screen.getByTestId('oci-save'));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(onSubmit.mock.calls[0][0]).toMatchObject({
      transport: 'native_cohere',
      compartment_id: 'ocid1.compartment.oc1..example',
    });
  });

  it('shows a failed save inline and re-enables the form', async () => {
    const onSubmit = vi.fn().mockRejectedValue(new Error('config file not found'));
    renderDialog({ onSubmit });
    type('oci-api-key', 'genai-key');
    type('oci-region', 'us-chicago-1');
    type('oci-project', 'ocid1.generativeaiproject.oc1..example');
    fireEvent.click(screen.getByTestId('oci-save'));
    expect(await screen.findByText(/config file not found/i)).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId('oci-save')).toBeEnabled());
  });

  it('calls onCancel from the Cancel button', async () => {
    const { onCancel } = renderDialog();
    fireEvent.click(await screen.findByRole('button', { name: /^cancel$/i }));
    expect(onCancel).toHaveBeenCalled();
  });
});
