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
    mode === 'edit' ? (
      <CustomServerFormModal
        mode="edit"
        server={target as InstalledServer}
        onClose={onClose}
        onSubmit={onSubmit}
      />
    ) : (
      <CustomServerFormModal mode="create" onClose={onClose} onSubmit={onSubmit} />
    )
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
    fireEvent.change(screen.getByLabelText('Value for API_KEY'), { target: { value: 'fresh' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalled());
    expect(onSubmit.mock.calls[0][0].env).toEqual({ API_KEY: 'fresh' });
  });

  /** Removing the row is the delete gesture — the key must stop being sent. */
  it('drops a removed env row from the payload', async () => {
    const { onSubmit } = renderForm('edit', server({ env_keys: ['API_KEY', 'OTHER'] }));
    fireEvent.click(screen.getByRole('button', { name: 'Remove API_KEY' }));
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalled());
    expect(onSubmit.mock.calls[0][0].env).toEqual({ OTHER: '' });
  });

  /**
   * Rows are otherwise indistinguishable by ear — every key input is "Key" and
   * every value "Value". Naming the value and remove controls after their row's
   * key is what makes the editor usable with a screen reader.
   */
  it('names each env row control after its key', () => {
    renderForm('edit', server({ env_keys: ['API_KEY', 'OTHER'] }));
    expect(screen.getByLabelText('Value for API_KEY')).toBeInTheDocument();
    expect(screen.getByLabelText('Value for OTHER')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Remove API_KEY' })).toBeInTheDocument();
    // A row with no key yet falls back to the bare label.
    fireEvent.click(screen.getByRole('button', { name: 'Add row' }));
    expect(screen.getByLabelText('Value')).toBeInTheDocument();
  });

  /**
   * The env label is the only thing distinguishing "local subprocess
   * environment" from "headers sent to a remote host" — it has to be exposed,
   * not just drawn.
   */
  it('exposes the env section label and hint to assistive tech', () => {
    renderForm();
    const group = screen.getByRole('group', { name: 'Environment variables' });
    expect(group).toHaveAccessibleDescription('Passed to the command when it starts.');

    fireEvent.click(screen.getByRole('button', { name: 'Remote URL' }));
    expect(screen.getByRole('group', { name: 'Request headers' })).toHaveAccessibleDescription(
      'Sent with every request. Use Authorization for a bearer token.'
    );
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

  /**
   * Toggling away and back changed nothing, so it must not delete anything. The
   * stored keys apply again once the transport matches what the server is saved
   * with — and since the submitted key set is authoritative, failing to re-seed
   * them would have the core drop every credential.
   */
  it('restores the stored env keys when the transport is toggled back', async () => {
    const { onSubmit } = renderForm('edit', server({ env_keys: ['GITHUB_TOKEN'] }));
    fireEvent.click(screen.getByRole('button', { name: 'Remote URL' }));
    fireEvent.click(screen.getByRole('button', { name: 'Local command' }));

    expect((screen.getAllByLabelText('Key')[0] as HTMLInputElement).value).toBe('GITHUB_TOKEN');
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalled());
    // Blank value = keep, so the credential survives a round trip untouched.
    expect(onSubmit.mock.calls[0][0].env).toEqual({ GITHUB_TOKEN: '' });
  });

  /**
   * A key deleted before toggling must stay deleted when the transport comes
   * back — the rows are stashed per transport, not re-seeded from the stored
   * set. Re-seeding would silently resurrect a credential the user just revoked.
   */
  it('does not resurrect a deleted key across a transport toggle', async () => {
    const { onSubmit } = renderForm('edit', server({ env_keys: ['API_KEY', 'OLD_KEY'] }));
    // Delete OLD_KEY (the second row).
    fireEvent.click(screen.getByRole('button', { name: 'Remove OLD_KEY' }));
    // Toggle away and back.
    fireEvent.click(screen.getByRole('button', { name: 'Remote URL' }));
    fireEvent.click(screen.getByRole('button', { name: 'Local command' }));

    const keys = (screen.getAllByLabelText('Key') as HTMLInputElement[]).map(i => i.value);
    expect(keys).toEqual(['API_KEY']);
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalled());
    expect(onSubmit.mock.calls[0][0].env).toEqual({ API_KEY: '' });
  });

  /** Rows built on the remote transport survive a toggle away and back too. */
  it('preserves in-progress rows for each transport independently', async () => {
    renderForm();
    // Add a header on remote.
    fireEvent.click(screen.getByRole('button', { name: 'Remote URL' }));
    fireEvent.change(screen.getAllByLabelText('Key')[0], { target: { value: 'X-Header' } });
    // Toggle to local and back to remote.
    fireEvent.click(screen.getByRole('button', { name: 'Local command' }));
    fireEvent.click(screen.getByRole('button', { name: 'Remote URL' }));
    expect((screen.getAllByLabelText('Key')[0] as HTMLInputElement).value).toBe('X-Header');
  });

  /** A map would silently keep the last row and drop the other's value. */
  it('rejects duplicate env keys instead of silently clobbering', async () => {
    const { onSubmit } = renderForm();
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'S' } });
    fireEvent.change(screen.getByLabelText('Command'), { target: { value: 'npx' } });
    fireEvent.change(screen.getAllByLabelText('Key')[0], { target: { value: 'API_KEY' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add row' }));
    fireEvent.change(screen.getAllByLabelText('Key')[1], { target: { value: 'API_KEY' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add server' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('API_KEY');
    expect(onSubmit).not.toHaveBeenCalled();
  });

  /**
   * The rows are HTTP header names on this transport, and RFC 9110 makes those
   * case-insensitive. Both would be stored and both sent, and the core hands
   * them to `build_http_auth` as a `HashMap` — whose iteration order Rust
   * randomises per process — so which one reached the wire would vary run to run.
   */
  it('rejects header names differing only by case on the remote transport', async () => {
    const { onSubmit } = renderForm();
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'S' } });
    fireEvent.click(screen.getByRole('button', { name: 'Remote URL' }));
    fireEvent.change(screen.getByLabelText('Server URL'), {
      target: { value: 'https://x.io/mcp' },
    });
    fireEvent.change(screen.getAllByLabelText('Key')[0], { target: { value: 'Authorization' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add row' }));
    fireEvent.change(screen.getAllByLabelText('Key')[1], { target: { value: 'authorization' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add server' }));

    expect(await screen.findByRole('alert')).toHaveTextContent('authorization');
    expect(onSubmit).not.toHaveBeenCalled();
  });

  /** Env var names are case-sensitive on Unix, so stdio must not over-reject. */
  it('allows env vars differing only by case on the local transport', async () => {
    const { onSubmit } = renderForm();
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'S' } });
    fireEvent.change(screen.getByLabelText('Command'), { target: { value: 'npx' } });
    fireEvent.change(screen.getAllByLabelText('Key')[0], { target: { value: 'Path' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add row' }));
    fireEvent.change(screen.getAllByLabelText('Key')[1], { target: { value: 'PATH' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add server' }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalled());
    expect(Object.keys(onSubmit.mock.calls[0][0].env).sort()).toEqual(['PATH', 'Path']);
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

  /**
   * Escape / backdrop / ✕ read as "cancel", but nothing aborts an in-flight
   * request — the mutation lands regardless, and once the host unmounts the
   * modal its error has nowhere to render. Gate the dismissals instead.
   */
  it('ignores dismissal while a save is in flight', async () => {
    let release: (() => void) | undefined;
    const onSubmit = vi.fn(() => new Promise<void>(resolve => (release = resolve)));
    const { onClose } = renderForm('create', undefined, onSubmit);

    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'S' } });
    fireEvent.change(screen.getByLabelText('Command'), { target: { value: 'npx' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add server' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Saving…' })).toBeDisabled());

    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).not.toHaveBeenCalled();

    release?.();
    // Once the request settles the form closes itself, as normal.
    await waitFor(() => expect(onClose).toHaveBeenCalled());
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
