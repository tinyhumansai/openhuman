/**
 * Tests for CustomServerFormModal — the add / edit form for hand-entered MCP
 * servers.
 *
 * The env-editor cases carry the most weight: values are write-only, so the
 * blank-means-keep contract between this form and the core is what stops an
 * unrelated rename from wiping a stored credential.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import CustomServerFormModal from './CustomServerFormModal';
import type { InstalledServer } from './types';

const server = (over: Partial<InstalledServer> = {}): InstalledServer => ({
  server_id: 'srv-1',
  qualified_name: 'custom/my-server',
  display_name: 'My Server',
  command_kind: 'node',
  command: 'npx',
  args: ['-y', 'pkg'],
  env_keys: [],
  installed_at: 0,
  enabled: true,
  provenance: 'custom',
  transport: { kind: 'stdio' },
  ...over,
});

const renderForm = (
  mode: 'create' | 'edit' = 'create',
  target?: InstalledServer,
  onSubmit = vi.fn().mockResolvedValue(undefined)
) => {
  const onClose = vi.fn();
  render(
    <CustomServerFormModal mode={mode} server={target} onClose={onClose} onSubmit={onSubmit} />
  );
  return { onSubmit, onClose };
};

describe('CustomServerFormModal', () => {
  beforeEach(() => vi.clearAllMocks());

  it('defaults to the local-command transport', () => {
    renderForm();
    expect(screen.getByLabelText('Command')).toBeInTheDocument();
    expect(screen.queryByLabelText('Server URL')).not.toBeInTheDocument();
  });

  it('swaps to the URL field when the remote transport is picked', () => {
    renderForm();
    fireEvent.click(screen.getByRole('button', { name: 'Remote URL' }));
    expect(screen.getByLabelText('Server URL')).toBeInTheDocument();
    expect(screen.queryByLabelText('Command')).not.toBeInTheDocument();
  });

  it('blocks submit until the transport-required field is filled', () => {
    renderForm();
    const submit = screen.getByRole('button', { name: 'Add server' });
    expect(submit).toBeDisabled();
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'S' } });
    expect(submit).toBeDisabled(); // command still empty
    fireEvent.change(screen.getByLabelText('Command'), { target: { value: 'npx' } });
    expect(submit).toBeEnabled();
  });

  it('splits arguments one per line and drops blank lines', async () => {
    const { onSubmit } = renderForm();
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'S' } });
    fireEvent.change(screen.getByLabelText('Command'), { target: { value: 'npx' } });
    fireEvent.change(screen.getByLabelText('Arguments'), {
      target: { value: '-y\n\n@scope/pkg\n  ' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Add server' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalled());
    expect(onSubmit.mock.calls[0][0].args).toEqual(['-y', '@scope/pkg']);
  });

  it('sends the URL and no command for a remote server', async () => {
    const { onSubmit } = renderForm();
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'S' } });
    fireEvent.click(screen.getByRole('button', { name: 'Remote URL' }));
    fireEvent.change(screen.getByLabelText('Server URL'), {
      target: { value: 'https://x.io/mcp' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Add server' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalled());
    expect(onSubmit.mock.calls[0][0]).toMatchObject({
      transport: 'http_remote',
      url: 'https://x.io/mcp',
    });
    expect(onSubmit.mock.calls[0][0].command).toBeUndefined();
  });

  it('seeds the form from the server being edited', () => {
    renderForm('edit', server({ transport: { kind: 'http_remote', url: 'https://x.io/mcp' } }));
    expect(screen.getByLabelText('Name')).toHaveValue('My Server');
    expect(screen.getByLabelText('Server URL')).toHaveValue('https://x.io/mcp');
  });

  /**
   * The regression this guards: values never come back from the core, so an
   * untouched row is blank. Dropping blank rows would delete the credential.
   * The key must still be sent — blank means "keep".
   */
  it('sends a known env key with a blank value so the core keeps the secret', async () => {
    const { onSubmit } = renderForm('edit', server({ env_keys: ['API_KEY'] }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalled());
    expect(onSubmit.mock.calls[0][0].env).toEqual({ API_KEY: '' });
  });

  it('sends a retyped env value', async () => {
    const { onSubmit } = renderForm('edit', server({ env_keys: ['API_KEY'] }));
    fireEvent.change(screen.getAllByLabelText('Value')[0], { target: { value: 'fresh' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalled());
    expect(onSubmit.mock.calls[0][0].env).toEqual({ API_KEY: 'fresh' });
  });

  /** Removing the row is the delete gesture — the key must stop being sent. */
  it('drops a removed env row from the payload', async () => {
    const { onSubmit } = renderForm('edit', server({ env_keys: ['API_KEY', 'OTHER'] }));
    fireEvent.click(screen.getAllByRole('button', { name: 'Remove this row' })[0]);
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalled());
    expect(onSubmit.mock.calls[0][0].env).toEqual({ OTHER: '' });
  });

  /** `__oauth__` is the core's, not the user's — showing it would invite an
   *  edit that breaks the OAuth session. */
  it('hides reserved internal env keys from the editor', () => {
    renderForm('edit', server({ env_keys: ['__oauth__', 'API_KEY'] }));
    const keyInputs = screen.getAllByLabelText('Key') as HTMLInputElement[];
    expect(keyInputs.map(i => i.value)).toEqual(['API_KEY']);
  });

  /**
   * The rows mean subprocess env on stdio and request headers on http_remote.
   * Carrying them across a switch would re-scope a stored local secret into a
   * header sent to the endpoint — and blank-means-keep would make that
   * invisible, since the user only ever sees an empty box.
   */
  it('clears env rows when the transport changes so secrets are not re-scoped', async () => {
    const { onSubmit } = renderForm('edit', server({ env_keys: ['GITHUB_TOKEN'] }));
    expect((screen.getAllByLabelText('Key')[0] as HTMLInputElement).value).toBe('GITHUB_TOKEN');

    fireEvent.click(screen.getByRole('button', { name: 'Remote URL' }));
    expect((screen.getAllByLabelText('Key')[0] as HTMLInputElement).value).toBe('');

    fireEvent.change(screen.getByLabelText('Server URL'), {
      target: { value: 'https://x.io/mcp' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalled());
    // The stdio token must not ride along as a header.
    expect(onSubmit.mock.calls[0][0].env).toEqual({});
  });

  /** Re-selecting the current transport is a no-op, not a row reset. */
  it('keeps env rows when the same transport is re-selected', () => {
    renderForm('edit', server({ env_keys: ['API_KEY'] }));
    fireEvent.click(screen.getByRole('button', { name: 'Local command' }));
    expect((screen.getAllByLabelText('Key')[0] as HTMLInputElement).value).toBe('API_KEY');
  });

  /** A map would silently keep the last row and drop the other's value. */
  it('rejects duplicate env keys instead of silently clobbering', async () => {
    const { onSubmit } = renderForm();
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'S' } });
    fireEvent.change(screen.getByLabelText('Command'), { target: { value: 'npx' } });
    fireEvent.change(screen.getAllByLabelText('Key')[0], { target: { value: 'API_KEY' } });
    fireEvent.change(screen.getAllByLabelText('Value')[0], { target: { value: 'one' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add row' }));
    fireEvent.change(screen.getAllByLabelText('Key')[1], { target: { value: 'API_KEY' } });
    fireEvent.change(screen.getAllByLabelText('Value')[1], { target: { value: 'two' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add server' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('API_KEY');
    expect(onSubmit).not.toHaveBeenCalled();
  });

  /** The transport picker is a button group, not an ARIA radiogroup — it has no
   *  roving tabindex or arrow-key handling to back that contract up. */
  it('exposes the transport picker as a pressed-button group', () => {
    renderForm();
    expect(screen.queryByRole('radiogroup')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Local command' })).toHaveAttribute(
      'aria-pressed',
      'true'
    );
    expect(screen.getByRole('button', { name: 'Remote URL' })).toHaveAttribute(
      'aria-pressed',
      'false'
    );
  });

  it('surfaces the core error message verbatim on submit failure', async () => {
    const onSubmit = vi
      .fn()
      .mockRejectedValue(new Error('url scheme must be http or https, got `file`'));
    const { onClose } = renderForm('create', undefined, onSubmit);
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'S' } });
    fireEvent.change(screen.getByLabelText('Command'), { target: { value: 'npx' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add server' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('url scheme must be http or https');
    // The form stays open so the user can fix the offending field.
    expect(onClose).not.toHaveBeenCalled();
  });
});
